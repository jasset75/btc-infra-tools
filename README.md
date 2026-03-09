# btc-infra-tools

Monorepo for `belter`, a Rust CLI/TUI for infrastructure operations.

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

## License
Licensed under either of:
- MIT license ([LICENSE-MIT](/Users/juan/work/btc-infra-tools/LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](/Users/juan/work/btc-infra-tools/LICENSE-APACHE))
