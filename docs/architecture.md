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
belter
  config
    init
    validate
    show
  service
    list
    status [name] [--ui ...|--tui]
    start <name>
    stop <name>
    restart <name>
    logs <name> [--follow]
  health
    check [--all|--id <check-id>] [--json] [--ui ...|--tui]
    snapshot [--json]
  run
    action <id> [--dry-run]
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
