# Changelog

All notable changes to this project will be documented in this file.

The project follows semantic versioning.

## [Unreleased]

This section includes implemented changes that are not released yet.

### Added
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

## [0.1.0] - 2026-03-10

### Added
- Initial Rust workspace scaffold (`infractl-core`, `infractl-adapters`, `belter` binary).
- Base CLI command tree and global `--json` output mode.
- `config init` generation for `belter.toml` with environment placeholders.
- Initial project documentation (`README`, architecture, command reference).
- Dual licensing setup (`MIT OR Apache-2.0`).
