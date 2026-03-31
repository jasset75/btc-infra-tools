# Belter Command Reference

Detailed command reference for the `belter` CLI.

Status:
- WIP
- Some commands still return scaffold responses.

## Global Flags
- `--config <PATH>` (optional): config file path. Default is `belter.toml`.
- `--json` (optional): output as JSON envelope.
- `--dry-run` (optional): simulate command without making actual changes. This is specially useful for testing commands on machines that are not the actual infrastructure target (e.g. your local development machine).

### JSON Envelope
When `--json` is set, commands emit a single structured JSON object on `stdout`.

Top-level fields:
- `ts`: RFC3339 timestamp for the envelope.
- `command`: stable command identifier, for example `service.restart`.
- `status`: command outcome, currently `ok` or `error`.
- `message`: short canonical summary of the result.
- `dry_run`: whether the command was simulated.
- `data`: command payload.
- `events`: structured auxiliary events; safe to ignore for consumers that only need the main result.

Event fields:
- `ts`: RFC3339 timestamp for the event.
- `level`: event severity, for example `debug`, `info`, `warning`, `error`, `fatal`.
- `code`: stable machine-friendly event code.
- `message`: human-readable event message.
- `details`: structured event payload.

Example:
```json
{
  "ts": "2026-03-13T13:05:00.376301Z",
  "command": "service.restart",
  "status": "ok",
  "message": "would restart service `bitcoind`",
  "dry_run": true,
  "data": {
    "plan": {
      "operations": [
        {
          "RestartService": {
            "manager": "launchd",
            "unit": "system/com.bitcoind.node"
          }
        }
      ]
    }
  },
  "events": [
    {
      "ts": "2026-03-13T13:05:00.376250Z",
      "level": "info",
      "code": "service.restart.preview",
      "message": "1. Would restart `launchd` unit `system/com.bitcoind.node`",
      "details": {
        "operation_index": 1,
        "manager": "launchd",
        "unit": "system/com.bitcoind.node"
      }
    }
  ]
}
```

### Example Usage
```bash
# Safe to run locally, even if the target infrastructure (e.g. bitcoind) is not present
belter --dry-run service restart bitcoind
```

## Execution Scope Notation

Command docs use the following scope markers:

- `Scope: local-only`: command must run on the managed host because it touches local process managers, files, or privileged runtime state.
- `Scope: remote-capable`: command can be run from another machine if network reachability and credentials are sufficient.
- `Scope: mixed`: command supports both patterns depending on the selected target or backend.

Documentation examples use reserved example addresses such as `192.0.2.10` and `pool.example.internal`, not real operator IPs.

Semantic guidance:

- `info`: read-only descriptive data and operator-facing metrics.
- `health`: checks and health-oriented probes; may evolve toward explicit readiness/liveness semantics.
- `service`: local control-plane actions over managed services.

## Command Tree
```text
belter
  config
    init [--path <PATH>] [--force]
    validate
    show
  info
    pool [target] [--port <PORT> | --url <URL>]
  service
    list
    status [name] [--ui <auto|cli|tui> | --tui]
    start <name>
    stop <name>
    restart <name>
    bring-up <name>
    logs <name> [--follow]
  health
    check [--all | --id <ID>] [--ui <auto|cli|tui> | --tui]
    snapshot
    pool [target] [--port <PORT> | --url <URL>]
  run
    action <id> [--dry-run]
  tui
    dashboard
```

## config

### `config init`
- Parameters:
  - `--path <PATH>` (optional)
  - `--force` (optional)
- Behavior:
  - Creates a config template (default output path: `belter.toml`).
  - The default template includes:
    - `service.bitcoind` (`manager = "launchd"`, `unit = "${BITCOIND_LAUNCHD_UNIT}"`)
    - `service.stratum` (`manager = "launchd"`, `unit = "${STRATUM_LAUNCHD_UNIT}"`)
    - `service.mempool` (`manager = "podman_compose"` with compose placeholders)

### `config validate`
- Parameters:
  - `--write-missing` (optional; appends missing required service blocks before validating)
