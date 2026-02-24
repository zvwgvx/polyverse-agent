# Agent State Space Dimensions

This document defines the agent state space for a true "digital entity" design:
- Continuous state (not reset per chat turn)
- Internal condition (body/affect/safety)
- Social relationships (social)
- Topic/entity attitudes (preference)
- Ongoing objectives (goal)

## 1) Notation and total dimension formula

Symbols:
- `U`: number of tracked users in `SocialState`
- `E`: number of tracked entities/topics in `PreferenceState`
- `G`: number of active goals

Total dimensions (full version):

`D_total = 37 + 8*U + 5*E + 7*G`

Where:
- `37`: fixed global dimensions
- `8*U`: per-user social dimensions
- `5*E`: per-entity/topic preference dimensions
- `7*G`: dynamic goal dimensions

If the preference module is not enabled:

`D_total = 37 + 8*U + 7*G`

## 2) Global domains (37 fixed dimensions)

### 2.1 BodyState (6 dimensions, global)

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `energy` | f32 | `[0,100]` | Current energy level |
| `fatigue` | f32 | `[0,1]` | Accumulated fatigue |
| `sleep_pressure` | f32 | `[0,1]` | Need for rest/consolidation |
| `attention_budget` | f32 | `[0,1]` | Remaining attention capacity |
| `recovery_rate` | f32 | `[0,1]` | Recovery speed |
| `overload` | f32 | `[0,1]` | Input/concurrency overload level |

### 2.2 AffectState (6 dimensions, global)

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `valence` | f32 | `[-1,1]` | Overall positive/negative affect |
| `arousal` | f32 | `[0,1]` | Internal activation intensity |
| `irritability` | f32 | `[0,1]` | Sensitivity to negative triggers |
| `mood_stability` | f32 | `[0,1]` | Mood stability |
| `stress` | f32 | `[0,1]` | Overall stress |
| `novelty_drive` | f32 | `[0,1]` | Tendency to seek novelty |

### 2.3 EpistemicState (6 dimensions, global)

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `certainty` | f32 | `[0,1]` | Current confidence in beliefs |
| `uncertainty` | f32 | `[0,1]` | Overall uncertainty |
| `contradiction_score` | f32 | `[0,1]` | Degree of conflict in memory/context |
| `evidence_freshness` | f32 | `[0,1]` | Recency of supporting evidence |
| `source_reliability` | f32 | `[0,1]` | Average source reliability |
| `known_unknowns_norm` | f32 | `[0,1]` | Normalized "known unknowns" level |

### 2.4 NarrativeState (6 dimensions, global)

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `identity_coherence` | f32 | `[0,1]` | Identity consistency |
| `persona_consistency` | f32 | `[0,1]` | Stability of persona/style |
| `self_efficacy` | f32 | `[-1,1]` | Self-assessed handling capability |
| `relationship_coherence` | f32 | `[0,1]` | Consistency of relationship narrative |
| `memory_continuity` | f32 | `[0,1]` | Memory continuity across sessions/reboots |
| `role_conflict` | f32 | `[0,1]` | Internal role conflict |

### 2.5 PolicyState (6 dimensions, global)

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `response_verbosity_target` | f32 | `[0,1]` | Target response length (normalized) |
| `assertiveness` | f32 | `[0,1]` | Response assertiveness |
| `creativity` | f32 | `[0,1]` | Expressive diversity level |
| `latency_budget_norm` | f32 | `[0,1]` | Turn latency budget (normalized) |
| `tool_use_threshold` | f32 | `[0,1]` | Threshold for invoking tools |
| `clarification_tendency` | f32 | `[0,1]` | Tendency to ask clarifying questions |

### 2.6 SafetyState (7 dimensions, global)

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `risk_score` | f32 | `[0,1]` | Aggregated risk score |
| `policy_violation_prob` | f32 | `[0,1]` | Probability of policy violation |
| `manipulation_risk` | f32 | `[0,1]` | Risk of manipulation |
| `privacy_risk` | f32 | `[0,1]` | Privacy misuse/leak risk |
| `autonomy_limit_pressure` | f32 | `[0,1]` | Pressure to constrain autonomy |
| `cooldown_pressure` | f32 | `[0,1]` | Pressure to pause/cool down |
| `escalation_required_flag` | f32 | `{0,1}` | Whether escalation is required |

## 3) Dynamic domains

### 3.1 SocialState (8 dimensions per user)

Each tracked user has one `SocialVector`:

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `affinity` | f32 | `[-1,1]` | Like/dislike signal |
| `attachment` | f32 | `[-1,1]` | Closeness/distance tendency |
| `trust` | f32 | `[-1,1]` | Trust level |
| `safety` | f32 | `[-1,1]` | Felt social safety |
| `tension` | f32 | `[0,1]` | Social tension |
| `familiarity` | f32 | `[0,1]` | Familiarity from history |
| `boundary_risk` | f32 | `[0,1]` | Boundary-crossing risk |
| `reciprocity_estimate` | f32 | `[-1,1]` | Estimated reciprocity |

Dimension contribution: `8*U`

### 3.2 PreferenceState (5 dimensions per entity/topic)

Each entity/topic has one `PreferenceVector`:

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `preference` | f32 | `[-1,1]` | Like/dislike toward entity/topic |
| `fascination` | f32 | `[0,1]` | Attraction/interest intensity |
| `stress_association` | f32 | `[0,1]` | Stress linkage to the entity/topic |
| `certainty` | f32 | `[0,1]` | Confidence in this preference profile |
| `freshness` | f32 | `[0,1]` | Data recency for this profile |

Dimension contribution: `5*E`

Compatibility note with current code:
- `preference`, `stress_association`, and `fascination` map directly to existing emotion-edge values.
- `certainty` and `freshness` are recommended additions for more stable updates.

### 3.3 GoalState (7 dimensions per goal)

Each active goal has one `GoalVector`:

| Dimension | Type | Range | Meaning |
|---|---|---|---|
| `utility` | f32 | `[0,1]` | Expected value if completed |
| `urgency` | f32 | `[0,1]` | Current urgency |
| `progress` | f32 | `[0,1]` | Completion progress |
| `expected_cost` | f32 | `[0,1]` | Expected cost (time/token/risk) |
| `deadline_pressure` | f32 | `[0,1]` | Deadline pressure |
| `commitment` | f32 | `[0,1]` | Commitment strength |
| `blockage_risk` | f32 | `[0,1]` | Probability of getting blocked |

Dimension contribution: `7*G`

`goal_status` (active/blocked/done) should be stored as enum metadata, not necessarily as a continuous dimension.

## 4) Quick summary

- Domain count:
  - Core view: `8 domains` (without splitting preference as a separate module)
  - Full recommended view: `9 domains` (with explicit `PreferenceState`)
- Total dimensions:
  - Core: `37 + 8*U + 7*G`
  - Full: `37 + 8*U + 5*E + 7*G`

## 5) Mapping to current polyverse-agent codebase

- Social dimensions already grounded:
  - `affinity`, `attachment`, `trust`, `safety`, `tension`
  - file: `crates/pa-memory/src/graph.rs`
- Preference dimensions already partially grounded:
  - `preference`, `stress_association` (currently `stress`), `fascination`
  - file: `crates/pa-memory/src/graph.rs`
- Goal state does not yet have a dedicated module; recommended: `GoalStore + GoalReducer`.
- Body/Affect/Narrative/Policy/Safety should be centralized in a `UnifiedInternalState` with event-driven reducers.
