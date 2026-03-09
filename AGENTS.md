# AGENTS.md

## Purpose
This file defines agentic contribution rules for `btc-infra-tools`.
All contributors (human or AI agents) must follow these rules.

## Language Policy
- Use English only for:
  - documentation
  - source code
  - comments
  - commit messages
  - pull request titles and descriptions
  - issue descriptions
- If user input is not in English, keep responses concise but produce repository artifacts in English.

## Product Direction
- Current strategy: monorepo, single binary (`belter`).
- Architecture target:
  - `crates/infractl-core`
  - `crates/infractl-adapters`
  - `crates/infractl-cli`
- CLI follows `clig.dev` principles.
- Dual CLI/TUI commands must support:
  - `--ui <auto|cli|tui>` (canonical)
  - `--tui` (alias to `--ui tui`)

## Configuration Principles
- Tool behavior must be infrastructure-agnostic.
- Infrastructure specifics belong to external config, not hardcoded logic.
- Default config format: TOML (`belter.toml`) unless superseded by an explicit decision.
- Secrets must never be committed.

## Operational Safety
- Prefer non-destructive actions by default.
- Service-control commands must include clear target/service resolution before execution.
- Provide `--dry-run` where meaningful for operational actions.
- Error messages must include actionable next steps.

## Coding Standards
- Rust edition and lint configuration should be centralized at workspace level.
- New code should favor:
  - explicit types at API boundaries
  - small focused modules
  - structured errors (`thiserror`/`anyhow` policy to be decided in codebase)
- Keep comments concise and only where they add non-obvious context.

## Testing and Validation
- Every new command path should include at least one automated test:
  - unit test for parsing/logic
  - integration test for command behavior where feasible
- Before merging:
  - `cargo fmt --check`
  - `cargo clippy -- -D warnings`
  - `cargo test`

## Documentation Requirements
- Update docs in the same change set when behavior changes.
- Command docs must include:
  - purpose
  - inputs/options
  - examples
  - expected exit behavior

## Contribution Workflow for Agents
1. Restate the requested change and assumptions.
2. Make the smallest viable change.
3. Verify with relevant checks/tests.
4. Report facts: what changed, what was verified, what remains open.
5. Do not silently refactor unrelated areas.

## Change Control
- Breaking changes require explicit note in docs.
- Keep repository clean: no generated artifacts unless intentionally versioned.
- If uncertain about a destructive or high-impact action, stop and ask first.
