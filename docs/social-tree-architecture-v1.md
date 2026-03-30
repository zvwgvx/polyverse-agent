# Social State Architecture Handoff (Graph + Tree)

This is the handoff document for the social-state redesign work.

It covers:
- core concepts (what Graph vs Tree are for)
- design goals and constraints
- what has already been implemented
- what is still pending
- recommended implementation direction for the next team

---

## 1) Executive summary

The system is moving from a single "always-injected social block" prompt strategy to a split architecture:

- **CognitiveGraph** remains the current write/storage backbone for social dynamics.
- **Social Tree** is introduced as a query/read model for scalable retrieval.

Operational policy:
- **Affect evaluator** should always retrieve social context (always-on).
- **Dialogue engine** should retrieve social context only when needed (conditional/on-demand).

Primary motivation:
- reduce role drift from over-conditioning
- keep social continuity quality
- prepare for MCP/tool-style retrieval at scale

---

## 2) Why this redesign was needed

### Problem observed

Dialogue prompt was composed from multiple system parts and historically over-conditioned by social/state context on every turn.

This caused:
- persona drift risk
- unstable style due to heavy meta-steering
- provider compatibility issues (some routers/providers effectively honor only the first system message)

### Design response

1. Minimize always-on social injection in dialogue.
2. Keep affect evaluation always context-aware.
3. Separate social retrieval concerns by use-case.
4. Merge outbound dialogue system context into one system message for compatibility.

---

## 3) Core concepts: Graph vs Tree

## 3.1 CognitiveGraph (current backbone)

What it is:
- relationship/affect dynamics stored in SurrealDB edges.

What it is good at:
- delta updates
- relationship math
- stable write semantics
- existing compatibility with current workers/cockpit/state updates

Current key operations (implemented):
- `get_social_context(...)`
- `update_social_graph(...)`
- `update_illusion_graph(...)`
- `update_observed_dynamic(...)`

## 3.2 Social Tree (target query model)

What it is:
- per-user structured read model optimized for retrieval.

What it is good at:
- affect-facing rich context
- dialogue-facing compact summaries
- selective fetch (rather than passive full injection)
- future MCP/tool querying

Important: v1 approach is **hybrid**, not a hard replacement.

- Graph = write/source-of-truth (for now)
- Tree = read/query projection layer

---

## 4) Social Tree v1 schema (target)

Per-user root:
- `social/{user_id}`

Nodes:
- `relationship_core`
- `dynamic_state`
- `self_other_model`
- `incidents/*`
- `derived_summaries`
- `meta`

### 4.1 `relationship_core`

Long-horizon baseline.

Suggested fields:
- `affinity`, `attachment`, `trust`, `safety` (`-1..1`)
- `familiarity` (`0..1`)
- `boundary_reliability` (`-1..1`)

### 4.2 `dynamic_state`

Short-horizon dynamics.

Suggested fields:
- `tension_live`, `warmth_live`
- `recent_shift`
- `last_turn_impact`
- `unresolved_friction_score`

### 4.3 `self_other_model`

Perceived user view of the agent (illusion/perception model).

Suggested fields:
- `perceived_user_affinity`, `perceived_user_trust`, `perceived_user_safety`, `perceived_user_tension`
- `confidence`

### 4.4 `incidents/{incident_id}`

Evidence-backed events.

Suggested fields:
- `id`, `ts`, `kind`, `summary`, `impact_delta`, `source_message_ids`, `confidence`

### 4.5 `derived_summaries`

Dialogue-friendly outputs.

Suggested fields:
- `dialogue_summary_short`
- `familiarity_bucket` (`new|known|close`)
- `trust_state` (`fragile|neutral|stable`)
- `tension_state` (`low|medium|high`)

### 4.6 `meta`

Versioning/lifecycle policy:
- `schema_version`, `updated_at`, `decay_policy`, `writer_version`

---

## 5) Edge semantics in Tree

Two categories:

1. **Structural edges** (`contains`)
- root contains core/dynamic/self_other/incidents/summaries/meta

