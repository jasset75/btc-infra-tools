# btc-infra-tools

Monorepo for `belter`, a Rust CLI/TUI for infrastructure operations.

## Initial Architecture
- Design decisions and initial scope: [Architecture](docs/architecture.md)
- Current implemented architecture (with runtime flow): [Architecture (Current)](docs/architecture-btc-infra-tools.md)
- Feature and release history: [CHANGELOG](CHANGELOG.md)
- Release workflow: [docs/release-process.md](docs/release-process.md)
- Planned work and upcoming features: [ROADMAP](ROADMAP.md)

## Workspace Layout
- `crates/infractl-core`: config/output/time primitives
- `crates/infractl-adapters`: service manager abstraction (launchd/systemd/podman, etc.)
- `crates/infractl-cli`: `belter` binary (`clap`-based)

## Quick Start
- Build and run a first command: `cargo run -p belter -- service list`

## Command Reference

### Belter CLI
- Detailed command/flag reference: [docs/belter-command-reference.md](docs/belter-command-reference.md)
- Current features:
  - Config-driven `service start|stop|restart <name>` for `launchd`, `podman_compose`, and `podman_machine`.
  - `service bring-up <name>` as a small dependency-aware orchestrator.
  - `info pool [target]` for read-only public-pool metrics from local or remote hosts.
  - `${ENV_VAR}` expansion in service `unit`.
  - Deterministic config/env resolution: `--config`, `BELTER_CONFIG`, XDG config, then local project fallback.
  - Automatic `.env` loading from the selected config directory, with `BELTER_ENV_FILE` override support.
  - Per-service runtime env loading for `podman_compose` services via `env_file`.
  - HTTP-aware `mempool` status and readiness checks.
  - Actionable launchd restart errors for target format and privilege requirements.

## Operator Setup (macOS, repo-local mise)

Recommended host layout for node operations:

```text
~/work/btc-infra/
|- ops/        # private operational docs/scripts
`- upstream/
   `- btc-infra-tools/   # public upstream clone
```

Bootstrap (without global Rust install):

```bash
cd ~/work/btc-infra/upstream
git clone https://github.com/jasset75/btc-infra-tools.git
cd btc-infra-tools

mise trust
mise use rust@stable
mise install

mise exec -- cargo --version
mise exec -- rustc --version
mise exec -- just --version
```

Common day-to-day tasks:

```bash
mise exec -- just build
mise exec -- just install
mise exec -- just install-latest-stable
mise exec -- just check
mise exec -- just clippy
mise exec -- just clippy-fix
```

Recommended before first install:

```bash
cp .env.example .env
# edit .env with real operator values
```

Install `belter` binary for direct use (`belter <args>`):

> *Just once, after initial mise install:*
```bash
mise exec -- just install
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
belter --version
```

`just install` also prepares the standard config directory for operators:
- `${XDG_CONFIG_HOME}/belter` when `XDG_CONFIG_HOME` is set
- otherwise `~/.config/belter`
- creates repo-root `.env` from `.env.example` when `.env` is missing
- treats repo-root `.env` as the single source of truth
- overwrites `.env` in the standard config directory from repo-root `.env` on each install
- overwrites `belter.toml` from the repo root on each install

Tip: update from repo and rebuild installed binary:
> *Every time you want to update:*
```bash
git pull --ff-only origin main
mise exec -- cargo install --path crates/infractl-cli --locked --root ~/.local --force
belter --version
```

Tip: install the latest stable release from the newest tag:
> *For nodes that should stay on the latest stable release instead of `main`:*
```bash
mise exec -- just install-latest-stable
```

This recipe:
- fetches tags from `origin`,
- resolves the highest `v*` release tag available locally after fetch,
- checks out that release tag,
- reinstalls `belter`,
- verifies the installed version.

- `--force`: reinstalls even when Cargo would otherwise skip installation.
- `--locked`: uses the repository `Cargo.lock` for reproducible dependency versions.

Smoke test:

```bash
mise exec -- cargo run -p belter -- --help
mise exec -- cargo run -p belter -- service list
mise exec -- cargo run -p belter -- info pool
mise exec -- cargo run -p belter -- info pool 192.0.2.10
mise exec -- cargo run -p belter -- health snapshot --json
```

## Preconditions
- Current practical integration target is `bitcoind` managed by `launchd`.
- Environment-specific values must be provided through local `.env` and config files.

## Configuration Bootstrap
`config init` generates a tracked-safe `belter.toml` template with environment placeholders.

