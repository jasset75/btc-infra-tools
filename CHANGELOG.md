# Changelog

All notable changes to this project will be documented in this file.

The project follows semantic versioning.

## [Unreleased]

This section includes implemented changes that are not released yet.

### Added
- Implemented real `service list` with config-driven discovery from `belter.toml`.
- Implemented real aggregated `service status` for all configured services when no service name is provided.
- Added `just install-latest-stable` to install `belter` from the newest tagged release instead of the current branch tip.
- Added deterministic config/env discovery for `belter` across Linux/macOS:
  - config resolution now follows `--config`, `BELTER_CONFIG`, XDG config path, then local `belter.toml`
  - `.env` loading now follows `BELTER_ENV_FILE`, config-local `.env`, then local `./.env`
  - `config init` now defaults to the standard XDG config location when available
- Added `belter-watchdog`, a command-based watchdog binary for periodically diagnosing configured services and running recovery commands when health checks fail:
  - supports `watchdog.toml` with per-watch intervals, confirmation delays, cooldowns, command timeouts, shell configuration, JSON field checks, and expected exit codes,
  - supports `belter-watchdog init` for generating a starter config,
  - supports `belter-watchdog run --once` for one-shot validation before installing a supervisor,
  - supports `[logging]` paths in `watchdog.toml`, creates log directories automatically, and writes normal events and error events to separate files when configured,
  - supports `belter-watchdog stats` with per-watch outage/recovery metrics, log window timestamps, and `--json` output,
  - supports `belter-watchdog clear-log` for truncating the selected watchdog event log.

### Changed
- Updated `service list` output to report the configured services, their managers, and declared dependencies in stable sorted order for both text and `--json`.
- Updated installation docs to distinguish installing from `main` versus installing the latest stable tagged release.
- Hardened `just install-latest-stable` so it resolves the highest fetched `v*` tag instead of relying on `git describe` from the current checkout.
- Updated `just install` and `just install-latest-stable` to prepare `${XDG_CONFIG_HOME}/belter` (or `~/.config/belter`) and refresh config on each install:
  - seed repo-root `.env` from `.env.example` when missing
  - treat repo-root `.env` as the source of truth for installed config
  - overwrite repo-root `.env` and `belter.toml` into the standard config directory on each install
- Updated `just install` and `just install-latest-stable` to install both `belter` and `belter-watchdog`.
- Hardened Podman-backed service recovery and diagnostics:
  - `service status podman_runtime` now requires both a running Podman machine and a responsive machine-specific API connection; a running VM with an unavailable API reports `degraded`,
  - `service start|restart podman_runtime` now polls API readiness before succeeding and retries the machine start once if the VM returns to `stopped`,
  - `service bring-up` now restarts a degraded `podman_machine` dependency before starting dependent services,
  - `service status mempool` now reports the real host HTTP status-line failure instead of a generic read error for keep-alive responses,
  - `service bring-up mempool` now uses `restart` for initial `degraded` state and retries `unknown` readiness failures with `restart`,
  - `service status podman_runtime` can now degrade when a dependent compose service remains up but unhealthy,
  - `service bring-up mempool` can now recognize the known `podman_runtime` forwarding failure pattern and recover by restarting `podman_runtime` before reapplying `mempool`.
- Improved `belter-watchdog` recovery accounting for services with delayed
  readiness:
  - added configurable recovery stabilization polling after a recovery command,
  - added `transitional_json_values` so states such as Mempool `syncing` do not
    trigger repeated restarts,
  - added explicit `recovery.outcome` events so stats distinguish recovered,
    stabilizing, and unrecovered recoveries.

## [0.1.1] - 2026-03-31

### Added
- Added support for `manager = "podman_machine"`:
  - `service start|stop|restart|status podman_runtime`
  - `machine = "${PODMAN_MACHINE_NAME}"` in config.
- Added `env_file` support for `podman_compose` services so runtime env files such as `mempool.env` can be loaded by `belter` before plan execution.
- Implemented `service bring-up <name>` as a small dependency-aware orchestrator:
  - resolves `depends_on` in dependency order,
  - plans the full chain in `--dry-run`,
  - skips healthy dependencies in real execution,
  - emits structured bring-up events.
- Implemented `service bring-up mempool`:
  - brings up `bitcoind` and `podman_runtime` only when needed,
  - loads `MEMPOOL_ENV_FILE`,
  - waits for `HTTP 200` on `/api/v1/backend-info` before reporting success.
- Added real `config validate` implementation:
  - validates that required services (`bitcoind`, `stratum`, `mempool`) exist in config,
  - validates manager-specific required fields (`unit` for `launchd`, `compose_file` for `podman_compose`),
  - validates `${ENV_VAR}` placeholder resolution using current environment (including auto-loaded `.env`),
  - returns actionable errors with missing config examples and missing env variable names.
