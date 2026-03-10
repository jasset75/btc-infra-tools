# btc-infra-tools

Monorepo for `belter`, a Rust CLI/TUI for infrastructure operations.

## Initial Architecture
- Design decisions and initial scope: [Architecture](docs/architecture.md)
- Current command surface (WIP): [Command Reference](docs/command-reference.md)
- Feature and release history: [CHANGELOG](CHANGELOG.md)

## Workspace Layout
- `crates/infractl-core`: config/output/time primitives
- `crates/infractl-adapters`: service manager abstraction (launchd/systemd/podman, etc.)
- `crates/infractl-cli`: `belter` binary (`clap`-based)

## Quick Start
```bash
cargo run -p belter -- service list
cargo run -p belter -- service status bitcoind --ui tui
cargo run -p belter -- health snapshot --json
```

## Preconditions
- Current practical integration target is `bitcoind` managed by `launchd`.
- Environment-specific values must be provided through local `.env` and config files.

## Configuration Bootstrap
`config init` generates a tracked-safe `belter.toml` template with environment placeholders.

By default, HTTP checks can reference:
- `MEMPOOL_HOST`
- `MEMPOOL_PORT`

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

## License
Licensed under either of:
- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
