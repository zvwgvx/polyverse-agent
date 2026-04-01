# Social Tree Query Model + MCP Context Server Architecture (Detailed)

**Version:** v1.1
**Date:** 2026-03-31
**Status:** Phase 1 implemented and runtime-smoked; Phase 2 dialogue tool loop implemented; Phase 3 planned
**Audience:** pa-cognitive, pa-memory, pa-cockpit-api, pa-mcp, future MCP runtime team

**Workspace note:** The target repo layout is `apps/`, `libs/`, `services/`, `testing/`, and `tools/`. Historical references in this document still use current in-repo paths under `crates/`. See `docs/workspace-layout.md`.

---

## 0) Executive summary

This document defines the architecture and current delivery status for:

1. A **Tree Query Model** for social-context retrieval.
2. A **read-only v1 MCP Context Server** for tool-style social-context access.

Core principle:

- **Graph = write/source of truth**
- **Tree = read/query model**

The goal is not to replace Graph. The goal is to standardize a query layer so that:

- Affect always receives rich and stable social context.
- Dialogue receives social context only when needed (gate-based).
- The runtime is ready for scalable MCP/tool retrieval.

### Current delivery snapshot (2026-03-31)

- **Done:** Phase 1 query contract + read-only MCP transport + tests + live local runtime smoke validation.
- **Done:** Phase 2 dialogue now has a bounded, feature-flagged model-driven read-tool loop with candidate-user allowlisting and legacy direct-summary fallback.
- **Not done yet:** action tools + approval/policy gates, broader parity automation, and richer MCP/dialogue metrics coverage.
- **Main risk now:** operational observability is still mostly log-first; metrics/SLO instrumentation should be expanded before broader rollout.

---

## 0.1 What we have implemented

### Query contract stabilization (done)

In `crates/pa-cognitive/src/social_context.rs`, we now have:

- `SocialQueryIntent`
- `SocialQueryOptions`
- `SocialQueryResult`
- `SocialQueryMeta`
- `query_social_context(...)` as normalized facade
- freshness/staleness behavior via `meta.updated_at` + policy controls
- standardized fallback sources (`tree_fresh`, `tree_stale`, `graph_fallback`, `default_fallback`)

### MCP read-only foundation (done)

In `crates/pa-mcp`:

- Worker + server transport with graceful lifecycle
- Endpoints:
  - `GET /api/mcp/tools`
  - `POST /api/mcp/tools/call`
- Registry with read-only tool descriptors (`namespace`, `read_only`)
- Tools:
  - `social.get_affect_context`
  - `social.get_dialogue_summary`
- Input validation + normalized response `meta` passthrough

### Runtime wiring (done)

In `crates/pa-agent/src/main.rs`:

- MCP worker is registered when `MCP_ENABLED=1`
- env-driven MCP config:
  - `MCP_BIND`
  - `MCP_REQUEST_TIMEOUT_MS`
  - `MCP_MAX_TOOL_CALLS_PER_TURN`

### Test coverage (done for Phase 1 baseline)

In `crates/pa-mcp/tests`:

- endpoint contract tests (`tools`, `tools/call` success + error)
- input validation tests (empty `user_id`)
- deterministic unknown-tool behavior test
- response meta presence tests for both tools

---

## 0.2 What is not implemented yet

1. Action namespace execution path (currently intentionally blocked by read-only scope).
2. Policy/approval/audit gate interfaces for mutating tools.
3. Full parity framework automation/alerts (Graph↔Tree drift monitoring beyond baseline logs/tests).
4. Broader operational metrics/SLO coverage for MCP and dialogue tool-call paths.

---

## 0.3 Known issues and operational concerns

1. **Runtime smoke completed:** live local process validation passed with `MCP_ENABLED=1`, including worker startup/bind, `GET /api/mcp/tools`, and `POST /api/mcp/tools/call` success/error paths.
2. **Transport shape is local/internal:** API is intentionally local, not yet hardened for remote/public exposure.
3. **No model autonomy yet:** current MCP is callable by transport, but dialogue model does not self-decide tool calls yet.
4. **Observability is mostly log-first:** metrics/SLO dashboards for MCP-specific traffic are still minimal and should be expanded.

