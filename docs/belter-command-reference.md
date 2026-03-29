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
  - `podman_compose` services: resolves `compose_file`/`compose_override`/`project`, queries runtime status via `podman compose ... ps -q`, and reports:
    - `data.state = running|stopped|unknown`
    - `data.running_containers` (container ids when available)
    - `data.query_error` when runtime query cannot be completed.
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
- Status: planned, not implemented yet.
- Parameters:
  - `name` (required)
- Intent:
  - Provide a reboot-safe flow to bring a managed service back after host restart or maintenance.
  - First specialized implementation target is `mempool` on macOS with Podman.
  - Keep generic `service start|stop|restart` simple and manager-oriented.
- Behavior:
  - Dispatches to a service-specific bring-up workflow.
  - First iteration supports `name = mempool`.
  - Loads `.env` and resolves the configured `service.mempool` `compose_file`, `compose_override`, and `project`.
  - For `mempool`, runs preflight checks before any compose lifecycle action:
    - validate `podman` is installed,
    - validate the configured compose files exist,
    - validate `podman compose` is usable on the host,
    - check Podman runtime availability and start `podman machine` when the VM exists but is stopped,
    - fail fast with actionable diagnostics if the runtime cannot be reached.
  - For `mempool`, runs controlled bring-up using the configured compose project:
    - `podman compose ... up -d`
  - For `mempool`, runs post-start validation:
    - `podman compose ... ps`
    - HTTP probes against:
      - `/api/v1/backend-info`
      - `/api/blocks/tip/height`
      - `/api/mempool`
  - Returns structured output that differentiates:
    - preflight failure,
    - compose bring-up failure,
    - runtime started but HTTP validation failed,
    - full success.
- Retry policy for first iteration:
  - Retries are reserved for transient readiness only, not for configuration or dependency errors.
  - Candidate retry points:
    - waiting for `podman machine` to become reachable after a successful start request,
    - waiting for `podman compose ... ps` to show running containers after `up -d`,
    - waiting for `mempool` HTTP endpoints to return healthy responses.
  - Candidate non-retriable failures:
    - missing `podman` binary,
    - missing compose files,
    - unusable `podman compose` provider,
    - unresolved environment placeholders,
    - invalid host port policy or known static config errors,
    - authentication failures against Bitcoin RPC when clearly reported as such.
  - Recommended initial backoff:
    - `attempts = 5`
    - `initial_delay = 1s`
    - exponential factor `2`
    - `max_delay = 8s`
  - Design guidance:
    - first implementation may use simple synchronous polling and sleep;
    - introducing `tokio-retry` is acceptable later if async execution becomes justified by multiple readiness loops or richer orchestration.
- Non-goals for the first iteration:
  - no mutation of upstream `mempool` compose files,
  - no secret creation or rotation,
  - no automatic patching of invalid host port mappings,
  - no Bitcoin Core RPC credential provisioning.
- Operator assumptions:
  - `service.mempool` points to external compose files, not to a pristine upstream clone,
  - the compose override already encodes the correct RPC host for Podman on macOS (`host.containers.internal`),
  - the published web port is non-privileged (recommended: `8080`).

Examples:

```bash
belter service bring-up mempool
belter --json service bring-up mempool
belter --dry-run service bring-up mempool
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
