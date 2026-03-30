# Truth KGraph v1

This document defines a minimal knowledge graph for stable, provenance-backed facts.

The current codebase already has:
- chat history in SQLite
- episodic memory in LanceDB
- social and affect graph in SurrealDB

What is still missing is a durable truth layer for facts such as:
- who a user is
- what the user prefers
- what project or topic the conversation refers to
- which commitments or constraints are currently true

This document describes the first version of that layer.

## 1. Goal

`Truth KGraph v1` is the source of truth for facts that should survive across turns and sessions.

It exists to support:
- prompt grounding
- goal tracking
- autonomy and follow-up decisions
- operator inspection in cockpit

It should answer questions like:
- "What do we know about this user?"
- "What is the current project identity?"
- "What constraints did the user state?"
- "Which fact is currently believed, and why?"

## 2. Non-goals

`Truth KGraph v1` is not:
- a replacement for episodic memory
- a replacement for the social or affect graph
- a full world model
- a storage layer for transient states such as emotion, tension, fatigue, or style
- a planner by itself

Do not store the following as truth facts:
- `session_social.*`
- `emotion.*`
- `system.*`
- temporary mood or turn-local tone

Those belong to the state engine or cognitive graph.

## 3. Position in the memory stack

The stack becomes:

1. `SQLite MemoryStore`
   Raw mention and reply log
2. `ShortTermMemory`
   Active session context
3. `EpisodicStore`
   Compressed semantic recollections
4. `CognitiveGraph`
   Relationship and affect dynamics
5. `Truth KGraph`
   Stable, provenance-backed facts

The rule is:
- episodic memory stores "what happened"
- cognitive graph stores "how the relationship feels"
- truth graph stores "what is believed to be true"

## 4. Storage boundary

Reuse the same SurrealDB endpoint path, but use a separate DB:

- namespace: `polyverse`
- db: `truth`

This keeps deployment simple while preventing cross-contamination with the existing cognitive graph in [graph.rs](/Users/zvwgvx/Antigravity/polyverse-agent/crates/pa-memory/src/graph.rs).

## 5. Core model

### 5.1 Entity

Entities are graph nodes.

Recommended kinds for v1:
- `agent`
- `user`
- `project`
- `topic`
- `place`
- `org`

Suggested record id shape:
- `entity:agent_<id>`
- `entity:user_<id>`
- `entity:project_<id>`
- `entity:topic_<id>`

Minimal fields:

| Field | Type | Meaning |
|---|---|---|
| `id` | record id | canonical Surreal record id |
| `kind` | string | entity kind |
| `label` | string | display name |
| `aliases` | array<string> | alternate names |
| `canonical_key` | string | stable normalized key |
| `created_at` | datetime | creation time |
| `updated_at` | datetime | last update time |

### 5.2 Fact

Facts are the actual truth layer. A fact is a normalized claim with provenance and lifecycle metadata.

Suggested table: `truth_fact`

Minimal fields:

| Field | Type | Meaning |
|---|---|---|
| `id` | record id | fact id |
| `subject` | record<entity> | fact subject |
| `predicate` | string | normalized relation name |
| `object_ref` | option<record<entity>> | object if entity-backed |
| `object_value` | option<string> | object if literal-backed |
| `value_type` | string | `entity`, `string`, `bool`, `number`, `datetime` |
| `confidence` | float | belief confidence `0..1` |
| `status` | string | `active`, `disputed`, `superseded`, `retracted` |
| `source_platform` | string | source platform |
| `source_message_id` | string | originating message id |
| `source_user_id` | string | who asserted it |
| `evidence_count` | int | number of supporting observations |
| `first_seen_at` | datetime | first observation |
| `last_seen_at` | datetime | most recent observation |
| `valid_from` | option<datetime> | explicit validity start |
| `valid_to` | option<datetime> | explicit validity end |
| `extractor` | string | model or rule name |
| `canonical_key` | string | dedup key |

`canonical_key` should be deterministic:
- entity object: `subject|predicate|object_ref`
- literal object: `subject|predicate|value_type|normalized_object_value`

### 5.3 Source record

Optional in v1, but recommended:

Suggested table: `truth_source`

Purpose:
- preserve extraction provenance without bloating every fact
- support future audit UI

Minimal fields:
- `id`
- `platform`
- `message_id`
- `user_id`
- `username`
- `snippet`
- `timestamp`

`truth_fact` can later point to one or more source records if needed.

## 6. Allowed predicates in v1

Do not allow arbitrary open-ended predicates in the first version.

Start with a tight allowlist:
- `identity.name`
- `identity.alias`
- `identity.role`
- `relationship.owner_of`
- `relationship.member_of`
- `preference.likes`
- `preference.dislikes`
- `preference.prefers`
- `project.works_on`
- `project.uses_stack`
- `constraint.do_not`
- `constraint.require`
- `commitment.promised`
- `location.based_in`

This gives enough coverage for:
- user profile grounding
- project grounding
- commitments and constraints

## 7. Ingestion pipeline

Truth extraction should not be bolted onto the semantic compressor output in [compressor.rs](/Users/zvwgvx/Antigravity/polyverse-agent/crates/pa-memory/src/compressor.rs). The compressor produces a diary-style memory event, not a normalized fact set.