---

## 0.4 Direction from here

### Next immediate milestone

- Run end-to-end runtime smoke for the new dialogue tool loop with the feature flag both off and on.
- Expand operational notes for default bind/timeout, degradation behavior, and failure modes.

### Phase 3

- Keep `read` / `action` namespace split.
- Introduce policy/approval/audit interfaces before enabling mutating tools.
- Maintain Rust MCP gateway as control plane and leave room for polyglot tool executors.
- Add broader parity and observability coverage for Graph↔Tree and tool-call traffic.

---

## 0.5 Implementation references (current code)

- Query facade: `crates/pa-cognitive/src/social_context.rs`
- Shared dialogue/MCP tool registry: `crates/pa-cognitive/src/dialogue_tools.rs`
- Dialogue engine tool loop + fallback path: `crates/pa-cognitive/src/dialogue_engine.rs`
- Query exports: `crates/pa-cognitive/src/lib.rs`
- MCP config: `crates/pa-mcp/src/config.rs`
- MCP registry: `crates/pa-mcp/src/registry/mod.rs`
- MCP transport/worker: `crates/pa-mcp/src/server/mod.rs`
- Runtime wiring: `crates/pa-agent/src/main.rs`
- Prompt registry entry: `config/prompt_registry.json`
- Internal tool policy prompt: `prompts/dialogue_engine/tool_policy.txt`
- MCP tests:
  - `crates/pa-mcp/tests/http_contract.rs`
  - `crates/pa-mcp/tests/tools.rs`
  - `crates/pa-mcp/tests/server.rs`
- Dialogue helper/gating tests:
  - `crates/pa-cognitive/src/dialogue_engine.rs`

---

## 0.6 Progress checklist

- [x] Standardized social query contract for affect/dialogue intents
- [x] Tree-first + graph/default fallback behavior with source metadata
- [x] Read-only MCP tool registry and execution path
- [x] MCP HTTP transport with list/call endpoints
- [x] Basic MCP contract and validation tests
- [x] Runtime smoke validated with live local process
- [x] Dialogue model-driven tool-calling loop
- [x] Bounded candidate-user orchestration policy
- [ ] Action tool policy/approval/audit framework

---

## 0.7 Key invariants (still true)

- Dialogue social retrieval remains gated.
- Affect social retrieval remains always-on.
- Graph remains source-of-truth for writes.
- Tree remains query/read projection.
- MCP v1 remains read-only.

---

## 1) Current baseline (as-is)

## 1.1 What already exists in the codebase

### Tree snapshot model in `pa-memory`

`SocialTreeSnapshot` and child nodes already exist:

- `relationship_core`
- `dynamic_state`
- `self_other_model`
- `derived_summaries`
- `meta`

Source: `crates/pa-memory/src/graph.rs`.

### Projection and read APIs in `CognitiveGraph`

Already implemented:

- `project_social_tree(user_id, memory_hint)`
- `get_social_tree_snapshot(user_id)`
- `get_or_project_social_tree_snapshot(user_id, memory_hint)`

Current projection maps Graph edges (`attitudes_towards`, `illusion_of`) into `social_tree_root`.

### Query facade in `pa-cognitive`

Already implemented runtime-facing helpers:

- `load_affect_social_context(...)` (tree-first, graph fallback)
- `load_dialogue_social_summary(...)` (tree-first, graph fallback)

Source: `crates/pa-cognitive/src/social_context.rs`.

### Runtime integration

- Dialogue: social-summary retrieval is gated.
- Affect: social context is always-on per turn.
- Affect update path already triggers tree projection after graph updates.

## 1.2 Current gaps

1. Full Graph↔Tree parity framework is still incomplete (currently logs/tests-first).
2. Runtime smoke on live process has been validated for Phase 1 operational close-out.
3. Incident lifecycle and semantic edges are not fully materialized.

---

## 2) Design goals and non-goals

## 2.1 Goals

1. Standardize query semantics around **two primary intents**:
   - Affect-rich context
   - Dialogue summary context
