# Architecture - Belter CLI/TUI (Rust, infrastructure-agnostic)

## Goal
Design an operations tool with both CLI and TUI interfaces, reusable across different infrastructure setups through declarative configuration.

## Initial Scope
- Primary CLI implemented in Rust.
- Optional TUI for dual-mode commands.
- External configuration for services, health checks, and actions.
- Initial support for an environment similar to `bitcoind + stratum (public-pool) + mempool`, without hardcoding infrastructure specifics.

## Monorepo Strategy (Current)
Current decision: **single monorepo, single binary**.

Rationale:
- maximize early iteration speed
- reduce CI/CD and cross-versioning overhead
- avoid cross-repo development friction while the domain is still evolving

Proposed internal layout:

1. `crates/infractl-core`
- Infrastructure-agnostic domain logic:
  - config loading and validation
  - action execution
  - health check engine
  - structured error and output model

2. `crates/infractl-adapters`
- Integrations by manager/platform:
  - launchd
  - systemd (future)
  - podman/docker compose
  - http

3. `crates/infractl-cli`
- Command-line UX + TUI (`clap` + `ratatui`).
- Produces the `belter` binary.

4. `docs/spec`
- Versioned configuration schema and examples.

## Future Split Plan (If Needed)
We will consider a multi-repo split only when clear signals appear:

1. Two or more real consumers of `core/spec` outside the main binary.
2. Need for independent release cadences (for example, adapters vs CLI).
3. Separate ownership across teams.
4. Monorepo CI starts creating sustained delivery bottlenecks.

Compatibility rule to keep split cost low:
- keep `core` and `spec` contracts stable even while they live in the same repository.

## CLI Design Principles (clig.dev)
- Predictable commands with explicit, consistent naming.
- Human-readable output by default; `--json` for automation.
- Reliable, well-defined exit codes.
- Actionable errors with suggested next steps.
- Useful command-level `--help` with real examples.

## CLI/TUI Convention
For dual-mode commands:
- Recommended option: `--ui <auto|cli|tui>` (default: `auto`).
- Ergonomic alias: `--tui` (equivalent to `--ui tui`).
- `--ui` and `--tui` are mutually exclusive in the parser.

Rationale:
- `--ui` scales better for future UI modes (`web`, custom views, etc.).
- `--tui` preserves convenience and meets the explicit requirement.

## Agnostic Configuration Model (v0)
Suggested file: `belter.toml`

Configuration policy:
- Default format is `TOML`.
- Use `YAML` only when a specific integration/tooling path explicitly requires it.
- Tracked config files should keep environment placeholders (for example `${MEMPOOL_HOST}`) instead of host-specific values.

```toml
version = 1
environment = "home-lab"

[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"
workdir = "${BITCOIND_WORKDIR}"
tags = ["bitcoin", "core"]

[service.stratum]
manager = "launchd"
unit = "gui/501/io.btc.public-pool"
tags = ["mining", "stratum"]

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "docker"
tags = ["explorer"]

[[check]]
id = "core_tip"
type = "command"
cmd = "bitcoin-cli -datadir=${BITCOIND_DATADIR} getblockcount"
expect = "exit_code == 0"

[[check]]
id = "mempool_backend_info"
type = "http"
url = "http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info"
expect = "status == 200"
```

## Initial Command Tree
```text
belter [--config <PATH>] [--json] [--dry-run]
  config
    init [--path <PATH>] [--force]
    validate [--write-missing]
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
  run
    action <id>
  tui
    dashboard
```

## Candidate Feature Set (First Pass)
1. Service lifecycle control by logical name (`bitcoind`, `stratum`, `mempool`) from config.
2. Unified status view across multiple service managers (`launchd`/`systemd`/`podman compose`).
3. JSON health snapshot for reporting and alerting.
4. Guided troubleshooting for common failures (for example, `mempool` returning `502`).
5. Fast TUI operations for status/restart/logs.
6. Environment profiles (`home-lab`, `staging`, `prod`) with inheritance.

## Open Decisions
1. Secret handling strategy: environment variables vs secret backend.
2. Extensibility model: built-in adapters vs external plugin system.

## Dependency Model (Bring-Up v1)

`belter service bring-up <name>` is now implemented as a small local orchestrator on top of the primitive manager actions.

Design intent:
- keep `start|stop|restart` as direct manager primitives,
- add `bring-up` as a dependency-aware orchestration command,
- model local runtime dependencies explicitly instead of hardcoding them inside service-specific logic,
- stop short of becoming a long-running reconciler or service manager replacement.

Configuration shape:

```toml
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.podman_runtime]
manager = "podman_machine"
machine = "${PODMAN_MACHINE_NAME}"

[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"
depends_on = ["bitcoind"]

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
depends_on = ["bitcoind", "podman_runtime"]
```

Field meanings:
- `depends_on`: ordered logical dependencies that must be healthy before a service bring-up can continue.
- `manager = "podman_machine"`: runtime manager for Podman VM-backed availability on macOS.
- `machine`: Podman machine logical name, for example `podman-machine-default`.
- `env_file`: runtime env file to load for a `podman_compose` service before planning or execution.

Current v1 behavior:
- `service bring-up stratum`
  - resolves `depends_on = ["bitcoind"]`,
  - starts `bitcoind` only if not already healthy,
  - starts `stratum` only if needed.
- `service bring-up mempool`
  - resolves `depends_on = ["bitcoind", "podman_runtime"]`,
  - loads `env_file` for the `mempool` compose stack,
  - starts only missing or unhealthy dependencies in dependency order,
  - starts `mempool`,
  - waits for `HTTP 200` on `/api/v1/backend-info`.

Runtime semantics:
- dry-run plans the full declared chain and does not consult runtime state,
- real execution checks the current state of dependencies and skips healthy services,
- `mempool` readiness is stronger than compose state:
  - running containers plus successful HTTP probe are required for `running`,
  - running containers with failed readiness become `degraded`.

Operator-facing behavior:
- bring-up emits structured events for:
  - skipped healthy dependencies,
  - services being started,
  - readiness waiting,
  - readiness success.

Scope guardrails for this model:
- dependency resolution remains local and acyclic,
- only explicitly declared dependencies are traversed,
- no scheduler, reconciliation loop, or background controller is introduced,
- dependency health remains explicit and actionable per manager.