- Behavior:
  - Loads and parses `belter.toml`.
  - Requires service definitions for `bitcoind`, `stratum`, and `mempool`.
  - Validates required fields by manager:
    - `launchd`: `unit` required
    - `podman_compose`: `compose_file` required (`compose_override` and `project` optional)
  - Resolves `${ENV_VAR}` placeholders against environment (`.env` is auto-loaded at CLI startup when present).
  - On failure, reports actionable errors:
    - missing config sections/fields include TOML example snippets,
    - unresolved placeholders include the exact missing env var name and a suggested `export` command.
  - `--write-missing` appends only missing required service blocks; existing blocks are not overwritten.

### `config show`
- Parameters: none
- Behavior: scaffold placeholder response.

## info

### `info pool`
- Scope: `remote-capable`
- Parameters:
  - `target` (optional; host or IP, default: `127.0.0.1`)
  - `--port <PORT>` (optional; default: `3334`)
  - `--url <URL>` (optional; advanced override, mutually exclusive with `target`)
- Behavior:
  - By default, builds `http://<target>:<port>/api/info`.
  - Queries the `public-pool` `/api/info` endpoint.
  - Text mode prints a compact single-line mining summary:
    - `best_share` formatted with metric suffixes (`K`, `M`, `G`, `T`)
    - miner / user agent
    - hashrate in `TH/s`
    - high-score update timestamp
    - process uptime timestamp
  - `--json`: returns a structured envelope with raw numeric fields and human-readable display fields.
  - `--dry-run`: does not perform the HTTP request; returns a simulated payload.

Example:

```bash
belter info pool
belter info pool 192.0.2.10
belter info pool pool.example.internal --port 3334
belter --json info pool
belter info pool --url http://192.0.2.10:3334/api/info
```

## service

### `service list`
- Parameters: none
- Behavior: scaffold static list.

### `service status [name]`
- Scope: `local-only`
- Parameters:
  - `name` (optional, default: all)
  - `--ui <auto|cli|tui>` (optional)
  - `--tui` (optional; shortcut for `--ui tui`; mutually exclusive with `--ui`)
- Behavior:
  - Loads `service.<name>` from config when `name` is provided.
  - `launchd` services: resolves `unit`, queries runtime status via `launchctl print`, and reports state in `data` (`running|stopped|unknown`) with optional `pid`.
  - `podman_compose` services: resolves `compose_file`/`compose_override`/`project`, filters container ids to containers whose Podman `State.Running` is actually `true`, and reports:
    - `data.state = running|stopped|degraded|unknown`
    - `data.running_containers` (container ids when available)
    - `data.query_error` when runtime query cannot be completed.
  - `mempool` has a stronger status contract:
    - `running` requires running containers and `HTTP 200` from `/api/v1/backend-info`
    - `degraded` means containers are up but readiness failed
    - `health_url` is included in JSON output for machine-readable consumers.
  - `podman_machine` services resolve `machine` and report `running|stopped|unknown`.
  - Unknown/unsupported managers return `state=unknown` with descriptive message.
  - `--dry-run`: does not query runtime state; returns a simulated status payload (`data.simulated = true`) and sets `dry_run = true` in the envelope.
  - `--json`: returns machine-readable envelope; command-level `status` indicates CLI execution success, while service runtime state is exposed in `data.state` when available.

### `service start <name>`
- Scope: `local-only`
- Parameters:
  - `name` (required)
- Behavior:
  - Loads `service.<name>` from config.
  - `launchd`: executes start against configured `unit`.
  - `podman_compose`: executes `podman compose ... up -d` using configured compose file(s) and optional project.
  - `--dry-run`: returns simulated plan data without executing commands.
  - `--json`: returns machine-readable envelope including `plan`; dry-run preview events are omitted.

### `service stop <name>`
- Scope: `local-only`
- Parameters:
  - `name` (required)
- Behavior:
  - Loads `service.<name>` from config.
  - `launchd`: executes stop against configured `unit`.
  - `podman_compose`: executes `podman compose ... down` using configured compose file(s) and optional project.
  - `--dry-run`: returns simulated plan data without executing commands.
  - `--json`: returns machine-readable envelope including `plan`; dry-run preview events are omitted.

### `service restart <name>`
- Scope: `local-only`
- Parameters:
  - `name` (required)
- Behavior:
  - Loads `service.<name>` from config.
  - Expands `${ENV_VAR}` placeholders from environment.
  - `launchd`: requires `unit` and runs `launchctl kickstart -k <unit>`.
  - `podman_compose`: requires `compose_file`; optional `compose_override` and `project`.
  - `podman_compose` restart is implemented as `podman compose ... down` followed by `podman compose ... up -d`.
  - If `.env` exists in current directory, it is autoloaded before command execution.
  - `--dry-run`: returns simulated plan data without executing commands.
  - `--json`: returns machine-readable envelope including `plan`; dry-run preview events are omitted.