Runtime config resolution order:
- `--config <PATH>`
- `BELTER_CONFIG`
- `${XDG_CONFIG_HOME}/belter/belter.toml`
- `~/.config/belter/belter.toml`
- `./belter.toml` (compatibility fallback)

Environment loading order:
- `BELTER_ENV_FILE`
- `.env` next to the selected config file
- `./.env`

By default, HTTP checks can reference:
- `MEMPOOL_HOST`
- `MEMPOOL_PORT`
- `BITCOIND_LAUNCHD_UNIT` (for `service restart bitcoind`)

Example:
```bash
cp .env.example .env
# edit .env with real values
cargo run -p belter -- config init --force
```

Generated URL example:
- `http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info`

Practical `.env` example for a local `mempool` stack:

```bash
PODMAN_MACHINE_NAME=podman-machine-default
MEMPOOL_HOST=127.0.0.1
MEMPOOL_PORT=8080
MEMPOOL_COMPOSE_FILE=$HOME/mempool-local/ops/mempool/config/docker-compose.base.yml
MEMPOOL_COMPOSE_OVERRIDE=$HOME/mempool-local/ops/mempool/config/docker-compose.override.yml
MEMPOOL_PROJECT=docker
MEMPOOL_ENV_FILE=$HOME/mempool-local/ops/env/mempool.env
BITCOIND_LAUNCHD_UNIT=system/com.bitcoind.node
STRATUM_LAUNCHD_UNIT=gui/501/io.btc.public-pool
```

Placeholder notes for `.env.example`:
- `<podman_machine_name>`: Podman machine logical name on the host; current expected value is usually `podman-machine-default`.
- `<mempool_host>`: host where belter reaches the local mempool HTTP API, usually `127.0.0.1`.
- `<mempool_port>`: published mempool web/API port, usually `8080`.
- `<path_to_mempool_compose_file>`: absolute path to the base compose file copied from upstream.
- `<path_to_mempool_compose_override_file>`: absolute path to the local override compose file.
- `<podman_compose_project_name>`: compose project name passed as `podman compose -p ...`; current recommended value is `docker`.
- `<path_to_mempool_runtime_env_file>`: absolute path to the runtime env file used by the `mempool` compose stack, usually `$HOME/mempool-local/ops/env/mempool.env`.
- `<path_to_bitcoind_workdir>`: host working directory for the managed Bitcoin Core service, if used.
- `<path_to_bitcoind_datadir>`: host datadir passed to `bitcoin-cli`, if used.
- `<launchd_unit_for_bitcoind>`: full launchd target, for example `system/com.bitcoind.node`.
- `<launchd_unit_for_stratum>`: full launchd target for local `public-pool`, for example `gui/501/io.btc.public-pool`.

## Bring-Up Model

`belter service bring-up <name>` is the current small orchestrator layer on top of the primitive `start|stop|restart` commands.

Current behavior:
- resolves `depends_on` from `belter.toml` as an acyclic local dependency graph,
- loads any `env_file` declared by `podman_compose` services before planning or execution,
- in real execution, skips dependencies already healthy/running,
- in `--dry-run`, prints the full declared bring-up chain without consulting host runtime state,
- emits structured events in JSON and text output so the operator can see what was skipped, started, or waited on.

Current practical example:

```bash
belter service bring-up mempool
```

For `mempool`, `bring-up` currently:
- validates and starts `bitcoind` only if needed,
- validates and starts `podman_runtime` only if needed,
- starts `mempool` with its configured `env_file`,
- waits for `http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info` to return `200`,
- reports `running`, `degraded`, or `stopped` through `service status mempool`.

Operational note:
- `service status mempool` is stronger than a plain compose `ps`; it requires both running containers and successful HTTP readiness before returning `running`.

## Development Cycle
Feature delivery follows this loop:
1. Develop feature.
2. Validate feature.
3. Document feature.

Versioning policy:
- Each delivered feature should be recorded in `CHANGELOG.md`.
- Project version should be bumped according to semantic versioning as features are released.
- Releases should follow the documented [release process](docs/release-process.md), including roadmap validation, changelog finalization, semantic version alignment, quality gates, smoke validation, and git tagging.

Current note:
- `0.1.1` is the current release version.

## Git Hooks
This repository uses `lefthook` to run local quality gates before push.

Pre-push checks:
1. `just check`
2. `just clippy`
3. `just test`

Setup:
```bash
mise install
lefthook install
```

## License
Licensed under either of:
- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