2. **Semantic edges**
- `derived_from`: where node values came from (graph/state/evaluator output)
- `affects`: incident impact targets
- `references`: provenance pointers to source message ids

---

## 6) Mapping from current Graph to Tree

Current mapping strategy:
- `attitudes_towards` -> `relationship_core`
- `illusion_of` -> `self_other_model`
- `interacts_with` (+ recent deltas) -> `dynamic_state`

This allows phased rollout with minimal disruption.

---

## 7) Current implementation status (already done)

## 7.1 Dialogue path changes

Implemented:
- removed always-on full social block from dialogue by default
- added conditional social retrieval gate
- added social observability logs (`kind="prompt.social"`, mode)
- merged all dialogue system parts into **one** outbound system message

Why single system message:
- provider/router compatibility (avoid loss of later system blocks)

## 7.2 Affect path changes

Implemented:
- affect still always retrieves social context per turn
- added concurrency limiter usage (`Semaphore` permit acquired before evaluation)
- hardened parser for illusion delta key drift with serde aliases (accept `expected_delta_*` and `delta_*` variants)

## 7.3 Shared social facade

Implemented in `crates/pa-cognitive/src/social_context.rs`:
- typed models for affect and dialogue summary retrieval
- affect context serialization helper (`to_prompt_text()`)
- dialogue summary retrieval helper (`load_dialogue_social_summary(...)`)

## 7.4 State prompt trimming

Implemented config reduction:
- kept domains: `session_social`, `emotion`, `system`, `environment`
- disabled state legend

Purpose:
- reduce over-steering and drift pressure in dialogue prompt.

---

## 8) What is not done yet

1. Full persisted Tree schema (nodes/incidents) is not fully materialized as a dedicated store yet.
2. MCP/tool runtime for social retrieval is not implemented yet.
3. Dialogue gate is heuristic scoring (not classifier/intent model).
4. Incident lifecycle (TTL/retention/parity checks) is not fully implemented yet.
5. Graph->Tree divergence monitoring is only partial (logs, not full parity framework).

---

## 9) Recommended direction (next team)

### Phase 1 (stabilization)
- keep current hybrid architecture
- monitor drift and social-mode logs
- monitor affect parse-error and summary hit/miss

### Phase 2 (tree projection)
- formalize tree structs and projection functions
- add incident creation/update policy
- add retention/decay strategy

### Phase 3 (query APIs)
- `get_affect_context(user_id)` rich payload (always-on)
- `get_dialogue_summary(user_id)` compact payload (conditional)

### Phase 4 (MCP/tool)
- expose social query interfaces via tool-style retrieval
- keep dialogue on-demand fetch
- keep affect system-orchestrated always-on retrieval

### Phase 5 (optional migration)
- evaluate dual-write / source-of-truth transition only after parity confidence is high

---

## 10) Operational invariants

These should remain true during migration:

1. Dialogue should not receive heavy social context unless gated.
2. Affect should never lose access to social context.
3. Outbound dialogue request should keep a single merged system message for compatibility.
4. Provider-specific fixes are out of scope unless explicitly requested.
5. Changes must preserve existing graph/state update semantics.

---

## 11) Key files for team handoff

- Dialogue assembly and social gate:
  - `crates/pa-cognitive/src/dialogue_engine.rs`
- Shared social facade/types:
  - `crates/pa-cognitive/src/social_context.rs`
- Affect retrieval/update and parser robustness:
  - `crates/pa-cognitive/src/affect_evaluator.rs`
- Graph read/write primitives:
  - `crates/pa-memory/src/graph.rs`
- State prompt scope config:
  - `config/state_prompt.json`

---

## 12) Handoff conclusion

The redesign has already delivered the critical first outcomes:
- reduced dialogue over-conditioning
- preserved affect continuity
- improved provider compatibility via single merged system prompt
- established a scalable path toward Social Tree + MCP retrieval

The next team should continue from the hybrid model and incrementally materialize tree projection and tool-query interfaces.
