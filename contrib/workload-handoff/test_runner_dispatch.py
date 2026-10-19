import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

from runner_dispatch import Deferred, Dispatcher, command, lease, main


class DispatcherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.app = Dispatcher({"version": 1, "state_dir": self.temp.name,
            "runner_id": 1, "project_ids": [2], "stop_command": ["stop-optional"],
            "health_command": ["read-only-health"], "admission_command": ["host-admission"],
            "runner_binary": "gitlab-runner", "runner_config": "runner.toml", "runner_name": "test"})

    def runner(self, **extra):
        return dict(id=1, paused=False, projects=[{"id": 2}], tag_list=["local"], run_untagged=False, **extra)

    def test_queue_api_failure_falls_back_to_native_runner(self):
        for error in (Deferred("glab_failed"), ValueError("invalid JSON")):
            with patch.object(self.app, "api", side_effect=error):
                self.assertTrue(self.app.queue_hint())

    def test_empty_queue_avoids_stopping_optional_work(self):
        with patch.object(self.app, "api", side_effect=[self.runner(), []]), \
                patch.object(self.app, "containers") as containers:
            self.assertEqual(self.app.dispatch(), "no_pending_work")
            containers.assert_not_called()

    def test_matching_jobs_and_unknown_scope_wake_native_runner(self):
        for response in ([self.runner(), [{"tag_list": ["local"]}]], [{"id": 1}], [{"id": 9, "paused": True}]):
            with patch.object(self.app, "api", side_effect=response):
                self.assertTrue(self.app.queue_hint())

    def test_remote_user_pause_is_respected(self):
        with patch.object(self.app, "api", return_value={"id": 1, "paused": True}):
            self.assertFalse(self.app.queue_hint())

    def test_lease_prevents_overlapping_work(self):
        with patch.object(self.app, "queue_hint", return_value=True), lease(self.app.root / "workload.lock"):
            with self.assertRaisesRegex(Deferred, "workload_active"):
                self.app.dispatch()

    def test_optional_stop_and_health_precede_native_spawn_without_coordinator(self):
        events = []
        child = Mock()
        child.wait.return_value = 0
        def spawn(*args, **kwargs):
            events.append("spawn")
            self.assertTrue(kwargs["pass_fds"])
            return child
        with patch.object(self.app, "queue_hint", return_value=True), \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", side_effect=[([{}], []), ([], [])]), \
                patch("runner_dispatch.command", side_effect=lambda argv, timeout, **kwargs: events.append(argv[0])), \
                patch("runner_dispatch.subprocess.Popen", side_effect=spawn):
            self.assertEqual(self.app.dispatch(), "runner_finished")
        self.assertEqual(events, ["stop-optional", "read-only-health", "spawn"])
        self.assertTrue((self.app.root / "runner-finished.json").exists())

    def test_stop_failure_or_foreign_containers_never_spawn_runner(self):
        for containers in [([{}], []), ([], [{}])]:
            with patch.object(self.app, "queue_hint", return_value=True), \
                    patch.object(self.app, "runner_present", return_value=False), \
                    patch.object(self.app, "containers", return_value=containers), \
                    patch("runner_dispatch.command", side_effect=Deferred("stop_failed")), \
                    patch("runner_dispatch.subprocess.Popen") as spawn:
                with self.assertRaises(Deferred):
                    self.app.dispatch()
                spawn.assert_not_called()

    def test_host_admission_deferral_keeps_work_queued(self):
        child = Mock()
        child.wait.return_value = 75
        with patch.object(self.app, "queue_hint", return_value=True), \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", return_value=([], [])), \
                patch("runner_dispatch.command", return_value=""), \
                patch("runner_dispatch.subprocess.Popen", return_value=child):
            with self.assertRaisesRegex(Deferred, "runner_deferred_or_failed"):
                self.app.dispatch()

    def test_dry_run_never_stops_containers_or_spawns_runner(self):
        with patch.object(self.app, "queue_hint", return_value=True), \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", return_value=([{}], [])), \
                patch("runner_dispatch.command") as command, \
                patch("runner_dispatch.subprocess.Popen") as spawn:
            self.assertEqual(self.app.dispatch(True), "would_stop_optional_and_request_runner_admission")
            command.assert_not_called()
            spawn.assert_not_called()

    def test_worker_import_and_health_probe_do_not_depend_on_handoff_module(self):
        code = "import sys; sys.modules['handoff']=None; import runner_dispatch, mining_health"
        result = subprocess.run([sys.executable, "-c", code], cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0)

    def test_foreground_child_retains_lease_if_dispatcher_dies(self):
        with lease(self.app.root / "workload.lock") as fd:
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(.2)"], pass_fds=(fd,))
        try:
            with self.assertRaises(Deferred):
                with lease(self.app.root / "workload.lock"):
                    self.fail("lease released underneath child")
        finally:
            child.wait()

    def test_shutdown_drains_only_owned_child(self):
        import signal
        self.app.child = Mock()
        self.app.child.poll.return_value = None
        self.app.shutdown()
        self.app.child.send_signal.assert_called_once_with(signal.SIGQUIT)

    def test_cli_runs_one_activation_and_exits(self):
        config = self.app.root / "config.toml"
        config.write_text('version = 1\nstate_dir = '+json.dumps(self.temp.name)+'\n')
        with patch("sys.argv", ["runner_dispatch.py", "--config", str(config)]), \
                patch("runner_dispatch.Dispatcher", return_value=self.app), \
                patch.object(self.app, "queue_hint", return_value=False), \
                patch("runner_dispatch.signal.signal"):
            self.assertEqual(main(), 0)

    def test_command_failure_identifies_operation_without_command_output(self):
        result = subprocess.CompletedProcess(["probe"], 1,
            '{"mining_healthy":false,"failed_check":"miner_api","error_type":"URLError"}',
            "secret-token")
        with patch("runner_dispatch.subprocess.run", return_value=result):
            with self.assertRaises(Deferred) as raised:
                command(["probe", "secret-argument"], operation="mining_health")
        self.assertEqual(raised.exception.details, {"operation": "mining_health", "returncode": 1,
            "failed_check": "miner_api", "error_type": "URLError"})
        self.assertNotIn("secret", str(raised.exception) + repr(raised.exception.details))

    def test_command_timeout_identifies_operation(self):
        with patch("runner_dispatch.subprocess.run", side_effect=subprocess.TimeoutExpired("probe", 1)):
            with self.assertRaises(Deferred) as raised:
                command(["probe"], operation="container_inspection")
        self.assertEqual(str(raised.exception), "command_timeout")
        self.assertEqual(raised.exception.details, {"operation": "container_inspection"})


if __name__ == "__main__":
    unittest.main()
