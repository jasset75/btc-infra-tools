# Roadmap

This file tracks planned work only.
When a feature is implemented, move it to `CHANGELOG.md` under `Unreleased`.

## In Progress
- [ ] Implement real `service logs <name>`

## Planned (Next Release)
- [ ] Implement real `config validate`
- [ ] Implement real `config show`
- [ ] Replace static `service list` with config-driven discovery
- [ ] Implement `service status` for all services (`service status` without `name`)
- [ ] Implement real `health check`
- [ ] Implement real `health snapshot`
- [ ] Implement real `run action <id>`
- [ ] Implement functional `tui dashboard`

## Planned (Future)
- [ ] Add `stratum` (public pool) service contract in config
- [ ] Add `stratum` defaults/placeholders to `config init`
- [ ] Add `service start|stop|restart stratum`
- [ ] Add `service status stratum`
- [ ] Add `service logs stratum` (`--follow` and non-follow)
- [ ] Add `stratum` health checks
- [ ] Add integration tests for `stratum` lifecycle/status/logs/errors
- [ ] Document `stratum` setup and operations
- [ ] Add `Bitcoin Peer Tier List (Score)` feature (tiered peer scoring report + disconnect candidate suggestions + low-impact node sampling)

## Exploring
- [ ] TUI dashboard scope and interaction model (operational MVP)
