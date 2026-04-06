---
title: Local Development
summary: Practical local workflow for running the agent, frontend apps, and test suites.
order: 30
---

# Local Development

This page summarizes the current local development workflow using the repository's checked-in commands and app scripts.

## Common root commands

The root `Makefile` defines the shortest commands for common tasks:

```bash
make agent
make wiki
make wiki-install
make test
make typecheck
```

## Run the main agent

```bash
make agent
```

Direct equivalent:

```bash
cargo run -p agent --bin polyverse-agent
```

The main binary lives in `apps/agent`.

## Run the wiki app

Install dependencies if needed:

```bash
make wiki-install
```

Start the app:

```bash
make wiki
```

Direct app-level commands:

```bash
npm --prefix apps/wiki run dev
npm --prefix apps/wiki run typecheck
npm --prefix apps/wiki run build
```

The wiki app reads content directly from `docs/wiki`.

## Run Rust tests

Run the workspace test suite:

```bash
make test
```

Useful focused commands:

```bash
cargo test -p kernel
cargo test -p mcp
cargo test -p kernel test_event_enum_variants -- --nocapture
cargo check -p cognitive
cargo build
cargo build --profile fast-release
```

## Typical local loop

A practical workflow for most repository changes looks like this:

1. run `make agent` if you are working on the runtime
2. run `make wiki` if you are editing `docs/wiki` or the wiki app
3. run focused package tests while iterating
4. finish with broader verification such as `make test` or app builds where relevant

## Notes

- Cargo is configured to use `scripts/protoc-wrapper.sh` for protobuf-related builds.
- Some local behavior is influenced by `settings.json`, even when `config.toml` is unchanged.
- MCP is not started by default unless `MCP_ENABLED` is set.
