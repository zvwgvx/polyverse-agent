export type AgentIdentityView = {
  agent_id: string;
  display_name: string;
  graph_self_id: string;
};

export type CockpitCounter = {
  raw_events: number;
  mention_events: number;
  response_events: number;
  bot_turns: number;
  system_events: number;
  intent_events: number;
  biology_events: number;
};

export type WorkerStateView = {
  name: string;
  status: string;
};

export type CockpitOverview = {
  identity: AgentIdentityView;
  started_at: string;
  uptime_seconds: number;
  counters: CockpitCounter;
  workers: WorkerStateView[];
};

export type StateRow = {
  id: string;
  domain: string;
  scope: string;
  description: string;
  update_mode: string;
  range_min: number;
  range_max: number;
  baseline?: number;
  decay_k?: number;
  max_delta_per_turn?: number;
  value: number;
  updated_at: string;
  source: string;
};

export type CockpitEventView = {
  ts: string;
  kind: string;
  summary: string;
};

export type PromptEntry = {
  id: string;
  path: string;
};

export type PromptDocument = {
  id: string;
  path: string;
  content: string;
};

export type PromptUpdateRequest = {
  id: string;
  content: string;
};

export type StateDeltaLog = {
  sequence: number;
  dimension_id: string;
  before: number;
  after: number;
  reason: string;
  actor: string;
  source: string;
  timestamp: string;
};

export type StateMetricEntry = {
  source: string;
  count: number;
};

export type StateMetricsSnapshot = {
  total_updates: number;
  last_updated_at: string | null;
  by_source: StateMetricEntry[];
};

export type ManualPatchRequest = {
  dimension_id: string;
  value: number;
  reason: string;
  actor?: string;
};

export type ManualPatchResult = {
  applied: StateDeltaLog;
  row: StateRow;
};

export type ActiveSessionSnapshot = {
  conversation: string;
  platform: string;
  channel_id: string;
  message_count: number;
  started_at: string;
  last_active: string;
  participants: string[];
};

export type MemoryMessage = {
  id: string;
  platform: string;
  channel_id: string;
  user_id: string;
  username: string;
  content: string;
  is_mention: boolean;
  is_bot_response: boolean;
  reply_to_user: string | null;
  timestamp: string;
  importance: number;
};

export type MemoryOverview = {
  persisted_message_count: number;
  persisted_recent_messages: MemoryMessage[];
  active_session_count: number;
  active_sessions: ActiveSessionSnapshot[];
  active_recent_messages: MemoryMessage[];
};

export type EpisodicChunk = {
  id: string;
  content: string;
  timestamp: string;
  importance: number;
  metadata: string;
};

export type EpisodicOverview = {
  total_chunks: number;
  recent_chunks: EpisodicChunk[];
};

export type RelationshipNode = {
  id: string;
  label: string;
  kind: string;
};

export type RelationshipEdge = {
  id: string;
  kind: string;
  source: string;
  target: string;
  affinity: number | null;
  attachment: number | null;
  trust: number | null;
  safety: number | null;
  tension: number | null;
};

export type RelationshipGraphSnapshot = {
  self_node_id: string;
  self_display_name: string;
  nodes: RelationshipNode[];
  edges: RelationshipEdge[];
};

export type LoadAverageView = {
  one: number;
  five: number;
  fifteen: number;
};

export type DiskUsageView = {
  name: string;
  mount_point: string;
  total_bytes: number;
  available_bytes: number;
  used_bytes: number;
  usage_ratio: number;
};

export type TemperatureView = {
  label: string;
  temperature_celsius: number | null;
};

export type GpuView = {
  name: string;
  utilization_percent: number | null;
  memory_used_mb: number | null;
  memory_total_mb: number | null;
  temperature_celsius: number | null;
};

export type SystemSnapshot = {
  collected_at: string;
  cpu_usage_percent: number;
  cpu_count: number;
  load_average: LoadAverageView;
  total_memory_bytes: number;
  used_memory_bytes: number;
  available_memory_bytes: number;
  disks: DiskUsageView[];
  temperatures: TemperatureView[];
  gpus: GpuView[];
};
