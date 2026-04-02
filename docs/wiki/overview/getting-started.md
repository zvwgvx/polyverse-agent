---
title: Getting Started
summary: Quick-start commands and first reading paths for working in the repository.
order: 10
---

# Getting Started

This page gives the shortest practical path to understanding and running the repository locally.

## 1. Know the main entrypoints

- `make agent` runs the main Rust agent.
- `make cockpit` starts the local cockpit web app.
- `make wiki` starts the local wiki app.
- `make test` runs the Rust test suite.

These commands come from the root `Makefile` and are the most useful defaults for day-to-day work.

## 2. Run the main runtime

```bash
make agent
```

Direct equivalent:

```bash
cargo run -p agent --bin polyverse-agent
```

The agent composition root lives in `apps/agent/src/main.rs`.

## 3. Run the local UIs

Cockpit web app:

```bash
make cockpit
```

Wiki app:

```bash
make wiki
```

The wiki app uses `next dev --hostname 0.0.0.0`, so it is reachable on the local network unless you override the default Next.js behavior.

## 4. Run the most common checks

Run the Rust tests:

```bash
make test
```

Check a single crate:

```bash
cargo check -p cognitive
```

Run MCP tests:

```bash
cargo test -p mcp
```

Typecheck or build the frontend apps directly when needed:

```bash
cd apps/cockpit-web && npm run typecheck
cd apps/cockpit-web && npm run build
npm --prefix apps/wiki run typecheck
npm --prefix apps/wiki run build
```

## 5. Know the local service defaults

By default:

- cockpit API binds to `127.0.0.1:4787`
- MCP binds to `127.0.0.1:4790` when enabled

Cockpit is enabled by default in the agent runtime if the state schema loads successfully. MCP is opt-in through `MCP_ENABLED`.

## 6. Read next

- Use [Repository Map](./repository-map.md) to understand where code lives.
- Use [Runtime Configuration](../operations/configuration/runtime-configuration.md) before changing environment or local runtime behavior.
- Use [Architecture](../architecture/core-runtime/) to follow the worker runtime in more detail.
