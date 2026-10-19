import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from handoff import Deferred, Handoff, eligible, locked, runner_processes


class Fake(Handoff):
    def __init__(self, root):
        self.clock = 0
        super().__init__({"state_dir": root, "settle_seconds": 120,
                          "restart_cooldown_seconds": 600}, now=lambda: self.clock)
        self.normal = True
        self.pids = []
        self.owned = []
        self.others = []
        self.pending = False
        self.calls = []

    def pressure(self):
        return self.normal

    def processes(self):
        return self.pids

    def containers(self):
        return self.owned, self.others

    def queue(self):
        if isinstance(self.pending, Exception):
            raise self.pending
        return self.pending

    def stop_optional(self, fd):
        self.calls.append("stop")
        self.owned = []

    def start_optional(self, fd):
        self.calls.append("start")
        self.owned = [{"id": "test"}]

    def run_job(self, fd):
        self.stop_optional(fd)
        self.calls.append("job")


class SafetyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.app = Fake(self.temp.name)

    def test_memory_pressure_stops_optional_and_defers_even_pending_jobs(self):
        self.app.normal = False
        self.app.pending = True
        self.app.tick()
        self.assertEqual(self.app.calls, ["stop"])
        self.assertEqual(self.app.gate(), "memory_pressure")

    def test_running_or_unknown_containers_prevent_optional_recovery(self):
        for attr, value, reason in [("pids", [44], "runner_active"),
                                    ("others", [{"id": "other"}], "other_containers_active")]:
            setattr(self.app, attr, value)
            self.app.tick()
            self.assertEqual(self.app.gate(), reason)
            setattr(self.app, attr, [])
        self.assertNotIn("start", self.app.calls)

    def test_pending_work_stops_optional_before_job(self):
        self.app.pending = True
        self.app.owned = [{"id": "optional"}]
        self.app.tick()
        self.assertEqual(self.app.calls, ["stop", "job"])

    def test_external_runner_is_never_launched_by_coordinator(self):
        self.app.c["runner_managed_externally"] = True
        self.app.pending = True
        self.app.owned = [{"id": "optional"}]
        self.app.tick()
        self.assertEqual(self.app.calls, ["stop"])
        self.assertEqual(self.app.last_event["state"], "ci_pending")

    def test_short_external_job_also_requires_fresh_settling(self):
        self.app.normal_since = 0
        self.app.clock = 180
        self.app.event("optional_allowed")
        (self.app.root / "runner-finished.json").write_text(json.dumps({"time": time.time()}))
        self.assertEqual(self.app.gate(), "runner_settling")
        self.app.tick()
        self.assertNotIn("start", self.app.calls)
        self.assertIsNone(self.app.normal_since)

    def test_recovery_requires_two_minutes_of_normal_pressure(self):
        self.app.tick()
        self.app.clock = 119
        self.app.tick()
        self.assertEqual(self.app.calls, [])
        self.assertEqual(self.app.gate(), "supervisor_not_ready")
        self.app.clock = 120
        self.app.tick()
        self.assertEqual(self.app.calls, ["start"])
        self.assertIsNone(self.app.gate())

    def test_warning_resets_settling_period(self):
        self.app.tick()
        self.app.clock = 120
        self.app.normal = False
        self.app.tick()
        self.app.normal = True
        self.app.clock = 180
        self.app.tick()
        self.assertNotIn("start", self.app.calls)
        self.app.clock = 300
        self.app.tick()
        self.assertEqual(self.app.calls[-1], "start")

    def test_manual_pause_survives_new_supervisor(self):
        (self.app.root / "optional.paused").touch()
        self.app.tick()
        other = Fake(self.temp.name)
        self.assertEqual(other.gate(), "manual_pause")
        self.assertNotIn("start", self.app.calls)

    def test_api_error_blocks_recovery(self):
        self.app.pending = Deferred("gitlab_unavailable")
        with self.assertRaises(Deferred):
            self.app.tick()
        with self.assertRaises(Deferred):
            self.app.gate()
        self.assertEqual(self.app.calls, [])

    def test_atomic_recovery_refuses_busy_lease_and_rechecks_queue(self):
        self.app.event("optional_allowed")
        with locked(self.app.lease):
            with self.assertRaisesRegex(Deferred, "lease_held"):
                self.app.recover()
        self.app.pending = True
        with self.assertRaisesRegex(Deferred, "ci_pending"):
            self.app.recover()
        self.assertEqual(self.app.calls, [])

    def test_stale_or_future_readiness_blocks_recovery(self):
        for offset in [-151, 30]:
            (self.app.root / "status.json").write_text(json.dumps(
                {"time": time.time() + offset, "state": "optional_allowed"}))
            self.assertEqual(self.app.gate(), "supervisor_not_ready")

    def test_restart_cooldown_avoids_crash_loop(self):
        self.app.tick()
        self.app.clock = 120
        self.app.tick()
        self.app.owned = []
        self.app.clock = 719
        self.app.tick()
        self.assertEqual(self.app.calls.count("start"), 1)
        self.app.clock = 720
        self.app.tick()
        self.assertEqual(self.app.calls.count("start"), 2)

    def test_dry_run_does_not_stop_start_or_run(self):
        for normal, pending in [(False, False), (True, True), (True, False)]:
            self.app.normal, self.app.pending = normal, pending
            self.app.tick(dry_run=True)
        self.assertEqual(self.app.calls, [])

    def test_stop_failure_never_reaches_runner_spawn(self):
        with patch.object(self.app, "stop_optional", side_effect=Deferred("stop_failed")), \
                patch("handoff.subprocess.Popen") as spawn:
            with self.assertRaisesRegex(Deferred, "stop_failed"):
                Handoff.run_job(self.app, 0)
            spawn.assert_not_called()

    def test_exec_child_keeps_lease_after_supervisor_releases_its_copy(self):
        with locked(self.app.lease) as fd:
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(.2)"], pass_fds=(fd,))
        try:
            with self.assertRaises(Deferred):
                with locked(self.app.lease):
                    self.fail("inherited lease was released")
        finally:
            child.wait()
        with locked(self.app.lease):
            pass

    def test_symlink_lock_is_never_followed(self):
        target = self.app.root / "original"
        target.write_text("unchanged")
        self.app.lease.symlink_to(target)
        with self.assertRaises(OSError):
            with locked(self.app.lease):
                pass
        self.assertEqual(target.read_text(), "unchanged")

    def test_only_runner_daemons_are_detected(self):
        data = "12 bin/gitlab-runner bin/gitlab-runner run --syslog\n13 bin/gitlab-runner bin/gitlab-runner run-single --runner x\n14 bin/gitlab-runner bin/gitlab-runner verify\n15 bin/node node stratum.js\n"
        self.assertEqual(runner_processes(data), [12, 13])

    def test_job_tags_must_match(self):
        runner = {"tag_list": ["local"], "run_untagged": False}
        self.assertTrue(eligible({"tag_list": ["local"]}, runner))
        self.assertFalse(eligible({"tag_list": []}, runner))
        self.assertFalse(eligible({"tag_list": ["gpu"]}, runner))
        with self.assertRaises(Deferred):
            eligible({}, runner)

    def test_remote_pause_is_respected_without_updating_gitlab(self):
        self.app.c.update(runner_id=1, project_ids=[2])
        runner = dict(id=1, runner_type="project_type", access_level="not_protected",
                      locked=True, tag_list=["local"], run_untagged=False,
                      paused=True, projects=[{"id": 2}])
        with patch.object(self.app, "api", side_effect=[runner, []]) as api:
            self.assertFalse(Handoff.queue(self.app))
            self.assertEqual(api.call_count, 2)

    def test_runner_scope_change_fails_closed(self):
        self.app.c.update(runner_id=1, project_ids=[2])
        with patch.object(self.app, "api", return_value={"id": 1, "runner_type": "instance_type"}):
            with self.assertRaisesRegex(Deferred, "unsupported_runner_scope"):
                Handoff.queue(self.app)

    def test_startup_cooldown_survives_supervisor_restart(self):
        (self.app.root / "last-start.json").write_text(json.dumps({"time": time.time()}))
        other = Fake(self.temp.name)
        self.assertFalse(other.cooldown_ready())
        (self.app.root / "last-start.json").write_text("damaged")
        self.assertFalse(other.cooldown_ready())

    def test_real_foreground_child_inherits_lock_until_it_finishes(self):
        marker = self.app.root / "child-result"
        code = ("import fcntl,os,pathlib; fd=os.open(" + repr(str(self.app.lease)) + ",os.O_RDWR); "
                "\ntry: fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)"
                "\nexcept BlockingIOError: pathlib.Path(" + repr(str(marker)) + ").write_text('locked')")
        self.app.c.update(health_command=[sys.executable, "-c", "pass"],
                          admission_command=[sys.executable, "-c", code],
                          runner_binary="fake-runner", runner_config="fake.toml", runner_name="test")
        with locked(self.app.lease) as fd:
            Handoff.run_job(self.app, fd)
        self.assertEqual(marker.read_text(), "locked")
        self.assertIsNone(self.app.child)
        self.assertIsNone(self.app.normal_since)

    def test_shutdown_signals_only_owned_child_gracefully(self):
        from unittest.mock import Mock
        import signal
        self.app.child = Mock()
        self.app.child.poll.return_value = None
        self.app.shutdown()
        self.app.child.send_signal.assert_called_once_with(signal.SIGQUIT)
        self.assertTrue(self.app.stopping)


if __name__ == "__main__":
    unittest.main()