2. Preserve invariants:
   - Dialogue should not be over-conditioned.
   - Affect should never lose social context.
3. Keep retrieval low-latency and resilient with safe stale fallback.
4. Provide a stable, versioned, read-only MCP interface.
5. Enable future extension toward incidents and semantic edges.

## 2.2 Non-goals (v1)

1. Do not move source-of-truth from Graph to Tree.
2. Do not expose write endpoints via MCP.
3. Do not alter provider-specific logic.
4. Do not redesign overall affect/dialogue prompt strategy.

---

## 3) Canonical architecture

```text
[AffectEvaluatorWorker] --writes--> [CognitiveGraph (Surreal edges)]
                                  \-> [project_social_tree] -> [social_tree_root]

[DialogueEngineWorker] --query--> [Social Query Facade] --read--> [social_tree_root]
[AffectEvaluatorWorker] --query-> [Social Query Facade] --read--> [social_tree_root]

Fallback path (controlled):
Social Query Facade -> Graph direct read -> derived context

Future:
[MCP Context Server read-only] -> Social Query Facade -> Tree/Graph fallback
```

### 3.1 Layer responsibilities

1. **Graph write layer (`pa-memory::graph`)**
   - Persists social/illusion/dynamics delta updates.
   - Remains short/mid-horizon source-of-truth.

2. **Projection layer (`project_social_tree`)**
   - Converts current Graph state into query-optimized snapshots.
   - Writes into `social_tree_root`.

3. **Query facade layer (`pa-cognitive::social_context` + future `pa-memory::social_query`)**
   - Accepts intent + policy.
   - Returns consumer-specific payloads.
   - Enforces freshness/fallback/observability behavior.

4. **Consumer layer (Affect/Dialogue/MCP)**
   - Affect: rich payload (always-on).
   - Dialogue: compact summary (conditional).
   - MCP: read-only tool-compatible outputs.

---

## 4) Data model specification

## 4.1 Tree root record

Table: `social_tree_root`
Primary lookup: `user_id` (record id components are sanitized in current write path)

### 4.1.1 `relationship_core`

| Field | Range | Meaning |
|---|---:|---|
| affinity | -1..1 | Long-horizon baseline affinity |
| attachment | -1..1 | Long-horizon baseline attachment |
| trust | -1..1 | Long-horizon baseline trust |
| safety | -1..1 | Long-horizon baseline safety |
| tension | -1..1 | Long-horizon baseline friction/tension |
| familiarity | 0..1 | Aggregated familiarity depth |
| boundary_reliability | -1..1 | Reliability of boundary respect |

### 4.1.2 `dynamic_state`

| Field | Range | Meaning |
|---|---:|---|
| tension_live | 0..1 | Short-horizon normalized tension |
| warmth_live | 0..1 | Short-horizon warmth/alignment |
| recent_shift | -1..1 | Recent directional shift |
| last_turn_impact | -1..1 | Last-turn impact estimate |
| unresolved_friction_score | 0..1 | Residual unresolved friction |

### 4.1.3 `self_other_model`

| Field | Range | Meaning |
|---|---:|---|
| perceived_user_affinity | -1..1 | Agent’s estimate of user’s perceived affinity |
| perceived_user_attachment | -1..1 | Agent’s estimate of user’s perceived attachment |
| perceived_user_trust | -1..1 | Agent’s estimate of user’s perceived trust |
| perceived_user_safety | -1..1 | Agent’s estimate of user’s perceived safety |
| perceived_user_tension | -1..1 | Agent’s estimate of user’s perceived tension |
| confidence | 0..1 | Confidence in self-other estimation |

### 4.1.4 `derived_summaries`

| Field | Domain | Meaning |
|---|---|---|
| dialogue_summary_short | string | Compact dialogue-ready summary |
| familiarity_bucket | `new|known|close` | Bucketized familiarity |
| trust_state | `fragile|neutral|stable` | Bucketized trust |
| tension_state | `low|medium|high` | Bucketized tension |

### 4.1.5 `meta`

