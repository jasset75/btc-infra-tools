#!/usr/bin/env python3
"""Serialize a GitLab Docker runner and an optional Compose workload on macOS."""
import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import time
import tomllib


class Deferred(Exception):
    pass


def expand(value):
    if isinstance(value, str):
        return os.path.expandvars(os.path.expanduser(value))
    if isinstance(value, list):
        return [expand(x) for x in value]
    if isinstance(value, dict):
        return {k: expand(v) for k, v in value.items()}
    return value


@contextlib.contextmanager
def locked(path):
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(fd)
        if info.st_uid != os.getuid() or not stat.S_ISREG(info.st_mode):
            raise Deferred("unsafe_lock")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Deferred("workload_lease_held") from None
        yield fd
    finally:
        # Closing, rather than explicit LOCK_UN, preserves an inherited lease
        # if the coordinator dies while its foreground runner is still alive.
        os.close(fd)


def run(argv, *, timeout=20, pass_fds=(), check=True):
    try:
        result = subprocess.run(argv, capture_output=True, text=True,
                                timeout=timeout, pass_fds=pass_fds)
    except (OSError, subprocess.SubprocessError):
        raise Deferred("command_unavailable") from None
    if check and result.returncode:
        # Never forward arbitrary CLI output: it may contain credentials.
        raise Deferred("command_failed")
    return result


def runner_processes(output):
    found = []
    for line in output.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) != 3:
            continue
        pid, executable, args = fields
        if Path(executable).name == "gitlab-runner" and any(
                x in args.split() for x in ("run", "run-single", "wrapper")):
            found.append(int(pid))
    return found


def eligible(job, runner):
    tags = job.get("tag_list")
    if not isinstance(tags, list):
        raise Deferred("queue_tags_unknown")
    return (set(tags).issubset(runner["tag_list"]) if tags
            else runner["run_untagged"])