- Added `belter config validate --write-missing` to append missing required service blocks to `belter.toml` before validation.
- Implemented `service restart <name>` for services configured with `manager = "launchd"`.
- Added config-driven restart flow:
  - load `service.<name>` from `belter.toml`
  - require `unit`
  - expand `${ENV_VAR}` placeholders in `unit`
  - run `launchctl kickstart -k <unit>`
- Updated scaffold template to include `service.bitcoind` with `unit = "${BITCOIND_LAUNCHD_UNIT}"`.
- Added `lefthook` pre-push configuration to enforce local `check`, `clippy`, and `test` gates.
- Added `.mise.toml` with `lefthook` tool pin so hook tooling is installable in remote/reproducible environments.
- Added automatic `.env` loading at CLI startup when `.env` exists in the current working directory.
- Added support for `manager = "podman_compose"` service lifecycle actions (`start`, `stop`, `restart`) with optional `compose_override` and `project`.
- Improved launchd restart error UX with actionable guidance for:
  - invalid target format (requires `<domain>/<label>`, for example `system/com.bitcoind.node`)
  - insufficient privileges for `system/...` units (use elevated execution)
- Added structured JSON error envelope output for CLI failures and explicit non-zero process exit code handling.
- Added `just install` smoke check (`belter --version`) to fail fast if the installed binary is not executable in the current environment.
- Added `just` as the project task runner with recipes for `build`, `install`, `check`, `clippy`, `clippy-fix`, and `test`.
- Added real `service status <name>` support for `manager = "podman_compose"`:
  - resolves compose placeholders from env
  - queries runtime status via `podman compose ... ps -q`
  - reports `data.state`, `data.running_containers`, and `data.query_error` (when applicable)
- Added stronger `service status mempool` semantics:
  - `running` requires real running containers plus `HTTP 200`,
  - `degraded` reports running containers with failed readiness,
  - `health_url` is included in JSON output.
- Added status JSON coverage for podman services (env-present and env-missing paths).
- Added CLI architecture class diagrams in `docs/architecture-btc-infra-tools.md` to document module responsibilities and `service status` computation/render flow.

### Changed
- Refactored CLI dotenv bootstrap to dependency injection (`DotenvLoader`) so tests can run without mutating process environment.
- Refactored `crates/infractl-cli` into focused modules:
  - `src/cli.rs` (command model + labels)
  - `src/runtime.rs` (runtime deps + dotenv loader)
  - `src/output.rs` (envelope/text emission helpers)
  - `src/commands/config.rs` and `src/commands/service.rs` (command handlers)
- Updated `lefthook` pre-push test command to use an isolated cargo target directory (`CARGO_TARGET_DIR=target/lefthook-prepush`) to reduce build lock contention.
- Updated default local example configuration and docs to include mempool placeholders (`MEMPOOL_COMPOSE_FILE`, `MEMPOOL_COMPOSE_OVERRIDE`, `MEMPOOL_PROJECT`) and a practical `.env` sample.
- Updated dry-run output model:
  - `--dry-run --json` for service plan commands no longer emits redundant preview events.
  - textual dry-run output now renders a JSON-shaped report block aligned with envelope fields.
- Updated `service status <name>` behavior:
  - launchd-backed services now query real runtime status and report it in `data` (`state`, `pid`, `unit`).
  - podman-compose-backed services now query real runtime status and report it in `data` (`state`, `running_containers`, `compose_file`, optional `compose_override`, optional `project`, `query_error`).
  - `--dry-run` now returns simulated status payloads (`dry_run: true`, `data.simulated: true`) instead of reporting non-dry-run envelopes.
- Refactored service status output to a typed status model (`ServiceStatusData`) and split status computation from rendering.
- Reworked CLI integration tests into focused suites:
  - `cli_smoke_test.rs`
  - `cli_service_status_test.rs`
  - `cli_service_plan_test.rs`
  - `cli_error_test.rs`
- Updated integration test helpers to avoid machine-local absolute paths by resolving workspace root from `CARGO_MANIFEST_DIR`.
- Updated CLI integration and unit tests to use isolated fixture directories instead of the repo `.env`, preventing host-environment contamination and parallel fixture collisions.

## [0.1.0] - 2026-03-10

### Added
- Initial Rust workspace scaffold (`infractl-core`, `infractl-adapters`, `belter` binary).
- Base CLI command tree and global `--json` output mode.
- `config init` generation for `belter.toml` with environment placeholders.
- Initial project documentation (`README`, architecture, command reference).
- Dual licensing setup (`MIT OR Apache-2.0`).
