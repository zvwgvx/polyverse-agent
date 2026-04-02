# Cockpit Web

Minimal local dashboard UI for a single Polyverse agent.

## Run
1. Ensure agent is running with cockpit API enabled (`COCKPIT_ENABLED=true`).
2. In this folder:
   - `npm install`
   - `npm run dev`
3. Open `http://localhost:3000`.

## API source
Frontend proxies all calls through `/api/cockpit/*` to:
- `COCKPIT_API_BASE` env var (default `http://127.0.0.1:4787`).