| Field | Meaning |
|---|---|
| schema_version | Snapshot schema version |
| updated_at | Snapshot write timestamp |
| decay_policy | Decay policy applied at projection time |
| writer_version | Projection writer version |

## 4.2 Versioning policy

- `meta.schema_version` is the parser authority.
- Query layer accepts `v1` in phase 1; unsupported versions soft-fallback.
- On schema upgrades:
  1. bump `writer_version`
  2. add reader migration/compat logic
  3. roll out dual-read before strict switch

---

## 5) Query model design

## 5.1 Query intents

### Intent A: Affect-rich context

- Consumer: `AffectEvaluatorWorker`
- Payload: full metrics + self-other model + context depth
- Staleness tolerance: low (prefer fresh)
- Fallback policy: **context must always be available**

### Intent B: Dialogue social summary

- Consumer: `DialogueEngineWorker`
- Payload: familiarity/trust/tension + short summary
- Staleness tolerance: moderate
- Fallback policy: may return `None` for graceful degradation

## 5.2 Proposed internal Rust contract

```rust
pub enum SocialQueryIntent {
    AffectRich,
    DialogueSummary,
}

pub struct SocialQueryOptions {
    pub memory_hint: f32,
    pub force_project: bool,
    pub max_staleness_ms: u64,
    pub allow_stale_fallback: bool,
}

pub enum SocialQueryResult {
    Affect(AffectSocialContext),
    Dialogue(Option<DialogueSocialSummary>),
}
```

## 5.3 Freshness defaults by intent

Suggested v1 defaults:

- **AffectRich**
  - `max_staleness_ms = 60_000`
  - `allow_stale_fallback = true` (if projection fails)
- **DialogueSummary**
  - `max_staleness_ms = 300_000`
  - `allow_stale_fallback = true`

Rationale:

- Affect is turn-sensitive and should prefer fresher snapshots.
- Dialogue summary can tolerate more staleness to reduce latency/compute pressure.

## 5.4 Canonical query algorithm

```text
Input: user_id, intent, options

1) Try reading tree snapshot by user_id.
2) If snapshot exists:
   a) If fresh => return mapped payload.
   b) If stale => continue to step 3.
3) Attempt projection (unless policy disables it).
4) If projection succeeds => return mapped payload from fresh snapshot.
5) If projection fails:
   a) If snapshot existed and allow_stale_fallback => return stale mapped payload + stale flag log.
   b) Else fallback to graph-derived mapping.
6) If all paths fail:
   - Affect intent: return default affect context (never hard-empty).
   - Dialogue intent: return None.
```

## 5.5 Deterministic mapping rules

Keep compatibility with current projection behavior:

- `graph_depth = avg(abs(affinity, attachment, trust, safety))`
- `context_depth = clamp(graph_depth + memory_hint, 0..1)`
- `familiarity_bucket(context_depth)`
- `trust_state(trust)`
- `tension_state(tension)`
- `tension_live = clamp((tension + 1)/2, 0..1)`
- `warmth_live = clamp((affinity + safety + 2)/4, 0..1)`

---

## 6) Projection strategy

## 6.1 Trigger points

### A) Eager projection on update path (already exists)

After `update_social_graph(...)` in affect flow, trigger `project_social_tree(...)` for the target user.

### B) Lazy projection on query miss (already exists)

`get_or_project_social_tree_snapshot(...)`:

- snapshot exists -> return snapshot
- snapshot missing -> project then return

### C) Planned: staleness-triggered lazy refresh

If snapshot is too old for the intent policy, query layer attempts refresh before returning.

## 6.2 Stampede control (planned)

Risk: concurrent requests for the same `user_id` on miss/stale can cause projection bursts.

Mitigation:

- Per-user async lock keyed by `user_id`.
- If lock is busy, either short-wait or use stale snapshot depending on intent policy.

## 6.3 Failure containment

- Projection failure must not break dialogue pipeline.
- Affect must still produce context via stale/graph/default fallback.
- Log explicit reasons: parse error / db error / projection error / stale return.

---

## 7) MCP Context Server design (read-only v1)

## 7.1 Scope (v1)

Expose read tools only:

