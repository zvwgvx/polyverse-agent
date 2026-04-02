---
title: Testing
summary: Current test layers, important test paths, and common verification commands.
order: 41
---

# Testing

Testing in this repository is layered. Some coverage lives next to the package being tested, and broader runtime coverage lives under `testing/`.

## Common commands

Run the Rust workspace tests:

```bash
make test
```

Run a specific package test suite:

```bash
cargo test -p kernel
cargo test -p mcp
```

Run a single Rust test:

```bash
cargo test -p kernel test_event_enum_variants -- --nocapture
```

Check a single crate without running tests:

```bash
cargo check -p cognitive
```

Frontend verification:

```bash
cd apps/cockpit && npm run typecheck
cd apps/cockpit && npm run build
npm --prefix apps/wiki run typecheck
npm --prefix apps/wiki run build
```

## Test layers

### Package-local tests

Rust packages can keep unit tests in `src/` and package-level integration tests in `<package>/tests/`.

Examples in the current repo:

- `services/mcp/tests/http_contract.rs`
- `services/mcp/tests/tools.rs`
- `services/mcp/tests/server.rs`

These lock down the MCP HTTP contract, tool listing behavior, validation, and error shapes.

### Shared test support

`testing/test-support` holds helpers and fixtures used across runtime and service tests.

### Cross-package integration tests

`testing/integration-tests` is for runtime-level and multi-package behavior.

Important current examples:

- `testing/integration-tests/tests/runtime_wiring.rs`
- `testing/integration-tests/tests/runtime_supervisor_mcp.rs`
- `testing/integration-tests/tests/social_mcp_roundtrip.rs`
- `testing/integration-tests/tests/dialogue_worker_smoke.rs`

These cover topics such as:

- coordinator event broadcasting
- supervisor lifecycle behavior
- live MCP worker startup and HTTP handling
- social MCP roundtrips against tree-backed responses
- dialogue worker startup, streaming, and tool-loop behavior

## How to choose the right level

- Use package-local tests when changing one package in isolation.
- Use `services/mcp/tests/*` when changing MCP transport or tool-call contracts.
- Use `testing/integration-tests/*` when a change crosses package boundaries or affects runtime wiring.
- Use frontend typecheck/build commands when changing either `apps/cockpit` or `apps/wiki`.

## Notes

- The Cargo workspace includes the Rust packages under `libs/`, `services/`, `apps/agent`, and `testing/`.
- The Next.js apps are verified through their own `npm` scripts rather than through Cargo.
- If you are documenting or changing behavior, tests under `testing/` and `services/mcp/tests/` are strong evidence of what the runtime currently supports.
