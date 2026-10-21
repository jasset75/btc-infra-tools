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

    def test_burst_limit_is_bounded_and_opt_in(self):
        self.assertEqual(self.app.max_activations, 1)
        for value in [0, 5, -1, True, "2", 2.0]:
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "runner_max_activations"):
                Dispatcher({**self.app.c, "runner_max_activations": value})

    def test_burst_stops_at_limit_and_keeps_native_single_job_invocation(self):
        self.app.max_activations = 2
        events = []
        def spawn(*args, **kwargs):
            self.assertEqual(args[0][-4:], ["--max-builds", "1", "--wait-timeout", "60"])
            events.append("spawn")
            child = Mock()
            child.wait.side_effect = lambda: events.append("complete") or 0
            return child
        with patch.object(self.app, "queue_hint", return_value=True) as queue, \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", return_value=([], [])), \
                patch("runner_dispatch.command", side_effect=lambda *a, **kw: events.append("health")), \
                patch("runner_dispatch.subprocess.Popen", side_effect=spawn):
            self.assertEqual(self.app.dispatch_burst(), "runner_finished")
        self.assertEqual(self.app.completed_activations, 2)
        self.assertEqual(queue.call_count, 2)
        self.assertEqual(events, ["health", "spawn", "complete", "health"] * 2)
        with lease(self.app.root / "workload.lock"):
            pass

    def test_burst_stops_when_queue_is_empty_or_runner_is_paused(self):
        self.app.max_activations = 4
        with patch.object(self.app, "dispatch", side_effect=["runner_finished", "no_pending_work"]) as dispatch:
            self.assertEqual(self.app.dispatch_burst(), "runner_finished")
        self.assertEqual(dispatch.call_count, 2)
        self.assertEqual(self.app.completed_activations, 1)

    def test_burst_never_retries_admission_or_health_deferral(self):
        self.app.max_activations = 4
        for reason in ["runner_admission_deferred", "command_failed"]:
            with self.subTest(reason=reason), \
                    patch.object(self.app, "dispatch", side_effect=["runner_finished", Deferred(reason)]) as dispatch:
                with self.assertRaisesRegex(Deferred, reason):
                    self.app.dispatch_burst()
                self.assertEqual(dispatch.call_count, 2)

    def test_burst_shutdown_and_dry_run_do_not_start_followup_activation(self):
        self.app.max_activations = 4
        def finish():
            self.app.stopping = True
            return "runner_finished"
        with patch.object(self.app, "dispatch", side_effect=lambda *_: finish()) as dispatch:
            self.assertEqual(self.app.dispatch_burst(), "runner_finished")
            dispatch.assert_called_once()
        self.app.stopping = False
        self.app.completed_activations = 0
        with patch.object(self.app, "dispatch", return_value="would_stop_optional_and_request_runner_admission") as dispatch:
            self.assertEqual(self.app.dispatch_burst(True), "would_stop_optional_and_request_runner_admission")
            dispatch.assert_called_once_with(True)

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
        self.assertEqual(events, ["stop-optional", "read-only-health", "spawn", "read-only-health"])
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
            with self.assertRaisesRegex(Deferred, "runner_admission_deferred"):
                self.app.dispatch()

    def test_runner_only_profile_does_not_change_optional_admission(self):
        self.app.c["runner_admission_command"] = ["host-admission", "--ci-runner", "--"]
        child = Mock()
        child.wait.return_value = 0
        with patch.object(self.app, "queue_hint", return_value=True), \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", return_value=([], [])), \
                patch("runner_dispatch.command", return_value=""), \
                patch("runner_dispatch.subprocess.Popen", return_value=child) as spawn:
            self.assertEqual(self.app.dispatch(), "runner_finished")
        argv = spawn.call_args.args[0]
        self.assertEqual(argv[:3], ["host-admission", "--ci-runner", "--"])
        self.assertEqual(argv[-4:], ["--max-builds", "1", "--wait-timeout", "60"])
        self.assertEqual(self.app.c["admission_command"], ["host-admission"])

    def test_failed_health_never_claims_work_even_with_runner_profile(self):
        self.app.c["runner_admission_command"] = ["host-admission", "--ci-runner", "--"]
        with patch.object(self.app, "queue_hint", return_value=True), \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", return_value=([], [])), \
                patch("runner_dispatch.command", side_effect=Deferred("mining_unhealthy")), \
                patch("runner_dispatch.subprocess.Popen") as spawn:
            with self.assertRaisesRegex(Deferred, "mining_unhealthy"):
                self.app.dispatch()
            spawn.assert_not_called()

    def test_completed_runner_is_followed_by_health_check(self):
        child = Mock()
        child.wait.return_value = 0
        with patch.object(self.app, "queue_hint", return_value=True), \
                patch.object(self.app, "runner_present", return_value=False), \
                patch.object(self.app, "containers", return_value=([], [])), \
                patch("runner_dispatch.command", side_effect=["", Deferred("mining_unhealthy")]), \
                patch("runner_dispatch.subprocess.Popen", return_value=child):
            with self.assertRaisesRegex(Deferred, "mining_unhealthy"):
                self.app.dispatch()
            child.send_signal.assert_not_called()

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
