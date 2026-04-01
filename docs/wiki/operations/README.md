---
title: Operations
summary: Local development, runtime setup, and day-to-day operation of the current repository.
order: 3
---

# Operations

This section covers how to run the repository locally and how the runtime is configured in practice.

Use these pages when you need to move from reading the code to operating the system.

## What operations means here

For this repository, operations mostly means local runtime workflow:

- starting the main agent
- running the cockpit and wiki apps
- enabling or disabling local service surfaces such as MCP
- understanding how configuration is layered and resolved

## Current operational surfaces

- `apps/agent` — starts the worker runtime
- `services/cockpit-api` — local observability API exposed through the runtime
- `services/mcp` — local read-only MCP HTTP surface when enabled
- `apps/cockpit-web` — Next.js frontend for cockpit
- `apps/wiki` — Next.js frontend for this wiki

## Read next

- [Local Development](./local-development.md) for the daily run/test loop
- [Runtime Configuration](./runtime-configuration.md) for config layering, env vars, and default binds