class Handoff:
    def __init__(self, config, now=time.monotonic):
        self.c = config
        self.now = now
        self.root = Path(config["state_dir"])
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.lease = self.root / "workload.lock"
        self.normal_since = None
        self.next_start = 0
        self.child = None
        self.stopping = False
        self.last_event = None

    def event(self, state, **extra):
        data = {"state": state, **extra}
        if data != self.last_event:
            print(json.dumps({"time": int(time.time()), **data}), flush=True)
            self.last_event = data
        tmp = self.root / ("status.%s.tmp" % os.getpid())
        tmp.write_text(json.dumps({"time": int(time.time()), **data}) + "\n")
        os.replace(tmp, self.root / "status.json")

    def pressure(self):
        result = run(["sysctl", "-n", "kern.memorystatus_vm_pressure_level"], check=False)
        return result.returncode == 0 and result.stdout.strip() == "1"

    def processes(self):
        return runner_processes(run(["ps", "-axo", "pid=,comm=,args="]).stdout)

    def containers(self):
        try:
            containers = json.loads(run([self.c["podman"], "ps", "--format", "json"]).stdout)
            if not isinstance(containers, list):
                raise ValueError()
            owned, other = [], []
            for item in containers:
                labels = item.get("Labels") or {}
                target = owned if labels.get("io.podman.compose.project") == self.c["compose_project"] else other
                target.append(item)
            return owned, other
        except (ValueError, TypeError, AttributeError):
            raise Deferred("container_state_unknown") from None

    def api(self, path):
        try:
            return json.loads(run([self.c["glab"], "api", "--hostname", self.c["gitlab_host"], path], timeout=15).stdout)
        except ValueError:
            raise Deferred("gitlab_state_unknown") from None

    def queue(self):
        runner = self.api("runners/%s" % self.c["runner_id"])
        if (runner.get("id") != self.c["runner_id"] or
                runner.get("runner_type") != "project_type" or
                runner.get("access_level") != "not_protected" or
                runner.get("locked") is not True or
                not isinstance(runner.get("tag_list"), list) or
                not isinstance(runner.get("run_untagged"), bool) or
                not isinstance(runner.get("paused"), bool)):
            raise Deferred("unsupported_runner_scope")
        projects = sorted(x["id"] for x in runner.get("projects", []))
        if projects != sorted(self.c["project_ids"]):
            raise Deferred("runner_projects_changed")
        # A user pause is never undone. Existing running jobs still block recovery.
        jobs = self.api("runners/%s/jobs?status=running&per_page=100" % self.c["runner_id"])
        if not isinstance(jobs, list) or jobs:
            raise Deferred("remote_runner_active")
        if runner["paused"]:
            return False
        for project in projects:
            for page in range(1, 11):
                jobs = self.api("projects/%s/jobs?scope[]=pending&per_page=100&page=%s" % (project, page))
                if not isinstance(jobs, list):
                    raise Deferred("queue_state_unknown")
                if any(eligible(job, runner) for job in jobs):
                    return True
                if len(jobs) < 100:
                    break
            else:
                raise Deferred("queue_page_limit")
        return False

    def stop_optional(self, fd):
        owned, _ = self.containers()
        if owned:
            self.event("stopping_optional")
            run(self.c["stop_command"], timeout=90, pass_fds=(fd,))
        if self.containers()[0]:
            raise Deferred("optional_still_running")
        if owned:
            run(self.c["health_command"], timeout=20)

    def recovery_reason(self):
        if (self.root / "optional.paused").exists():
            return "manual_pause"
        if not self.pressure():
            return "memory_pressure"
        if self.processes():
            return "runner_active"
        if self.containers()[1]:
            return "other_containers_active"
        if self.queue():
            return "ci_pending"
        if not self.cooldown_ready():
            return "startup_cooldown"
        # The supervisor publishes a readiness lease only after sustained normal
        # pressure. An absent, stale or future-dated lease never enables recovery.
        try:
            status = json.loads((self.root / "status.json").read_text())
            age = time.time() - status["time"]
            if status["state"] not in ("optional_allowed", "optional_running") or not 0 <= age <= 150:
                return "supervisor_not_ready"
        except (OSError, ValueError, KeyError, TypeError):
            return "supervisor_not_ready"
        return None

    def cooldown_ready(self):
        try:
            stamp = json.loads((self.root / "last-start.json").read_text())["time"]
            return time.time() - stamp >= self.c["restart_cooldown_seconds"]
        except FileNotFoundError:
            return True
        except (OSError, ValueError, KeyError, TypeError):
            return False

    def gate(self):
        with locked(self.lease):
            return self.recovery_reason()

    def recover(self):
        with locked(self.lease) as fd:
            reason = self.recovery_reason()
            if reason:
                raise Deferred(reason)
            self.start_optional(fd)

    def start_optional(self, fd):
        if self.stopping:
            raise Deferred("shutdown")
        if not self.cooldown_ready():
            raise Deferred("startup_cooldown")
        # The configured probe is read-only. It must check the protected workload
        # without trying to recover or reconfigure it.
        run(self.c["health_command"], timeout=20)
        stamp = self.root / ("last-start.%s.tmp" % os.getpid())
        stamp.write_text(json.dumps({"time": time.time()}))
        os.replace(stamp, self.root / "last-start.json")
        result = run(self.c["admission_command"] + self.c["start_command"],
                     timeout=90, pass_fds=(fd,), check=False)
        if result.returncode:
            raise Deferred("optional_start_deferred_or_failed")
        if not self.pressure():
            self.stop_optional(fd)
            raise Deferred("memory_pressure_after_start")

    def run_job(self, fd):
        self.stop_optional(fd)
        if self.containers()[1] or self.processes():
            raise Deferred("foreign_workload_active")
        if self.stopping or not self.pressure():
            raise Deferred("runner_admission_deferred")
        run(self.c["health_command"], timeout=20)
        self.event("runner_active")
        argv = self.c["admission_command"] + [self.c["runner_binary"], "run-single",
                "--config", self.c["runner_config"], "--runner", self.c["runner_name"],
                "--max-builds", "1", "--wait-timeout", "60"]
        # The child inherits the lease across the admission guard's exec. A
        # crashed supervisor cannot release it underneath an active CI job.
        with open(self.root / "runner.log", "a") as log:
            self.child = subprocess.Popen(argv, stdout=log, stderr=log, pass_fds=(fd,))
            try:
                code = self.child.wait()
            finally:
                self.child = None
        self.event("runner_finished", exit_code=code)
        self.normal_since = None
        self.next_start = self.now() + self.c["settle_seconds"]

    def tick(self, dry_run=False):
        with locked(self.lease) as fd:
            normal = self.pressure()
            processes = self.processes()
            owned, others = self.containers()
            if not normal or processes or others:
                self.normal_since = None
                reason = "runner_active" if processes else "other_containers_active" if others else "memory_pressure"
                if not dry_run:
                    self.stop_optional(fd)
                self.event(reason, dry_run=dry_run)
                return
            pending = self.queue()
            if pending:
                self.normal_since = None
                if dry_run:
                    self.event("would_stop_optional_then_run_one_job")
                else:
                    self.run_job(fd)
                return
            if (self.root / "optional.paused").exists():
                if not dry_run:
                    self.stop_optional(fd)
                self.event("manual_pause")
                return
            if self.normal_since is None:
                self.normal_since = self.now()
            if self.now() - self.normal_since < self.c["settle_seconds"] or self.now() < self.next_start:
                self.event("settling")
                return
            if not owned and not self.cooldown_ready():
                self.event("startup_cooldown")
                return
            if dry_run:
                self.event("would_allow_optional")
                return
            self.event("optional_running" if owned else "optional_allowed")
            if not owned:
                self.next_start = self.now() + self.c["restart_cooldown_seconds"]
                self.start_optional(fd)
                self.event("optional_running")

    def shutdown(self, *_):
        self.stopping = True
        if self.child is not None and self.child.poll() is None:
            # Only this exact, unreaped child; never a process group or a PID
            # loaded from a file. SIGQUIT lets GitLab finish its active job.
            self.child.send_signal(signal.SIGQUIT)


