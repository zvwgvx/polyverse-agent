---
title: Polyverse Wiki
summary: Public technical documentation for the current Polyverse repository and runtime.
order: 0
---

# Polyverse Wiki

Polyverse is a worker-based agent runtime built as a Rust workspace, with local web and HTTP surfaces for observability and tool access.

This wiki documents the repository as it exists today. It focuses on the current codebase, current commands, and current runtime behavior rather than older handoff or RFC-style notes.

## What this wiki covers

- repository layout and reading paths
- runtime architecture and event flow
- local development and runtime configuration
- configuration and testing reference

## Read this wiki by goal

- Start with [Overview](./overview/) if you are new to the repo.
- Read [Architecture](./architecture/) to understand the worker runtime, event flow, and memory layers.
- Use [Operations](./operations/) for local setup and runtime configuration.
- Use [Reference](./reference/) for configuration and testing lookups.

## Current repository surfaces

- `apps/agent` — main binary and composition root
- `apps/cockpit-web` — local Next.js cockpit UI
- `apps/wiki` — filesystem-driven wiki frontend for `docs/wiki`
- `services/cockpit-api` — local observability and prompt/state API
- `services/mcp` — local read-only MCP HTTP surface

## Scope note

The repository still contains older design and handoff notes under `docs/`. Some of those files are useful background material, but this wiki only documents behavior that is supported by the live repository layout, code, config, and tests.
