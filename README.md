# btc-infra-tools

Monorepo for `belter`, a Rust CLI/TUI for infrastructure operations.

## Initial Architecture
- Design decisions and initial scope: [Architecture](docs/architecture.md)
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
  - Config-driven `service restart <name>` for `launchd`.
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
1. `cargo check -p belter`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-targets`

Setup:
```bash
mise install
lefthook install
```

## License
Licensed under either of:
- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
