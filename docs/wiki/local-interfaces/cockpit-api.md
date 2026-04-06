---
title: Cockpit API (Removed)
summary: Legacy cockpit interface removed from runtime.
order: 31
---

# Cockpit API (Removed)

The legacy `cockpit-api` service and `apps/cockpit` frontend were removed from the workspace.

## Current status

- `services/cockpit-api` no longer exists.
- `apps/cockpit` no longer exists.
- `apps/agent` no longer registers `CockpitWorker`.
- `COCKPIT_*` runtime environment variables are no longer used.

## Replacement direction

For runtime tool access, use the MCP server documented in [MCP Server](./mcp-server.md).

For architecture and configuration, use the pages under `docs/wiki/operations` and `docs/wiki/reference` as the current source of truth.

This page is intentionally kept only as a legacy marker during the redesign period.
