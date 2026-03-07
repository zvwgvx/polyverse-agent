# Local Cockpit MVP

## Scope
- Single-agent local dashboard.
- Local API for overview, events, prompts, state list, state patch.
- 1-second refresh in UI.
- Memory inspection, relationship graph, and system resource monitoring.
- Minimal grayscale UI.

## Backend
- Worker: `cockpit_api` (`crates/pa-cockpit-api`).
- State schema/store: `crates/pa-state`.
- Default schema path: `config/state_schema.v0.json`.

## Runtime env
- `COCKPIT_ENABLED=true`
- `COCKPIT_BIND=127.0.0.1:4787`
- `COCKPIT_MAX_RECENT_EVENTS=300`
- `STATE_SCHEMA_PATH=config/state_schema.v0.json`
- `PA_AGENT_PROFILE=config/agent_profile.toml`
- `MEMORY_DB_PATH=data/agent_memory.db` (optional override)

## API
- `GET /api/cockpit/overview`
- `GET /api/cockpit/events?limit=80`
- `GET /api/cockpit/states`
- `GET /api/cockpit/states/history?limit=120`
- `POST /api/cockpit/state/patch`
- `GET /api/cockpit/memory?limit=24`
- `GET /api/cockpit/relationships`
- `GET /api/cockpit/system`
- `GET /api/cockpit/prompts`

## Frontend
- App path: `apps/cockpit-web`
- Start:
  - `cd apps/cockpit-web`
  - `npm install`
  - `npm run dev`

By default frontend proxies to `http://127.0.0.1:4787` through Next route `/api/cockpit/*`.
