# Roadmap

This file tracks planned work only.
When a feature is implemented, move it to `CHANGELOG.md` under `Unreleased`.

## Release Strategy

Current versioning intent:
- use `0.1.x` releases to close the gap between prototype and operationally coherent CLI,
- reserve `1.0.0` for the checkpoint where the shipped command surface no longer contains scaffold behavior in the executable,
- do not require every currently imagined or planned feature for `1.0.0`,
- do require that the commands already exposed by the executable have real, defensible behavior.

## In Progress
- [ ] (none)

## 0.1.2

Goal:
- remove the most visible operator-facing scaffolds in the `service` command group.

Planned:
- [x] Replace static `service list` with config-driven discovery
- [ ] Implement real `service status` for all configured services (`service status` without `name`)
- [ ] Implement real `service logs <name>`
- [ ] Add integration tests for aggregated service status and logs

Why this release:
- this closes the biggest UX gap after `bring-up` by making the service control plane feel complete enough for daily use.

## 0.1.3

Goal:
- turn the `health` and config-readback surface from placeholders into usable operational commands.

Planned:
- [ ] Implement real `health check`
- [ ] Implement real `health snapshot`
- [ ] Implement real `config show`
- [ ] Design and implement a real `health pool` command with health-oriented semantics distinct from `info pool`

Why this release:
- after `service` matures, health/readback becomes the next missing pillar for routine operations and automation.

## 0.1.4+

Goal:
- address remaining executable scaffolds and low-risk productization gaps before calling the CLI stable.

Planned candidates:
- [ ] Implement real `run action <id>`
- [ ] Remove `tui dashboard` from the public CLI unless it gains a clearly defined operational purpose before `1.0.0`
- [ ] Extend `service bring-up` beyond `mempool` where real operational value exists
- [ ] Add more manager-aware logs/status integrations where needed

Notes:
- `tui dashboard` should not stay in the shipped CLI as a no-op placeholder.
- `run action <id>` should either become real or be removed/hidden before `1.0.0`.

## 1.0.0 Exit Criteria

The target for `1.0.0` is not “every planned idea implemented”.
The target is “the executable no longer exposes scaffold commands in its shipped surface”.

Required before `1.0.0`:
- [ ] `service list` is real
- [ ] `service status` without `name` is real
- [ ] `service logs <name>` is real
- [ ] `health check` is real
- [ ] `health snapshot` is real
- [ ] `config show` is real
- [ ] `run action <id>` is either real or intentionally removed from the public CLI
- [ ] `tui dashboard` is removed from the public CLI unless it has a real, clearly defined operational purpose
- [ ] no command documented in the public command tree returns scaffold/placeholder behavior
- [ ] `README`, command reference, and architecture docs no longer describe shipped commands as WIP where execution is already expected
- [ ] the current operator workflows (`bitcoind`, `stratum`, `mempool`, `podman_runtime`) are reproducible without undocumented manual steps

Explicitly out of scope for `1.0.0`:
- implementing every future idea in this roadmap
- supporting every possible manager/backend
- full TUI maturity, because the TUI itself is not part of the current `1.0.0` target
- broad plugin or extensibility work

## Future / Post-1.0 Candidates

- [ ] Add `stratum` logs with manager-aware ergonomics (`--follow` and non-follow)
- [ ] Add richer `stratum` health checks
- [ ] Add `info pool` extended metrics with per-miner/worker visibility
  - Scope: worker-level hashrate, accepted/rejected shares, connection time, best share, and best-share percentile estimate
  - Dependencies: define the observability/data source contract first (public-pool API extension or sidecar collector), plus persistence strategy for miner history across process restarts
- [ ] Add integration tests for `stratum` lifecycle/status/logs/errors
- [ ] Document `stratum` setup and operations in more depth
- [ ] Add `Bitcoin Peer Tier List (Score)` feature (tiered peer scoring report + disconnect candidate suggestions + low-impact node sampling)

## Exploring

- [ ] TUI dashboard scope and interaction model (operational MVP)
