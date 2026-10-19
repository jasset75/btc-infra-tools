# Optional workload / GitLab Runner handoff

`handoff.py` is a macOS host supervisor for one locked, unprotected, project-scoped GitLab Docker runner and one optional Podman Compose project. Python 3.11+ and the existing authenticated `glab` CLI are required. Host details and commands live in external TOML; credentials remain in their existing stores.

The supervisor replaces the persistent `gitlab-runner run` service. Every 60 seconds it checks eligible pending jobs, including their tags. It stops the optional project, verifies its containers are gone, checks the protected workload, and launches `gitlab-runner run-single --max-builds 1` through the host admission guard. That process completes artifact upload and executor cleanup before exiting. The optional project can return only after 120 seconds of normal pressure with no runner process, active foreign container or eligible queued job. It uses cached images and explicit container limits. Startup retries are separated by at least 600 seconds. Jobs arriving between checks wait in GitLab.

## Commands

```sh
python3 contrib/workload-handoff/handoff.py --config handoff.toml run --once --dry-run
python3 contrib/workload-handoff/handoff.py --config handoff.toml run
python3 contrib/workload-handoff/handoff.py --config handoff.toml gate
python3 contrib/workload-handoff/handoff.py --config handoff.toml recover
python3 contrib/workload-handoff/handoff.py --config handoff.toml status
python3 contrib/workload-handoff/handoff.py --config handoff.toml pause-optional
python3 contrib/workload-handoff/handoff.py --config handoff.toml resume-optional
```

Exit 0 means completed; 75 means deferred/inhibited. Unexpected configuration or I/O errors exit nonzero and do not enable recovery. `run` retries deferred checks at the configured interval, logging state changes. `--dry-run` avoids service mutations; use a separate `state_dir` for inspection because it records diagnostic status. `pause-optional` persists a manual pause, applied at the next cycle. It does not interrupt CI. `resume-optional` removes that intent; normal admission and settling still apply. A GitLab remote pause is respected and never modified.

## Watchdog integration

Add the following to the existing watch, replacing its unguarded recovery command:

```toml
allow_command = 'python3 <repo>/contrib/workload-handoff/handoff.py --config <config_dir>/handoff.toml gate'
recovery = 'python3 <repo>/contrib/workload-handoff/handoff.py --config <config_dir>/handoff.toml recover'
```

The watchdog checks `allow_command` before diagnosis and immediately before recovery. Code 75 logs `watch.paused`; other nonzero exits or timeouts log `watch.blocked`. Neither starts recovery. The recovery wrapper acquires the same OS lock as the runner and repeats admission, eliminating the check/start race. A code-75 recovery is a deferral, not a failed repair. An absent/stale supervisor readiness lease blocks recovery.

## Deployment and rollback

1. Back up the runner LaunchAgent, watchdog configuration/binary and Compose override. Keep credentials out of version control and logs.
2. Validate the watchdog build and Python tests. Verify Core, P2P and primary-miner connectivity. Do not start workloads under warning/unknown memory pressure.
3. Drain the old runner using its main PID and SIGQUIT, preserving active work; disable automatic respawning first. Confirm it has exited and no job/helper container remains. Never terminate a job to make the switch.
4. Run the supervisor under launchd with `AbandonProcessGroup=true`, `KeepAlive=true`, a 60-second throttle and the correct PATH. Its foreground runner inherits the workload lease. The old persistent runner service must remain disabled. Never run both admission models intentionally.
5. Install the guarded watchdog configuration. Keep recovery restricted to the optional project; do not use dependency bring-up commands that might alter protected services.
6. Inspect status and logs. Test a queued CI job before declaring the production handoff fully validated. Watch for memory pressure and container OOMs during the first optional-service synchronization.

For rollback, request graceful supervisor shutdown and wait for any job to finish. Confirm no runner/optional containers remain, stop only the optional project, then restore the original runner LaunchAgent. Keep the watchdog paused unless its replacement recovery remains guarded. Restoring an unguarded watchdog while CI runs defeats the policy.

## Safety boundaries

Only the configured optional Compose project is stopped. No Core, Stratum, miner, networking, chats or app processes are managed. The health command must be read-only and exit nonzero if protected service health is uncertain. Existing or unknown runner processes/containers inhibit recovery. A shared advisory lock is inherited by the foreground runner across exec; it survives supervisor failure until the runner exits. A launchd process-group kill would defeat that behavior, hence the required launchd setting.

These checks serialize cooperating entry points. An administrator can bypass them by starting a service directly. The host guard is admission control, not a RAM reservation; it cannot regulate app-internal helpers or guarantee future block submission. During CI, the supervisor does not terminate active jobs because pressure rises. Container limits remain necessary.

The optional `mining_health.py --config handoff.toml` probe checks Core synchronization, enabled/connected P2P, primary Stratum selection, positive reported hash rate and an established miner TCP peer. Its `[mining]` config supplies `bitcoin_cli`, `miner_url`, `stratum_host`, `stratum_port` and `tcp_peer`; it never changes these services. Exit 0 confirms the snapshot; exit 1 inhibits admission. This does not test found-block submission or promise future availability.

Run the focused lightweight tests with `python3 -m unittest discover -s contrib/workload-handoff -p 'test_*.py'`. Run Rust compilation/tests through the shared host admission command.
