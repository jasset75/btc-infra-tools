#!/usr/bin/env python3
"""One independent GitLab Runner activation with optional-workload preflight.

Run periodically through the runner's own service manager. No watchdog or handoff
process/status is required. Queue API access is an optimization, never a gate.
"""
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
    def __init__(self, reason, **details):
        super().__init__(reason)
        self.details = details


def command(argv, timeout=20, *, operation=None):
    operation = operation or Path(argv[0]).name
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        if result.returncode:
            details = {"operation": operation, "returncode": result.returncode}
            if operation == "mining_health":
                try:
                    health = json.loads(result.stdout)
                    for key in ("failed_check", "error_type"):
                        if isinstance(health.get(key), str) and health[key].isidentifier():
                            details[key] = health[key]
                except (ValueError, AttributeError):
                    pass
            raise Deferred("command_failed", **details)
        return result.stdout
    except subprocess.TimeoutExpired:
        raise Deferred("command_timeout", operation=operation) from None
    except (OSError, subprocess.SubprocessError):
        raise Deferred("command_unavailable", operation=operation) from None


@contextlib.contextmanager
def lease(path):
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(fd)
        if info.st_uid != os.getuid() or not stat.S_ISREG(info.st_mode):
            raise Deferred("unsafe_lease")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Deferred("workload_active") from None
        yield fd
    finally:
        # A foreground runner retains its inherited lease after parent failure.
        os.close(fd)


def expand(value):
    if isinstance(value, str):
        return os.path.expandvars(os.path.expanduser(value))
    if isinstance(value, list):
        return [expand(x) for x in value]
    if isinstance(value, dict):
        return {k: expand(v) for k, v in value.items()}
    return value


class Dispatcher:
    def __init__(self, config):
        self.c = config
        self.root = Path(config["state_dir"])
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.child = None
        self.stopping = False

    def api(self, path):
        return json.loads(command([self.c["glab"], "api", "--hostname", self.c["gitlab_host"], path], 15, operation="gitlab_api"))

    def queue_hint(self):
        # On API/configuration uncertainty, let the native runner use its own
        # authentication and job-matching protocol. No glab token is required.
        try:
            runner = self.api("runners/%s" % self.c["runner_id"])
            if runner.get("id") != self.c["runner_id"]:
                return True
            if runner.get("paused") is True:
                return False
            if (runner.get("paused") is not False or
                    not isinstance(runner.get("tag_list"), list) or
                    not isinstance(runner.get("run_untagged"), bool) or
                    sorted(x["id"] for x in runner["projects"]) != sorted(self.c["project_ids"])):
                return True
            for project in self.c["project_ids"]:
                for page in range(1, 11):
                    jobs = self.api("projects/%s/jobs?scope[]=pending&per_page=100&page=%s" % (project, page))
                    if not isinstance(jobs, list):
                        return True
                    for job in jobs:
                        tags = job.get("tag_list")
                        if not isinstance(tags, list):
                            return True
                        if (set(tags).issubset(runner["tag_list"]) if tags else runner["run_untagged"]):
                            return True
                    if len(jobs) < 100:
                        break
                else:
                    return True
            return False
        except (Deferred, ValueError, KeyError, TypeError, AttributeError):
            return True

    def containers(self):
        try:
            data = json.loads(command([self.c["podman"], "ps", "--format", "json"], operation="container_inspection"))
            if not isinstance(data, list):
                raise ValueError()
            owned, other = [], []
            for item in data:
                target = owned if (item.get("Labels") or {}).get("io.podman.compose.project") == self.c["compose_project"] else other
                target.append(item)
            return owned, other
        except (ValueError, AttributeError, TypeError):
            raise Deferred("container_state_unknown") from None

    def runner_present(self):
        for line in command(["ps", "-axo", "comm=,args="], operation="runner_inspection").splitlines():
            fields = line.strip().split(None, 1)
            if len(fields) == 2 and Path(fields[0]).name == "gitlab-runner":
                if any(x in fields[1].split() for x in ("run", "run-single", "wrapper")):
                    return True
        return False

    def shutdown(self, *_):
        self.stopping = True
        if self.child is not None and self.child.poll() is None:
            self.child.send_signal(signal.SIGQUIT)

    def dispatch(self, dry_run=False):
        if self.stopping or not self.queue_hint():
            return "no_pending_work"
        with lease(self.root / "workload.lock") as fd:
            if self.runner_present():
                raise Deferred("runner_active")
            owned, other = self.containers()
            if other:
                raise Deferred("other_containers_active")
            if dry_run:
                return "would_stop_optional_and_request_runner_admission"
            if owned:
                command(self.c["stop_command"], 90, operation="optional_stop")
            if self.containers() != ([], []):
                raise Deferred("containers_still_running")
            command(self.c["health_command"], 20, operation="mining_health")
            if self.stopping:
                raise Deferred("shutdown")
            admission = self.c.get("runner_admission_command", self.c["admission_command"])
            argv = admission + [self.c["runner_binary"], "run-single",
                    "--config", self.c["runner_config"], "--runner", self.c["runner_name"],
                    "--max-builds", "1", "--wait-timeout", "60"]
            with open(self.root / "runner.log", "a") as log:
                self.child = subprocess.Popen(argv, stdout=log, stderr=log, pass_fds=(fd,))
                try:
                    code = self.child.wait()
                finally:
                    self.child = None
                    tmp = self.root / ("runner-finished.%s.tmp" % os.getpid())
                    tmp.write_text(json.dumps({"time": time.time()}))
                    os.replace(tmp, self.root / "runner-finished.json")
            if code == 75:
                raise Deferred("runner_admission_deferred", returncode=code)
            # The native runner has finished cleanup; health is checked again
            # without changing its completed GitLab job or protected services.
            command(self.c["health_command"], 20, operation="mining_health")
            if code:
                raise Deferred("runner_failed", returncode=code)
            return "runner_finished"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    config = expand(tomllib.loads(Path(args.config).read_text()))
    if config.get("version") != 1:
        raise ValueError("version=1 required")
    app = Dispatcher(config)
    signal.signal(signal.SIGTERM, app.shutdown)
    signal.signal(signal.SIGINT, app.shutdown)
    try:
        with lease(app.root / "runner-dispatch.lock"):
            state = app.dispatch(args.dry_run)
        print(json.dumps({"time": int(time.time()), "state": state}), flush=True)
        return 0
    except Deferred as error:
        print(json.dumps({"time": int(time.time()), "state": "deferred", "reason": str(error), **error.details}), flush=True)
        return 75


if __name__ == "__main__":
    sys.exit(main())
