# btc-infra-tools

Monorepo for `belter`, a Rust CLI/TUI for infrastructure operations.

## Initial Architecture
- Design decisions and initial scope: [Architecture](docs/architecture.md)
- Current implemented architecture (with runtime flow): [Architecture (Current)](docs/architecture-btc-infra-tools.md)
- Feature and release history: [CHANGELOG](CHANGELOG.md)

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
  - Config-driven `service start|stop|restart <name>` for `launchd` and `podman_compose`.
  - `${ENV_VAR}` expansion in service `unit`.
  - Automatic `.env` loading from current working directory.
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
mise exec -- just check
mise exec -- just clippy
mise exec -- just clippy-fix
```

Install `belter` binary for direct use (`belter <args>`):

> *Just once, after initial mise install:*
```bash
mise exec -- cargo install --path crates/infractl-cli --locked --root ~/.local
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
belter --version
```

Tip: update from repo and rebuild installed binary:
> *Every time you want to update:*
```bash
git pull --ff-only origin main
mise exec -- cargo install --path crates/infractl-cli --locked --root ~/.local --force
belter --version
```

- `--force`: reinstalls even when Cargo would otherwise skip installation.
- `--locked`: uses the repository `Cargo.lock` for reproducible dependency versions.

Smoke test:

```bash
mise exec -- cargo run -p belter -- --help
mise exec -- cargo run -p belter -- service list
mise exec -- cargo run -p belter -- health snapshot --json
```

## Preconditions
- Current practical integration target is `bitcoind` managed by `launchd`.
- Environment-specific values must be provided through local `.env` and config files.

## Configuration Bootstrap
`config init` generates a tracked-safe `belter.toml` template with environment placeholders.

By default, HTTP checks can reference:
- `MEMPOOL_HOST`
- `MEMPOOL_PORT`
- `BITCOIND_LAUNCHD_UNIT` (for `service restart bitcoind`)

Example:
```bash
cp .env.example .env
cargo run -p belter -- config init --force
```

Generated URL example:
- `http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info`

Practical `.env` example for a local `mempool` stack:

```bash
MEMPOOL_HOST=127.0.0.1
MEMPOOL_PORT=8080
MEMPOOL_COMPOSE_FILE=$HOME/mempool-local/ops/mempool/config/docker-compose.base.yml
MEMPOOL_COMPOSE_OVERRIDE=$HOME/mempool-local/ops/mempool/config/docker-compose.override.yml
MEMPOOL_PROJECT=docker
```

Placeholder notes for `.env.example`:
- `<mempool_host>`: host where belter reaches the local mempool HTTP API, usually `127.0.0.1`.
- `<mempool_port>`: published mempool web/API port, usually `8080`.
- `<path_to_mempool_compose_file>`: absolute path to the base compose file copied from upstream.
- `<path_to_mempool_compose_override_file>`: absolute path to the local override compose file.
- `<podman_compose_project_name>`: compose project name passed as `podman compose -p ...`; current recommended value is `docker`.
- `<path_to_bitcoind_workdir>`: host working directory for the managed Bitcoin Core service, if used.
- `<path_to_bitcoind_datadir>`: host datadir passed to `bitcoin-cli`, if used.
- `<launchd_unit_for_bitcoind>`: full launchd target, for example `system/com.bitcoind.node`.

## Development Cycle
Feature delivery follows this loop:
1. Develop feature.
2. Validate feature.
3. Document feature.

Versioning policy:
- Each delivered feature should be recorded in `CHANGELOG.md`.
- Project version should be bumped according to semantic versioning as features are released.

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
