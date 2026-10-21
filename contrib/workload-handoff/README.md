# Optional workload / GitLab Runner handoff

`handoff.py` is an optional-workload supervisor on macOS for one locked, unprotected, project-scoped GitLab Docker runner and one optional Podman Compose project. Python 3.11+ and the existing authenticated `glab` CLI are required. Host details and commands live in external TOML; credentials remain in their existing stores.

The runner has its own macOS activation: `runner_dispatch.py`, scheduled every 60 seconds by launchd, independently of Belter and the optional-workload supervisor. It uses the GitLab API only to skip idle activations. If that API is unavailable or its result is uncertain, it falls back to the native runner job protocol and authentication. Before launching `gitlab-runner run-single --max-builds 1` through the host admission guard, it holds the workload lease, stops the optional project, verifies its containers are gone and checks the protected workload. The native runner completes artifact upload and executor cleanup before exiting. With `runner_managed_externally = true`, the supervisor never launches the runner; it only controls the optional project. The optional project can return only after 120 seconds of normal pressure with no runner process, active foreign container or eligible queued job. It uses cached images and explicit container limits. Startup retries are separated by at least 600 seconds. Jobs arriving between checks wait in GitLab.

## Commands

```sh
python3 contrib/workload-handoff/runner_dispatch.py --config handoff.toml --dry-run
python3 contrib/workload-handoff/runner_dispatch.py --config handoff.toml
python3 contrib/workload-handoff/handoff.py --config handoff.toml run --once --dry-run
python3 contrib/workload-handoff/handoff.py --config handoff.toml run
python3 contrib/workload-handoff/handoff.py --config handoff.toml gate
python3 contrib/workload-handoff/handoff.py --config handoff.toml recover
python3 contrib/workload-handoff/handoff.py --config handoff.toml status
python3 contrib/workload-handoff/handoff.py --config handoff.toml pause-optional
python3 contrib/workload-handoff/handoff.py --config handoff.toml resume-optional
```

Exit 0 means completed; 75 means deferred/inhibited. Unexpected configuration or I/O errors exit nonzero and do not enable recovery. `run` retries deferred checks at the configured interval, logging state changes. `--dry-run` avoids service mutations; use a separate `state_dir` for inspection because it records diagnostic status. `pause-optional` persists a manual pause, applied at the next cycle. It does not interrupt CI. `resume-optional` removes that intent; normal admission and settling still apply. A GitLab remote pause is respected and never modified.

## Independent runner activation

Set `runner_managed_externally = true` at the top level of `handoff.toml`. Run `runner_dispatch.py --config <config_dir>/handoff.toml` from the runner's own LaunchAgent with `RunAtLoad=true`, `StartInterval=60`, `AbandonProcessGroup=true`, a bounded `ExitTimeOut` (for example 30 seconds), and no `KeepAlive`. Keep the original persistent runner daemon disabled. launchd does not overlap activations of a running job; an additional OS lock also protects manual invocations.

To avoid waiting for another periodic activation between validation and deployment,
set `runner_max_activations = 2` in the same configuration. The default remains one;
the accepted range is 1–4. Each activation still runs the exact guarded
`run-single --max-builds 1 --wait-timeout 60` command. Native cleanup and the
post-run health check finish before another activation can begin. Queue eligibility,
remote pause, process/container exclusion, mining health and host admission are
checked again for every activation, with the workload and host locks reacquired.
An empty queue, shutdown, failed health check or admission deferral ends the burst;
there is no retry loop and no increase in concurrency. Dry-run checks only once.
The JSON result includes the number of completed runner activations (not a claim
about how many jobs the native runner received). Exit 0 means an idle/successful
activation; exit 75 retains the existing deferral behavior, even after an earlier
activation completed successfully.