def load_config(path):
    c = expand(tomllib.loads(Path(path).read_text()))
    if c.get("version") != 1 or c.get("poll_seconds", 0) < 60 or c.get("settle_seconds", 0) < 120 or c.get("restart_cooldown_seconds", 0) < 600:
        raise ValueError("version=1, poll>=60, settle>=120 and restart cooldown>=600 required")
    for field in ("stop_command", "start_command", "admission_command", "health_command"):
        if not isinstance(c.get(field), list) or not c[field]:
            raise ValueError("a nonempty command array is required: " + field)
    return c


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    parser.add_argument("action", choices=["run", "gate", "recover", "status", "pause-optional", "resume-optional"])
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    app = Handoff(load_config(args.config))
    try:
        if args.action == "gate":
            reason = app.gate()
            if reason:
                raise Deferred(reason)
        elif args.action == "recover":
            if args.dry_run:
                reason = app.gate()
                if reason:
                    raise Deferred(reason)
            else:
                app.recover()
        elif args.action == "status":
            print((app.root / "status.json").read_text())
        elif args.action in ("pause-optional", "resume-optional"):
            if not args.dry_run:
                pause = app.root / "optional.paused"
                if args.action == "pause-optional":
                    pause.touch(mode=0o600)
                else:
                    pause.unlink(missing_ok=True)
        else:
            signal.signal(signal.SIGTERM, app.shutdown)
            signal.signal(signal.SIGINT, app.shutdown)
            with locked(app.root / "supervisor.lock"):
                while not app.stopping:
                    try:
                        app.tick(args.dry_run)
                    except Deferred as error:
                        app.normal_since = None
                        app.event("deferred", reason=str(error))
                    if args.once:
                        break
                    deadline = time.monotonic() + app.c["poll_seconds"]
                    while not app.stopping and time.monotonic() < deadline:
                        time.sleep(1)
        return 0
    except Deferred as error:
        print(json.dumps({"state": "paused", "reason": str(error)}))
        return 75


if __name__ == "__main__":
    sys.exit(main())
