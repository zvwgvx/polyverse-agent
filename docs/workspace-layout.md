# Workspace Layout

## Status

This document defines the **target top-level folder architecture** for the repository.

It exists to keep `crates/` from becoming a catch-all bucket for every Rust package, test harness, and transport surface as the workspace grows.

## Target layout

```text
apps/
  pa-agent/
  cockpit-web/

libs/
  pa-core/
  pa-runtime/
  pa-cognitive/
  pa-memory/
  pa-state/
  pa-sensory/

services/
  pa-cockpit-api/
  pa-mcp/

testing/
  pa-test-support/
  pa-integration-tests/

tools/
  xtask/
```

## Folder roles

### `apps/`
Runnable applications and user-facing entrypoints.

Current/target members:
- `pa-agent`
- `cockpit-web`

Important: **cockpit belongs in `apps/`**. It is an application, not a library or service.

### `libs/`
Reusable internal libraries that hold the main system logic and shared runtime/domain code.

Expected members:
- `pa-core`
- `pa-runtime`
- `pa-cognitive`
- `pa-memory`
- `pa-state`
- `pa-sensory`

Why `libs/`:
- more readable than a Rust-specific `crates/` bucket
- more neutral than `core/`, which would overstate some packages
- avoids turning one directory into a mixed collection of apps, services, and test-only packages

### `services/`
Transport and API boundaries exposed as processes or local service surfaces.

Expected members:
- `pa-cockpit-api`
- `pa-mcp`

### `testing/`
Shared test infrastructure and cross-package/system test packages.

Expected members:
- `pa-test-support`
- `pa-integration-tests`

Rule of thumb:
- package-local tests stay next to their package (`src` unit tests or `<package>/tests/`)
- cross-package harnesses and shared fixtures go under `testing/`

### `tools/`
Developer tooling, automation, codegen, and maintenance utilities.

Expected members:
- `xtask`
- future repo maintenance or code generation tools

## Classification rules

When adding a new workspace member, classify it by role first:

1. **Executable product or UI** -> `apps/`
2. **Reusable internal library** -> `libs/`
3. **API/MCP/server boundary** -> `services/`
4. **Shared test harness or system test package** -> `testing/`
5. **Developer automation/tooling** -> `tools/`

Do **not** use `libs/` as a generic fallback for everything written in Rust.

## Migration guidance

This layout is the target structure. Migration can happen incrementally.

Recommended order:
1. keep existing crate/package names (`pa-*`) stable
2. move directories by top-level role
3. update workspace `members` in the root `Cargo.toml`
4. update path references in docs and commands after each move

Until migration is complete, older docs may still reference historical paths under `crates/`.

## Current mapping intent

```text
apps/
  pa-agent                 # target move from crates/pa-agent
  cockpit-web              # already under apps/

libs/
  pa-core                  # target move from crates/pa-core
  pa-runtime               # target move from crates/pa-runtime
  pa-cognitive             # target move from crates/pa-cognitive
  pa-memory                # target move from crates/pa-memory
  pa-state                 # target move from crates/pa-state
  pa-sensory               # target move from crates/pa-sensory

services/
  pa-cockpit-api           # target move from crates/pa-cockpit-api
  pa-mcp                   # target move from crates/pa-mcp

testing/
  pa-test-support          # planned
  pa-integration-tests     # planned

tools/
  xtask                    # future option
```
