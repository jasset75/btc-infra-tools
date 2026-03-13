# Belter Command Reference

Detailed command reference for the `belter` CLI.

Status:
- WIP
- Some commands still return scaffold responses.

## Global Flags
- `--config <PATH>` (optional): config file path. Default is `belter.toml`.
- `--json` (optional): output as JSON envelope.
- `--dry-run` (optional): simulate command without making actual changes. This is specially useful for testing commands on machines that are not the actual infrastructure target (e.g. your local development machine).

### JSON Envelope
When `--json` is set, commands emit a single structured JSON object on `stdout`.

Top-level fields:
- `ts`: RFC3339 timestamp for the envelope.
- `command`: stable command identifier, for example `service.restart`.
- `status`: command outcome, currently `ok` or `error`.
- `message`: short canonical summary of the result.
- `dry_run`: whether the command was simulated.
- `data`: command payload.
- `events`: structured auxiliary events; safe to ignore for consumers that only need the main result.

Event fields:
- `ts`: RFC3339 timestamp for the event.
- `level`: event severity, for example `debug`, `info`, `warning`, `error`, `fatal`.
- `code`: stable machine-friendly event code.
- `message`: human-readable event message.
- `details`: structured event payload.

Example:
```json
{
  "ts": "2026-03-13T13:05:00.376301Z",
  "command": "service.restart",
  "status": "ok",
  "message": "would restart service `bitcoind`",
  "dry_run": true,
  "data": {
    "plan": {
      "operations": [
        {
          "RestartService": {
            "manager": "launchd",
            "unit": "system/com.bitcoind.node"
          }
        }
      ]
    }
  },
  "events": [
    {
      "ts": "2026-03-13T13:05:00.376250Z",
      "level": "info",
      "code": "service.restart.preview",
      "message": "1. Would restart `launchd` unit `system/com.bitcoind.node`",
      "details": {
        "operation_index": 1,
        "manager": "launchd",
        "unit": "system/com.bitcoind.node"
      }
    }
  ]
}
```

### Example Usage
```bash
# Safe to run locally, even if the target infrastructure (e.g. bitcoind) is not present
belter --dry-run service restart bitcoind
```

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

## config

### `config init`
- Parameters:
  - `--path <PATH>` (optional)
  - `--force` (optional)
- Behavior:
  - Creates a config template (default output path: `belter.toml`).

### `config validate`
- Parameters: none
- Behavior: scaffold success response.

### `config show`
- Parameters: none
- Behavior: scaffold placeholder response.

## service

### `service list`
- Parameters: none
- Behavior: scaffold static list.

### `service status [name]`
- Parameters:
  - `name` (optional, default: all)
  - `--ui <auto|cli|tui>` (optional)
  - `--tui` (optional; shortcut for `--ui tui`; mutually exclusive with `--ui`)
- UI behavior differences:
  - Current implementation: no output behavior change yet; mode is reported in output.

### `service start <name>`
- Parameters:
  - `name` (required)
- Behavior: scaffold echo response.

### `service stop <name>`
- Parameters:
  - `name` (required)
- Behavior: scaffold echo response.

### `service restart <name>`
- Parameters:
  - `name` (required)
- Behavior:
  - Loads `service.<name>` from config.
  - Requires `manager = "launchd"` and `unit`.
  - Expands `${ENV_VAR}` placeholders from environment.
  - Runs `launchctl kickstart -k <unit>`.
  - If `.env` exists in current directory, it is autoloaded before command execution.
- Operational notes:
  - For launchd units in `system/...`, restart may require elevation (`sudo -E`).
  - Unit must be full launchd target (`<domain>/<label>`, for example `system/com.bitcoind.node`).

### `service logs <name>`
- Parameters:
  - `name` (required)
  - `--follow` (optional)
- Behavior: scaffold echo response.

## health

### `health check`
- Parameters:
  - `--all` (optional; mutually exclusive with `--id`)
  - `--id <ID>` (optional; mutually exclusive with `--all`)
  - `--ui <auto|cli|tui>` (optional)
  - `--tui` (optional; shortcut for `--ui tui`; mutually exclusive with `--ui`)
- UI behavior differences:
  - Current implementation: no output behavior change yet; mode is reported in output.

### `health snapshot`
- Parameters: none
- Behavior: scaffold snapshot response.

## run

### `run action <id>`
- Parameters:
  - `id` (required)
  - `--dry-run` (optional)
- Behavior: scaffold echo response.

## tui

### `tui dashboard`
- Parameters: none
- Behavior: scaffold placeholder response.