1. `social.get_affect_context`
2. `social.get_dialogue_summary`
3. (optional debug) `social.get_tree_snapshot`

Do **not** expose write tools in v1.

## 7.2 Tool contracts (JSON)

### Tool: `social.get_affect_context`

**Input**

```json
{
  "user_id": "zvwgvx",
  "memory_hint": 0.15,
  "max_staleness_ms": 60000,
  "allow_stale_fallback": true
}
```

**Output**

```json
{
  "user_id": "zvwgvx",
  "known": true,
  "metrics": {
    "affinity": 0.12,
    "attachment": 0.03,
    "trust": 0.10,
    "safety": 0.11,
    "tension": -0.02,
    "context_depth": 0.28
  },
  "illusion": {
    "affinity": 0.08,
    "attachment": 0.01,
    "trust": 0.09,
    "safety": 0.07,
    "tension": 0.02
  },
  "meta": {
    "source": "tree",
    "stale": false,
    "schema_version": "v1",
    "updated_at": "2026-03-30T15:16:01.534514+00:00"
  }
}
```

### Tool: `social.get_dialogue_summary`

**Input**

```json
{
  "user_id": "zvwgvx",
  "memory_hint": 0.15,
  "max_staleness_ms": 300000,
  "allow_stale_fallback": true
}
```

**Output**

```json
{
  "user_id": "zvwgvx",
  "familiarity": "known",
  "trust_state": "neutral",
  "tension_state": "low",
  "summary": "[social summary] user=zvwgvx familiarity=known trust=neutral tension=low.",
  "meta": {
    "source": "tree",
    "stale": false,
    "schema_version": "v1",
    "updated_at": "2026-03-30T15:16:01.534514+00:00"
  }
}
```

## 7.3 Transport + hosting

### Recommended option

- Host MCP context server as a dedicated process (new crate) or integrate into `pa-cockpit-api` based on operational preference.
- MCP handlers must call the Social Query Facade and should not query DB directly.

### Why

- Clear boundary between protocol and domain logic.
- Better testability, observability, and versioning.
- Avoid duplicated business rules.

## 7.4 Authorization and safety

- v1 assumption: local trusted runtime (not public internet).
- If remote exposure is later required:
  - enforce auth token/session
  - enforce tool-level ACL
- Log redaction:
  - never log API keys/tokens
  - truncate long free-text fields where needed

---

## 8) Dialogue integration model

## 8.1 Existing behavior (kept)

- Dialogue always runs social-gate decision before summary retrieval.
- Summary fetch is attempted only when `decision.should_fetch == true`.

## 8.2 Planned enhancements

- Add a lightweight classifier (later phase) in addition to heuristic score.
- Allow stale summary injection when policy allows, to reduce cold-start behavior.

## 8.3 Prompt-assembly invariant

Continue using a single merged dialogue system prompt for provider/router compatibility.

---

## 9) Affect integration model

## 9.1 Existing behavior (kept)

- Affect always loads social context per turn.
- Tree failure already falls back to graph/default path so context is not lost.

## 9.2 Planned enhancements

- Affect path uses `AffectRich` intent with stricter freshness.
- Add explicit stale markers in logs when stale snapshot is returned.

---

## 10) Observability and telemetry

## 10.1 Log events

Keep existing logs and add standardized events:

- `kind="social.query"` for request/result/fallback
- `kind="social.projection"` for projection attempts
- `kind="social.parity"` for divergence reports

## 10.2 Metrics

Minimum set:

1. `social_query_total{intent,source,result}`
2. `social_query_latency_ms_bucket{intent}`
3. `social_projection_total{result}`
4. `social_projection_latency_ms_bucket`
5. `social_stale_return_total{intent}`
6. `social_fallback_total{intent,fallback=tree_stale|graph|default}`
7. `social_parity_drift_gauge{field}`

## 10.3 Suggested SLOs

- Affect query p95 < 80ms (excluding LLM call)
- Dialogue summary query p95 < 40ms
- Projection success rate > 99%
- Affect stale-return ratio < 5% in steady state

---

## 11) Parity framework (Graph ↔ Tree)