The runner LaunchAgent may use `StartInterval=15` to reduce initial queue latency.
This only increases lightweight queue checks while idle; the optional supervisor's
60-second polling, 120-second settling and recovery cooldown stay unchanged. Apply
LaunchAgent changes only while the dispatcher and runner are idle, under the
dispatcher lock, and preserve `AbandonProcessGroup=true`. Do not unload a running
dispatcher to change its interval. To roll back, restore `runner_max_activations=1`
and the 60-second runner interval at an idle boundary. No protected service needs
to be restarted or signalled.

The dispatcher exits between activations and remains in the foreground only while a runner is polling or executing a job. It shares no imports, liveness checks or readiness requirements with `handoff.py` or Belter. The mining probe is also independent of the supervisor module. A failed optional coordinator therefore cannot prevent native GitLab polling by requiring a healthy coordinator process, status file or successful `glab` call. A running coordinator may briefly hold the shared workload lease while checking or changing the optional project; the dispatcher retries at its next activation. The lease is inherited by the native runner, and an unreaped runner receives only SIGQUIT on dispatcher shutdown so its active job can finish.

The dispatcher returns 0 after an idle activation or a completed runner invocation, 75 when resource admission or a workload lease defers it, and nonzero for unexpected configuration/I/O errors. Native runner output goes to `runner.log`; dispatcher events contain no command output or credentials. `--dry-run` performs queue/container inspection without stopping workloads or spawning a runner. GitLab's remote pause remains respected. Disallowed memory pressure, an unavailable Podman API, failed mining health or foreign workloads still defer local CI without claiming a job. This resource policy is intentional; CI is not guaranteed to run on an unhealthy or resource-constrained host.

### Runner-specific memory policy

An optional `runner_admission_command` overrides `admission_command` only in the
independent dispatcher. Without it, behavior is unchanged. This allows an
explicitly authorized host policy to admit bounded CI at warning pressure while
optional services, previews and local builds continue to require normal pressure.
For a host guard implementing the bounded runner profile:

```toml
runner_admission_command = ["python3", "<host_policy>/budget.py", "--ci-runner", "--"]
```

The host profile must validate the exact single-job native runner invocation and
its container limits before allowing warning pressure: concurrency one, Docker
executor, memory at most 1536 MiB with no additional container swap, two CPUs or
less, no service sidecars, no privileged containers and OOM termination enabled.
Keep the VM bounded separately. Critical/unknown pressure still blocks new work;
the shared host lease still serializes CI with other guarded work. Never apply
the runner profile to the optional project's `admission_command`.

Mining health is checked both before admission and after the native runner exits.
Exit 75 from admission reports `runner_admission_deferred`; another nonzero native
exit reports `runner_failed`, both with the exit code and without raw output.
A failed post-run probe reports its fixed check stage; it does not rewrite the
completed GitLab job or signal protected services. No resident monitor or active
job termination is added. Remove the runner override to roll back on the next
activation without interrupting an existing job.

A completion timestamp enforces settling even when a short job starts and finishes between coordinator observations. No optional-workload restart is authorized by the runner. If the supervisor or Belter fails, CI can proceed independently; automatic optional-workload recovery may remain unavailable. The legacy combined supervisor mode remains available only when `runner_managed_externally` is false or absent; never deploy that mode alongside the independent dispatcher.

## Watchdog integration

Add the following to the existing watch, replacing its unguarded recovery command:

```toml
allow_command = 'python3 <repo>/contrib/workload-handoff/handoff.py --config <config_dir>/handoff.toml gate'
recovery = 'python3 <repo>/contrib/workload-handoff/handoff.py --config <config_dir>/handoff.toml recover'
```

The watchdog checks `allow_command` before diagnosis and immediately before recovery. Code 75 logs `watch.paused`; other nonzero exits or timeouts log `watch.blocked`. Neither starts recovery. The recovery wrapper acquires the same OS lock as the runner and repeats admission, eliminating the check/start race. A code-75 recovery is a deferral, not a failed repair. An absent/stale supervisor readiness lease blocks recovery.

## Deployment and rollback