- Operational notes:
  - For launchd units in `system/...`, restart may require elevation (`sudo -E`).
  - Unit must be full launchd target (`<domain>/<label>`, for example `system/com.bitcoind.node`).

Example for `mempool`:

```bash
belter --config belter.toml service restart mempool
belter --config belter.toml --dry-run --json service start mempool
```

Example for `stratum` (launchd-backed):

```bash
belter --config belter.toml service start stratum
belter --config belter.toml service stop stratum
belter --config belter.toml --dry-run --json service restart stratum
```

### `service logs <name>`
- Scope: `local-only`
- Parameters:
  - `name` (required)
  - `--follow` (optional)
- Behavior: scaffold echo response.

## health

### `health check`
- Scope: `mixed`
- Parameters:
  - `--all` (optional; mutually exclusive with `--id`)
  - `--id <ID>` (optional; mutually exclusive with `--all`)
  - `--ui <auto|cli|tui>` (optional)
  - `--tui` (optional; shortcut for `--ui tui`; mutually exclusive with `--ui`)
- UI behavior differences:
  - Current implementation: no output behavior change yet; mode is reported in output.

### `health snapshot`
- Scope: `local-only`
- Parameters: none
- Behavior: scaffold snapshot response.

### `health pool`
- Scope: `remote-capable`
- Status: compatibility alias for `info pool`
- Parameters:
  - `target` (optional; host or IP, default: `127.0.0.1`)
  - `--port <PORT>` (optional; default: `3334`)
  - `--url <URL>` (optional; advanced override, mutually exclusive with `target`)
- Behavior:
  - Same behavior and output as `info pool`.
  - Kept as a temporary alias while health-oriented pool checks are designed.

Example:

```bash
belter health pool 192.0.2.10
```

### `service bring-up <name>`
- Scope: `local-only`
- Parameters:
  - `name` (required)
- Behavior:
  - Resolves `depends_on` from config and traverses them in dependency order.
  - Uses the primitive manager actions internally; `bring-up` does not introduce a background controller or scheduler.
  - Loads `env_file` for `podman_compose` services before planning or execution.
  - `--dry-run` is deterministic:
    - it shows the full declared bring-up chain,
    - it does not consult the real runtime state of the host.
  - Real execution is state-aware:
    - healthy dependencies are skipped,
    - stopped or unknown dependencies are started,
    - `mempool` in `degraded` state is brought up again.
  - Emits structured events such as:
    - dependency skipped because already healthy,
    - dependency being started,
    - waiting for readiness,
    - readiness reached.

Current implemented specialization:
- `service bring-up mempool`
  - resolves `depends_on = ["bitcoind", "podman_runtime"]`,
  - ensures `service.mempool.env_file` is loaded,
  - starts `bitcoind` only if not already healthy,
  - starts `podman_runtime` only if not already healthy,
  - starts `mempool`,
  - waits for `http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info` to return `200`,
  - fails if readiness does not stabilize within the built-in retry loop.

Readiness policy:
- current implementation uses synchronous polling with exponential backoff
- attempts: `5`
- initial delay: `1s`
- maximum delay: `8s`
- readiness target for `mempool`: `/api/v1/backend-info`

Current design limits:
- no reconciliation loop or daemon mode,
- no automatic secret provisioning,
- no mutation of upstream `mempool` compose files,
- no automatic repair of bad compose configuration.

Examples:

```bash
belter service bring-up mempool
belter --json service bring-up mempool
belter --dry-run service bring-up mempool
```

Typical text-mode events for `mempool`:

```text
[info] bring_up.skipped: `bitcoind` already healthy; skipping
[info] bring_up.starting: starting `podman_runtime`
[info] bring_up.starting: starting `mempool`
[info] bring_up.waiting_readiness: waiting for mempool HTTP readiness
[info] bring_up.ready: mempool backend responded with HTTP 200
```

## run

### `run action <id>`
- Parameters:
  - `id` (required)
  - `--dry-run` (optional)
- Behavior: scaffold echo response.

## tui

### `tui dashboard`
- Parameters: none
- Behavior: scaffold placeholder response.