## 11.1 Why

Projection drift can happen due to:

- formula changes
- prolonged stale snapshots
- local update-path failures

## 11.2 Parity checks

Run periodic checks (time-based or request-sampled):

- compare key fields:
  - affinity/attachment/trust/safety/tension
  - perceived_user_*
- compute absolute diff and classify:
  - `<= 0.01` good
  - `0.01..0.05` warning
  - `> 0.05` investigate

## 11.3 Alerting conditions

- drift above threshold sustained for > M minutes
- projection-failure spikes
- sudden increase in default-fallback ratio

---

## 12) Error model and fallback semantics

## 12.1 Error classes

1. `TreeNotFound`
2. `TreeReadError`
3. `ProjectionError`
4. `SchemaMismatch`
5. `StaleExceeded`

## 12.2 Fallback rules

### Affect

- Prefer fresh tree
- Else stale tree
- Else graph-derived context
- Else default zero context (no panic)

### Dialogue

- Prefer fresh summary
- Else stale summary
- Else `None` (no social injection)

---

## 13) Security and privacy

## 13.1 Data safety

- Do not store secrets in tree records.
- Sanitize `user_id` before record-id/component usage.
- Do not persist raw secret-bearing prompt content into tree fields.

## 13.2 Prompt safety

- `dialogue_summary_short` is internal derived text; enforce length limits to prevent prompt bloat.
- If future summaries are externally generated, sanitize control characters before injection.

## 13.3 MCP boundary

- Read-only by default.
- Tool outputs may include sensitive social signals; define explicit policy for external consumers.

---

## 14) Implementation blueprint (file-by-file)

## 14.1 `crates/pa-memory/src/graph.rs`

- Keep existing projection/read primitives.
- Add helper utilities for `meta.updated_at` freshness checks as needed.
- Add parity sampling helper (later phase).

## 14.2 `crates/pa-cognitive/src/social_context.rs`

- Normalize facade into intent/options-driven query API (possibly split into dedicated module).
- Return extra response meta (`source`, `stale`, `schema/version`) for observability.
- Standardize fallback logging keys.

## 14.3 `crates/pa-cognitive/src/dialogue_engine.rs`

- Keep current gate logic.
- Route summary fetch through the normalized facade API.

## 14.4 `crates/pa-cognitive/src/affect_evaluator.rs`

- Keep always-on context retrieval.
- Add explicit logs for source tree/graph/default and stale flags.

## 14.5 MCP module (new, phase v1)

Create new crate/worker or module inside cockpit runtime:

- expose tool handlers
- map request -> query facade
- encode standardized JSON response contracts

---

## 15) Rollout plan

## Phase 1 — Query contract + MCP read-only transport (**current status: implemented + runtime-smoked**)

Done:

- Defined `SocialQueryIntent` + `SocialQueryOptions`
- Normalized payload + `meta` via `query_social_context`
- Added mapping/fallback tests in `pa-cognitive`
- Shipped read-only MCP transport with two tools + contract tests in `pa-mcp`
- Validated local runtime startup plus MCP list/call success and error paths

Still open after close-out:

- extra operational metrics/dashboarding for MCP request paths

## Phase 2 — Model-driven dialogue tool calling (**current status: implemented in code, runtime smoke still pending**)

Done:

- Added feature-flagged tool-call turn loop in dialogue engine
- Kept the social gate as pre-filter to control latency
- Added bounded candidate-user targeting policy from current author + recent history
- Restricted dialogue to internal read tools only
- Preserved graceful degradation to the legacy direct-summary path

**Exit criteria:** dialogue can safely call read tools on-demand and degrade gracefully.

## Phase 3 — Parity and action-ready architecture

- Expand Graph↔Tree parity checks + alerting.
- Introduce policy/approval/audit interfaces for future action tools.
- Keep action namespace disabled by default.

**Exit criteria:** drift monitoring is operational and action safety interfaces exist.

## Phase 4 — Optional expansion

- incident lifecycle
- semantic edges (`derived_from`, `affects`, `references`)
- classifier-assisted dialogue social fetch
- polyglot tool executor adapters under Rust MCP control plane

