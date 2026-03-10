# Command Reference (WIP)

This document describes the current `belter` command surface from the scaffold implementation.

Status:
- WIP
- Several commands currently return scaffold responses and do not execute real infrastructure actions yet.

## Global Options
- `--json`: emit structured JSON output instead of human-readable text.

## Command Tree
```text
belter
  config
    init [--path <PATH>] [--force]
    validate
    show
  service
    list
    status [name] [--ui <auto|cli|tui> | --tui]
    start <name>
    stop <name>
    restart <name>
    logs <name> [--follow]
  health
    check [--all | --id <ID>] [--ui <auto|cli|tui> | --tui]
    snapshot
  run
    action <id> [--dry-run]
  tui
    dashboard
```

## `config`

### `config init`
Creates a `belter.toml` template.

Options:
- `--path <PATH>`: target path (default: `belter.toml`)
- `--force`: overwrite target file if it already exists

Behavior:
- Writes a template config that includes environment placeholders such as `${MEMPOOL_HOST}` and `${MEMPOOL_PORT}`.

### `config validate`
Scaffold command; currently returns a success message.

### `config show`
Scaffold command; currently returns a placeholder message.

## `service`

### `service list`
Scaffold command; currently returns a static list (`bitcoind`, `stratum`, `mempool`).

### `service status [name]`
Scaffold command; currently reports requested target and UI mode.

UI options:
- `--ui <auto|cli|tui>`
- `--tui` (shortcut for `--ui tui`, mutually exclusive with `--ui`)

### `service start <name>`
Scaffold command; currently echoes the requested target.

### `service stop <name>`
Scaffold command; currently echoes the requested target.

### `service restart <name>`
Scaffold command; currently echoes the requested target.

### `service logs <name>`
Scaffold command; currently echoes target and follow mode.

Options:
- `--follow`

## `health`

### `health check`
Scaffold command; currently echoes selection and UI mode.

Options:
- `--all`
- `--id <ID>`
- `--ui <auto|cli|tui>`
- `--tui` (mutually exclusive with `--ui`)

### `health snapshot`
Scaffold command; currently returns a placeholder snapshot message.

## `run`

### `run action <id>`
Scaffold command; currently echoes action id and dry-run mode.

Options:
- `--dry-run`

## `tui`

### `tui dashboard`
Scaffold command; currently returns a placeholder dashboard start message.
