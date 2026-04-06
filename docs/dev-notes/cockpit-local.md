# Local Cockpit MVP (Deprecated)

This note is kept only as historical context.

The original local cockpit system has been removed from the current workspace:

- `services/cockpit-api` deleted
- `apps/cockpit` deleted
- `CockpitWorker` removed from runtime wiring
- `COCKPIT_*` env toggles removed from active runtime config

For current local interfaces, use MCP (`services/mcp`) and the wiki docs under `docs/wiki/`.