---

## 16) Test strategy (detailed)

## 16.1 Unit tests

1. Graph -> Tree mapping formulas:
   - boundary values (-1, 0, 1)
   - memory_hint clamp behavior
2. Bucketization:
   - threshold-edge cases for familiarity/trust/tension
3. Freshness parser:
   - invalid timestamps
   - empty metadata

## 16.2 Integration tests (local SurrealDB)

1. `project_social_tree` creates/updates root correctly.
2. `get_or_project_social_tree_snapshot` miss -> project -> readback.
3. stale snapshot + projection failure -> correct intent-based fallback.
4. concurrent requests for same user do not trigger projection storms.

## 16.3 End-to-end tests

1. Dialogue turn with no social cue -> mode none.
2. Dialogue turn with social cue -> summary injection.
3. Affect turn always receives social context text.
4. Graph update -> projection -> subsequent query returns fresh tree.

## 16.4 MCP tests

1. input validation (missing/invalid user_id)
2. response schema compliance
3. read-only behavior enforcement
4. latency/load smoke tests

---

## 17) Capacity and performance notes

1. Single root record per user keeps read path simple and low round-trip.
2. Projection cost is mostly graph reads + one upsert; parity should be sampled, not full-scan by default.
3. Dialogue path must remain low-latency: summary query should fail fast and permit `None`.
4. Affect path prioritizes correctness but still needs bounded latency.

---

## 18) Open questions

1. Should `social_tree_root` become a true multi-record tree (node-level tables) in v1.1?
2. Should incidents be stored in dedicated Surreal tables now or after parity stabilization?
3. Should MCP live in a dedicated crate or piggyback cockpit runtime?
4. Do we need an in-process LRU cache before external MCP exposure?

---

## 19) Recommended immediate next actions

1. Run runtime smoke for the dialogue tool loop with `dialogue_tool_calling_enabled` both `false` and `true`.
2. Add MCP/dialogue tool-call metrics (success/error/timeout + latency) and wire to cockpit/monitoring path.
3. Draft action-tool safety interfaces (policy/approval/audit) without enabling mutating tools.
4. Decide whether to expose a lightweight MCP health/debug view through cockpit.

---

## 20) Invariants checklist (must remain true)

- [x] Dialogue does not receive heavy social context unless gate requires it.
- [x] Affect always receives social context (tree/graph/default).
- [x] Graph remains write/source-of-truth.
- [x] Tree remains read/query projection.
- [~] All fallbacks are observable via logs/metrics (log coverage in place; metrics expansion still needed).
- [x] MCP v1 remains read-only.

---

## Appendix A) Canonical facade pseudo-code

```rust
async fn query_social(intent: SocialQueryIntent, user_id: &str, opt: SocialQueryOptions) -> SocialQueryResult {
    let maybe_snapshot = read_tree(user_id).await.ok();

    if let Some(snapshot) = &maybe_snapshot {
        if is_fresh(snapshot, intent, opt.max_staleness_ms) {
            return map_snapshot(intent, snapshot);
        }
    }

    if opt.force_project || maybe_snapshot.is_none() || is_stale_for_intent(&maybe_snapshot, intent, opt.max_staleness_ms) {
        if let Ok(fresh) = project_tree(user_id, opt.memory_hint).await {
            return map_snapshot(intent, &fresh);
        }
    }

    if opt.allow_stale_fallback {
        if let Some(snapshot) = maybe_snapshot {
            return map_snapshot_with_stale_flag(intent, &snapshot);
        }
    }

    match intent {
        SocialQueryIntent::AffectRich => map_graph_or_default_affect(user_id, opt.memory_hint).await,
        SocialQueryIntent::DialogueSummary => map_graph_or_none_dialogue(user_id, opt.memory_hint).await,
    }
}
```

---

## Appendix B) Decision log

- Keep hybrid architecture (Graph write + Tree read) to minimize operational risk.
- Keep MCP v1 read-only to avoid early write-path coupling.
- Keep dialogue social gate to avoid over-conditioning.
- Keep affect always-on social retrieval to preserve continuity.
