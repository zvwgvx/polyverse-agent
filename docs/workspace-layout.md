# Workspace Layout

## Status

This document defines the **target top-level folder architecture** for the repository.

It exists to keep `crates/` from becoming a catch-all bucket for every Rust package, test harness, and transport surface as the workspace grows.

## Target layout

```text
apps/
  agent/
  cockpit-web/

libs/
  kernel/
  runtime/
  cognitive/
  memory/
  state/
  sensory/

services/
  cockpit-api/
  mcp/

testing/
  test-support/
  integration-tests/

tools/
  xtask/
```

## Folder roles

### `apps/`
Runnable applications and user-facing entrypoints.

Current/target members:
- `agent`
- `cockpit-web`

Important: **cockpit belongs in `apps/`**. It is an application, not a library or service.

### `libs/`
Reusable internal libraries that hold the main system logic and shared runtime/domain code.

Expected members:
- `kernel`
- `runtime`
- `cognitive`
- `memory`
- `state`
- `sensory`

Why `libs/`:
- more readable than a Rust-specific `crates/` bucket
- more neutral than `core/`, which would overstate some packages
- avoids turning one directory into a mixed collection of apps, services, and test-only packages

### `services/`
Transport and API boundaries exposed as processes or local service surfaces.

Expected members:
- `cockpit-api`
- `mcp`

### `testing/`
Shared test infrastructure and cross-package/system test packages.

Expected members:
- `test-support`
- `integration-tests`

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
1. move directories by top-level role
2. rename crate/package names to match the role-based layout
3. update workspace `members` in the root `Cargo.toml`
4. update path references in docs and commands after each move
5. run focused cargo checks/tests after each rename step

Some older docs may still reference historical paths under `crates/`; treat those as pre-migration references and prefer the live `apps/`, `libs/`, and `services/` paths.

## Current mapping intent

```text
apps/
  agent                 # moved from crates/agent
  cockpit-web              # already under apps/

libs/
  kernel                  # moved from crates/kernel
  runtime               # moved from crates/runtime
  cognitive             # moved from crates/cognitive
  memory                # moved from crates/memory
  state                 # moved from crates/state
  sensory               # moved from crates/sensory

services/
  cockpit-api           # moved from crates/cockpit-api
  mcp                   # moved from crates/mcp

testing/
  test-support          # planned
  integration-tests     # planned

tools/
  xtask                    # future option
```
