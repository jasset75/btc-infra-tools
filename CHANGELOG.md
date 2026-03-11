# Changelog

All notable changes to this project will be documented in this file.

The project follows semantic versioning.

## [Unreleased]

### Planned Process
- Add new features through the live-cycle: develop -> validate -> document -> release.
- Record each delivered feature in this changelog before release.

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

## [0.1.0] - 2026-03-10

### Added
- Initial Rust workspace scaffold (`infractl-core`, `infractl-adapters`, `belter` binary).
- Base CLI command tree and global `--json` output mode.
- `config init` generation for `belter.toml` with environment placeholders.
- Initial project documentation (`README`, architecture, command reference).
- Dual licensing setup (`MIT OR Apache-2.0`).