1. Back up the runner LaunchAgent, watchdog configuration/binary and Compose override. Keep credentials out of version control and logs.
2. Validate the watchdog build and Python tests. Verify Core, P2P and primary-miner connectivity. Optional work still requires normal pressure; warning admits CI only with an explicitly authorized bounded runner profile. Unknown/critical pressure never admits new work.
3. Drain the old runner using its main PID and SIGQUIT, preserving active work; disable automatic respawning first. Confirm it has exited and no job/helper container remains. Never terminate a job to make the switch.
4. Set external-runner mode and install the independent runner LaunchAgent described above. Run the optional supervisor separately under launchd with `AbandonProcessGroup=true`, `KeepAlive=true`, a 60-second throttle and the correct PATH. The old persistent runner service must remain disabled. Never run both admission models intentionally.
5. Install the guarded watchdog configuration. Keep recovery restricted to the optional project; do not use dependency bring-up commands that might alter protected services.
6. Inspect status and logs. Test a queued CI job before declaring the production handoff fully validated. Watch for memory pressure and container OOMs during the first optional-service synchronization.

For rollback, disable further runner activations, request graceful dispatcher shutdown and wait for any job to finish; then stop the optional supervisor. Confirm no runner/optional containers remain, stop only the optional project, then restore the original runner LaunchAgent. Keep the watchdog paused unless its replacement recovery remains guarded. Restoring an unguarded watchdog while CI runs defeats the policy.

## Safety boundaries

Only the configured optional Compose project is stopped. No Core, Stratum, miner, networking, chats or app processes are managed. The health command must be read-only and exit nonzero if protected service health is uncertain. Existing or unknown runner processes/containers inhibit recovery. A shared advisory lock is inherited by the foreground runner across exec; it survives supervisor failure until the runner exits. A launchd process-group kill would defeat that behavior, hence the required launchd setting.

These checks serialize cooperating entry points. An administrator can bypass them by starting a service directly. The host guard is admission control, not a RAM reservation; it cannot regulate app-internal helpers or guarantee future block submission. During CI, the supervisor does not terminate active jobs because pressure rises. Container limits remain necessary.

The optional `mining_health.py --config handoff.toml` probe checks Core synchronization, enabled/connected P2P, primary Stratum selection, positive reported hash rate and an established miner TCP peer. Its `[mining]` config supplies `bitcoin_cli`, `miner_url`, `stratum_host`, `stratum_port` and `tcp_peer`; it never changes these services. Exit 0 confirms the snapshot; exit 1 inhibits admission. This does not test found-block submission or promise future availability.

For a launchd job that cannot read the node's data volume, configure the probe's optional loopback RPC client instead of accessing the data directory through `bitcoin-cli`:

```toml
[mining.rpc]
endpoint = "http://127.0.0.1:8332/"
credentials_file = "<config_dir>/existing-rpc-client.env"
user_key = "RPC_USER"
password_key = "RPC_PASSWORD"
```

The referenced environment file contains existing RPC credentials as literal `KEY=value` assignments, optionally quoted or prefixed with `export`. Keep its restrictive permissions; the probe reads it without executing shell code, copying credentials, or changing the node or its existing client. The endpoint must be loopback. Proxies and HTTP redirects are disabled, credentials are sent only in the authorization header, and the only supported RPC calls are `getblockchaininfo` and `getnetworkinfo`, each with a five-second timeout. When `[mining.rpc]` is present, `bitcoin_cli` is not invoked; configurations without that table retain the CLI behavior. All other mining-health requirements remain enforced.

Failed dispatcher commands report the operation and exit code; timeouts report the operation. A failed mining probe also reports its check stage and exception type when available. Command arguments, exception messages, subprocess output and credentials are excluded. Use these fields to distinguish a host-resource deferral from a failed probe or unavailable container runtime before changing services.

Run the focused lightweight tests with `python3 -m unittest discover -s contrib/workload-handoff -p 'test_*.py'`. Run Rust compilation/tests through the shared host admission command.