Use a separate pipeline:

1. Input window
   Use the triggering raw message plus a short local context window from STM.
2. Candidate extraction
   Extract only allowlisted predicates.
3. Normalization
   Normalize subject, predicate, object, and entity ids.
4. Validation
   Reject low-confidence or malformed claims.
5. Upsert
   Insert or update the corresponding fact record.
6. Conflict check
   Compare against existing active facts with the same `subject + predicate`.

Recommended trigger points:
- `Event::Raw` for direct user assertions
- optionally `Event::BotTurnCompletion` for confirmed commitments made by the agent

Do not extract truth from every random message. Use a gate:
- direct self-description
- preference statement
- explicit instruction or constraint
- commitment or project statement

## 8. Conflict handling

This is the most important part of the design.

The system must not overwrite old facts blindly.

Rules for v1:
- if the same canonical fact appears again, raise `confidence`, update `last_seen_at`, increment `evidence_count`
- if a conflicting fact appears for the same `subject + predicate`, do not delete the old one
- mark the lower-confidence fact as `disputed` or `superseded`
- keep provenance on both sides

Examples:
- old: `user prefers discord`
- new: `user prefers telegram`

Possible result:
- latest, high-confidence fact becomes `active`
- previous fact becomes `superseded`

This prevents silent memory corruption.

## 9. Retrieval for prompt grounding

Prompt integration should be selective.

Do not dump the full truth graph into the prompt.

Retrieval policy for v1:
- always fetch top profile facts for the current user entity
- fetch project facts for the active project entity if recognized
- fetch active constraints and commitments
- cap output to a small number of facts, for example `5..12`

Suggested prompt section:
- `context.truth.header`
- `context.truth.item`

Example prompt rendering:

```text
### STABLE FACTS
- User prefers concise technical answers. [confidence=0.91]
- User owns project Polyverse Agent. [confidence=0.97]
- Constraint: do not expose private base persona prompt. [confidence=0.99]
```

This should be added in the shared cognitive context layer near [context.rs](/Users/zvwgvx/Antigravity/polyverse-agent/crates/pa-cognitive/src/context.rs), not directly hardcoded into the dialogue worker.

## 10. Relationship to goals

The goal system should consume truth facts, not invent them.

Examples:
- if truth says `commitment.promised = build cockpit`
- a goal reducer can create or strengthen a real goal object

Examples of goal-relevant truth:
- constraints
- explicit requests
- named projects
- recurring user preferences
- commitments made by the agent

This is why `Truth KGraph` should land before a full goal engine.

## 11. Suggested Rust surface

Add a separate module rather than overloading the current cognitive graph:

- `crates/pa-memory/src/truth_graph.rs`

Suggested types:

```rust
pub struct TruthGraph { ... }

pub struct TruthEntity { ... }
pub struct TruthFact { ... }
pub struct TruthFactCandidate { ... }

pub enum TruthFactStatus {
    Active,
    Disputed,
    Superseded,
    Retracted,
}
```

Suggested methods:

```rust
impl TruthGraph {
    pub async fn new(path: &str) -> Result<Self>;
    pub async fn ensure_entity(&self, input: EntityInput) -> Result<String>;
    pub async fn upsert_fact(&self, fact: TruthFactCandidate) -> Result<TruthFact>;
    pub async fn get_entity_facts(&self, entity_id: &str, limit: usize) -> Result<Vec<TruthFact>>;
    pub async fn get_relevant_facts(&self, user_id: &str, query: &str, limit: usize) -> Result<Vec<TruthFact>>;
}
```

## 12. Suggested worker boundary

Add a dedicated worker:

- `TruthGraphWorker`

Responsibilities:
- subscribe to raw events
- run extraction on gated messages
- upsert normalized facts
- expose retrieval helpers for prompt construction

Do not merge this into `MemoryWorker`.

Reason:
- memory persistence and truth extraction have different latency and failure profiles
- truth extraction will likely use its own prompts and model settings

## 13. Cockpit requirements

The cockpit should later show:
- entities
- active facts
- conflicts
- provenance
- fact confidence
- manual correction actions

Minimum useful views:
- `Facts`
- `Entities`
- `Conflicts`

## 14. Implementation order

Recommended order:

1. create `TruthGraph` storage module
2. add entity and fact schema
3. add allowlisted predicate enum
4. implement `upsert_fact` and conflict resolution
5. add read APIs for prompt retrieval
6. add `TruthGraphWorker`
7. add prompt rendering section
8. add cockpit inspection and manual override

## 15. v1 success criteria

`Truth KGraph v1` is successful if:
- facts survive process restart
- facts are provenance-backed
- conflicts do not silently overwrite each other
- prompt grounding can fetch stable facts for the active user/project
- the graph remains small, explainable, and operator-debuggable

## 16. Immediate recommendation

The next coding step should be:
- implement `truth_graph.rs`
- keep schema narrow
- do not start with automated extraction for every predicate

Start with only:
- user identity facts
- user preference facts
- project identity facts
- hard constraints
- commitments

That is enough to make the layer useful without making it noisy.
