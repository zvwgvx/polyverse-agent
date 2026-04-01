---
title: Overview
summary: High-level orientation for the repository, runtime, and main reading paths.
order: 1
---

# Overview

Polyverse is organized as a worker-based runtime: platform adapters ingest events, the runtime broadcasts them through a shared event system, cognitive and memory workers react to them, and local service surfaces expose observability and tool access.

This section is the best entry point if you want a quick mental model of the repository before diving into implementation details.

## At a glance

- `apps/agent` is the composition root that loads config, initializes storage, registers workers, and starts the runtime.
- `libs/` holds the reusable runtime, cognitive, memory, sensory, and state subsystems.
- `services/` exposes local HTTP surfaces for cockpit and MCP.
- `apps/cockpit-web` and `apps/wiki` are Next.js apps that sit alongside the Rust workspace.

## Recommended reading path

- Read [Getting Started](./getting-started.md) for the shortest path to running the repo locally.
- Read [Repository Map](./repository-map.md) for the top-level structure and package roles.
- Continue to [Architecture](../architecture/) once you want the runtime shape in more detail.
- Use [Operations](../operations/) and [Reference](../reference/) when you need exact commands or config lookups.
