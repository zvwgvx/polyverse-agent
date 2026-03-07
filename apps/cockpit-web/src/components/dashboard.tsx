"use client";

import { FormEvent, useEffect, useMemo, useState } from "react";
import type {
  CockpitEventView,
  CockpitOverview,
  EpisodicOverview,
  ManualPatchRequest,
  MemoryMessage,
  MemoryOverview,
  PromptDocument,
  PromptEntry,
  PromptUpdateRequest,
  RelationshipEdge,
  RelationshipGraphSnapshot,
  StateDeltaLog,
  StateRow,
  SystemSnapshot
} from "@/lib/types";

type LoadState = "idle" | "loading" | "ready" | "error";
type ViewId = "overview" | "metrics" | "memory" | "episodic" | "graph" | "prompts" | "state";
type GraphLayoutMode = "radial" | "focus";

type GraphPosition = {
  x: number;
  y: number;
};

type RelationshipPair = {
  id: string;
  nodeA: string;
  nodeB: string;
  forwardEdges: RelationshipEdge[];
  reverseEdges: RelationshipEdge[];
  forwardStrength: number;
  reverseStrength: number;
  strength: number;
};

type MetricChartProps = {
  title: string;
  note: string;
  values: number[];
  formatter: (value: number) => string;
  ceiling?: number;
};

type LoadLiveOptions = {
  overview?: boolean;
  states?: boolean;
  events?: boolean;
  prompts?: boolean;
  history?: boolean;
  memory?: boolean;
  relationships?: boolean;
  system?: boolean;
};

const NAV_ITEMS: Array<{ id: ViewId; label: string }> = [
  { id: "overview", label: "Overview" },
  { id: "metrics", label: "Metrics" },
  { id: "memory", label: "Memory" },
  { id: "episodic", label: "Episodic" },
  { id: "graph", label: "Graph" },
  { id: "prompts", label: "Prompts" },
  { id: "state", label: "State" }
];

const GRAPH_KIND_META = [
  { id: "social", label: "Social" },
  { id: "illusion", label: "Illusion" },
  { id: "observed_dynamic", label: "Observed" }
] as const;

async function fetchJson<T>(path: string): Promise<T> {
  const response = await fetch(path, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) {
    return "0 B";
  }

  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let index = 0;

  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }

  return `${value.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatAgo(iso: string): string {
  const timestamp = new Date(iso).getTime();
  if (Number.isNaN(timestamp)) {
    return iso;
  }

  const diff = Math.max(0, Date.now() - timestamp);
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) {
    return `${seconds}s ago`;
  }

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m ago`;
  }

  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}h ago`;
  }

  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function formatEdgeSummary(edge: RelationshipEdge): string {
  if (edge.kind === "observed_dynamic") {
    return `tension ${edge.tension?.toFixed(2) ?? "0.00"}`;
  }

  return [
    `aff ${edge.affinity?.toFixed(2) ?? "0.00"}`,
    `att ${edge.attachment?.toFixed(2) ?? "0.00"}`,
    `tr ${edge.trust?.toFixed(2) ?? "0.00"}`
  ].join(" | ");
}

function relationshipStrength(edge: RelationshipEdge): number {
  const values = [edge.affinity, edge.attachment, edge.trust, edge.safety]
    .filter((value): value is number => typeof value === "number")
    .map((value) => Math.abs(value));

  if (values.length === 0) {
    return Math.abs(edge.tension ?? 0);
  }

  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function renderSpeaker(message: MemoryMessage): string {
  return message.username || "assistant";
}

function average(values: number[]): number {
  if (values.length === 0) {
    return 0;
  }

  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function buildRelationshipPairs(edges: RelationshipEdge[]): RelationshipPair[] {
  const buckets = new Map<
    string,
    { nodeA: string; nodeB: string; forwardEdges: RelationshipEdge[]; reverseEdges: RelationshipEdge[] }
  >();

  for (const edge of edges) {
    const [nodeA, nodeB] = [edge.source, edge.target].sort();
    const id = `${nodeA}::${nodeB}`;
    const bucket = buckets.get(id) ?? {
      nodeA,
      nodeB,
      forwardEdges: [],
      reverseEdges: []
    };

    if (edge.source === nodeA) {
      bucket.forwardEdges.push(edge);
    } else {
      bucket.reverseEdges.push(edge);
    }

    buckets.set(id, bucket);
  }

  return Array.from(buckets.values())
    .map((bucket) => {
      const forwardStrength = average(bucket.forwardEdges.map(relationshipStrength));
      const reverseStrength = average(bucket.reverseEdges.map(relationshipStrength));

      return {
        id: `${bucket.nodeA}::${bucket.nodeB}`,
        nodeA: bucket.nodeA,
        nodeB: bucket.nodeB,
        forwardEdges: bucket.forwardEdges.sort((left, right) => left.kind.localeCompare(right.kind)),
        reverseEdges: bucket.reverseEdges.sort((left, right) => left.kind.localeCompare(right.kind)),
        forwardStrength,
        reverseStrength,
        strength: Math.max(forwardStrength, reverseStrength)
      };
    })
    .sort((left, right) => right.strength - left.strength || left.id.localeCompare(right.id));
}

function formatSigned(value: number | null | undefined): string {
  if (typeof value !== "number" || Number.isNaN(value)) {
    return "n/a";
  }

  return `${value >= 0 ? "+" : ""}${value.toFixed(2)}`;
}

function relationshipMetrics(edge: RelationshipEdge): Array<{ label: string; value: number | null | undefined }> {
  if (edge.kind === "observed_dynamic") {
    return [{ label: "tension", value: edge.tension }];
  }

  return [
    { label: "affinity", value: edge.affinity },
    { label: "attachment", value: edge.attachment },
    { label: "trust", value: edge.trust },
    { label: "safety", value: edge.safety },
    { label: "tension", value: edge.tension }
  ];
}

function nodeLabel(
  nodeId: string,
  nodeMap: Map<string, RelationshipGraphSnapshot["nodes"][number]>
): string {
  return nodeMap.get(nodeId)?.label ?? nodeId.split(":").pop() ?? nodeId;
}

function pairCoverage(pair: RelationshipPair): string {
  if (pair.forwardEdges.length > 0 && pair.reverseEdges.length > 0) {
    return "bidirectional";
  }

  return "one-way";
}

function pairImbalance(
  pair: RelationshipPair,
  nodeMap: Map<string, RelationshipGraphSnapshot["nodes"][number]>
): string {
  const delta = Math.abs(pair.forwardStrength - pair.reverseStrength);
  if (delta < 0.08) {
    return "balanced";
  }

  if (pair.forwardStrength > pair.reverseStrength) {
    return `${nodeLabel(pair.nodeA, nodeMap)} leads`;
  }

  return `${nodeLabel(pair.nodeB, nodeMap)} leads`;
}

function toRatio(numerator: number, denominator: number): number {
  if (denominator <= 0) {
    return 0;
  }

  return (numerator / denominator) * 100;
}

function buildChartPoints(values: number[], width: number, height: number, ceiling: number) {
  if (values.length === 0) {
    return [] as Array<{ x: number; y: number }>;
  }

  const maxValue = Math.max(ceiling, 1);
  const step = values.length > 1 ? width / (values.length - 1) : 0;

  return values.map((value, index) => ({
    x: index * step,
    y: height - (Math.min(value, maxValue) / maxValue) * height
  }));
}

function buildLinePath(points: Array<{ x: number; y: number }>): string {
  if (points.length === 0) {
    return "";
  }

  return points
    .map((point, index) => `${index === 0 ? "M" : "L"} ${point.x.toFixed(2)} ${point.y.toFixed(2)}`)
    .join(" ");
}

function buildAreaPath(points: Array<{ x: number; y: number }>, height: number): string {
  if (points.length === 0) {
    return "";
  }

  const line = buildLinePath(points);
  const first = points[0];
  const last = points[points.length - 1];
  return `${line} L ${last.x.toFixed(2)} ${height} L ${first.x.toFixed(2)} ${height} Z`;
}

function isPageVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

function curveDirection(id: string): number {
  let total = 0;
  for (const ch of id) {
    total += ch.charCodeAt(0);
  }
  return total % 2 === 0 ? 1 : -1;
}

function buildCurvePath(
  source: GraphPosition,
  target: GraphPosition,
  pairId: string,
  force: number
): string {
  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const length = Math.hypot(dx, dy) || 1;
  const midpoint = {
    x: (source.x + target.x) / 2,
    y: (source.y + target.y) / 2
  };
  const normal = {
    x: (-dy / length) * force * curveDirection(pairId),
    y: (dx / length) * force * curveDirection(pairId)
  };

  return `M ${source.x.toFixed(2)} ${source.y.toFixed(2)} Q ${(midpoint.x + normal.x).toFixed(2)} ${(midpoint.y + normal.y).toFixed(2)} ${target.x.toFixed(2)} ${target.y.toFixed(2)}`;
}

function MetricChart({ title, note, values, formatter, ceiling }: MetricChartProps) {
  const width = 320;
  const height = 124;
  const maxObserved = values.length > 0 ? Math.max(...values) : 0;
  const chartCeiling = ceiling ?? Math.max(maxObserved, 1);
  const points = buildChartPoints(values, width, height, chartCeiling);
  const latest = values.length > 0 ? values[values.length - 1] : 0;

  return (
    <article className="chart-card">
      <div className="chart-head">
        <div>
          <div className="panel-kicker">Live metric</div>
          <h3>{title}</h3>
        </div>
        <div className="chart-value">{formatter(latest)}</div>
      </div>
      <div className="chart-meta">{note}</div>
      <svg viewBox={`0 0 ${width} ${height}`} className="chart-svg" aria-hidden="true">
        {[0.25, 0.5, 0.75].map((ratio) => (
          <line
            key={ratio}
            x1="0"
            y1={height * ratio}
            x2={width}
            y2={height * ratio}
            className="chart-grid-line"
          />
        ))}
        {points.length > 1 ? <path d={buildAreaPath(points, height)} className="chart-area" /> : null}
        {points.length > 1 ? <path d={buildLinePath(points)} className="chart-line" /> : null}
      </svg>
      <div className="chart-foot">
        <span>{values.length} samples</span>
        <span>peak {formatter(maxObserved)}</span>
      </div>
    </article>
  );
}

export function Dashboard() {
  const [overview, setOverview] = useState<CockpitOverview | null>(null);
  const [states, setStates] = useState<StateRow[]>([]);
  const [events, setEvents] = useState<CockpitEventView[]>([]);
  const [prompts, setPrompts] = useState<PromptEntry[]>([]);
  const [history, setHistory] = useState<StateDeltaLog[]>([]);
  const [memory, setMemory] = useState<MemoryOverview | null>(null);
  const [episodic, setEpisodic] = useState<EpisodicOverview | null>(null);
  const [relationships, setRelationships] = useState<RelationshipGraphSnapshot | null>(null);
  const [system, setSystem] = useState<SystemSnapshot | null>(null);
  const [systemHistory, setSystemHistory] = useState<SystemSnapshot[]>([]);
  const [loadState, setLoadState] = useState<LoadState>("idle");
  const [error, setError] = useState<string>("");
  const [activeView, setActiveView] = useState<ViewId>("overview");
  const [selectedPromptId, setSelectedPromptId] = useState<string>("");
  const [promptDocument, setPromptDocument] = useState<PromptDocument | null>(null);
  const [promptDraft, setPromptDraft] = useState<string>("");
  const [promptBusy, setPromptBusy] = useState<boolean>(false);
  const [promptStatus, setPromptStatus] = useState<string>("");

  const [filterDomain, setFilterDomain] = useState<string>("all");
  const [search, setSearch] = useState<string>("");

  const [patchDimension, setPatchDimension] = useState<string>("");
  const [patchValue, setPatchValue] = useState<string>("0");
  const [patchReason, setPatchReason] = useState<string>("manual tuning");
  const [patchActor, setPatchActor] = useState<string>("owner");
  const [patchBusy, setPatchBusy] = useState<boolean>(false);
  const [selectedPairId, setSelectedPairId] = useState<string>("");
  const [hoveredPairId, setHoveredPairId] = useState<string>("");
  const [graphLayout, setGraphLayout] = useState<GraphLayoutMode>("radial");
  const [graphMinStrength, setGraphMinStrength] = useState<number>(0.08);
  const [graphZoom, setGraphZoom] = useState<number>(1);
  const [graphSearch, setGraphSearch] = useState<string>("");
  const [enabledKinds, setEnabledKinds] = useState<Record<string, boolean>>({
    social: true,
    illusion: true,
    observed_dynamic: true
  });

  const domains = useMemo(() => {
    const values = Array.from(new Set(states.map((state) => state.domain))).sort();
    return ["all", ...values];
  }, [states]);

  const filteredStates = useMemo(() => {
    const q = search.trim().toLowerCase();
    return states.filter((item) => {
      const matchDomain = filterDomain === "all" || item.domain === filterDomain;
      const matchSearch =
        q.length === 0 ||
        item.id.toLowerCase().includes(q) ||
        item.description.toLowerCase().includes(q);
      return matchDomain && matchSearch;
    });
  }, [states, filterDomain, search]);

  const relationshipNodeMap = useMemo(
    () => new Map((relationships?.nodes ?? []).map((node) => [node.id, node])),
    [relationships]
  );
  const normalizedGraphSearch = graphSearch.trim().toLowerCase();

  const graphPairs = useMemo(
    () =>
      buildRelationshipPairs(
        (relationships?.edges ?? []).filter((edge) => enabledKinds[edge.kind] ?? true)
      ),
    [relationships, enabledKinds]
  );

  const selectedPair = useMemo(
    () => graphPairs.find((pair) => pair.id === selectedPairId) ?? graphPairs[0] ?? null,
    [graphPairs, selectedPairId]
  );

  const visiblePairs = useMemo(
    () =>
      graphPairs.filter((pair) => {
        if (pair.id === selectedPair?.id) {
          return true;
        }

        if (pair.strength < graphMinStrength) {
          return false;
        }

        if (!normalizedGraphSearch) {
          return true;
        }

        const searchable = [
          nodeLabel(pair.nodeA, relationshipNodeMap),
          nodeLabel(pair.nodeB, relationshipNodeMap),
          pair.forwardEdges.map((edge) => edge.kind).join(" "),
          pair.reverseEdges.map((edge) => edge.kind).join(" ")
        ]
          .join(" ")
          .toLowerCase();

        return searchable.includes(normalizedGraphSearch);
      }),
    [graphMinStrength, graphPairs, normalizedGraphSearch, relationshipNodeMap, selectedPair]
  );

  const visibleNodes = useMemo(() => {
    const ids = new Set<string>();
    visiblePairs.forEach((pair) => {
      ids.add(pair.nodeA);
      ids.add(pair.nodeB);
    });

    if (selectedPair) {
      ids.add(selectedPair.nodeA);
      ids.add(selectedPair.nodeB);
    }

    if (relationships?.self_node_id && relationshipNodeMap.has(relationships.self_node_id)) {
      ids.add(relationships.self_node_id);
    }

    return (relationships?.nodes ?? []).filter((node) => ids.has(node.id));
  }, [relationships, relationshipNodeMap, selectedPair, visiblePairs]);

  const graphPositions = useMemo(() => {
    const nodes = visibleNodes;
    const positions = new Map<string, GraphPosition>();
    const width = 820;
    const height = 460;
    const center = { x: width / 2, y: height / 2 };
    if (graphLayout === "focus" && selectedPair) {
      positions.set(selectedPair.nodeA, { x: center.x - 120, y: center.y });
      positions.set(selectedPair.nodeB, { x: center.x + 120, y: center.y });

      const orbitNodes = nodes.filter(
        (node) => node.id !== selectedPair.nodeA && node.id !== selectedPair.nodeB
      );

      orbitNodes.forEach((node, index) => {
        const angle = -Math.PI / 2 + (Math.PI * 2 * index) / Math.max(orbitNodes.length, 1);
        const radiusX = orbitNodes.length > 4 ? 275 : 235;
        const radiusY = orbitNodes.length > 4 ? 162 : 140;
        positions.set(node.id, {
          x: center.x + Math.cos(angle) * radiusX,
          y: center.y + Math.sin(angle) * radiusY
        });
      });
    } else {
      const anchor =
        nodes.find((node) => node.id === relationships?.self_node_id) ?? nodes[0];
      if (anchor) {
        positions.set(anchor.id, center);
      }

      const orbitNodes = nodes.filter((node) => node.id !== anchor?.id);
      orbitNodes.forEach((node, index) => {
        const angle = (Math.PI * 2 * index) / Math.max(orbitNodes.length, 1);
        const radius = orbitNodes.length <= 4 ? 162 : 190;
        positions.set(node.id, {
          x: center.x + Math.cos(angle) * radius,
          y: center.y + Math.sin(angle) * radius
        });
      });
    }

    return { width, height, center, positions };
  }, [graphLayout, relationships?.self_node_id, selectedPair, visibleNodes]);

  const primaryDisk = useMemo(() => {
    const disks = system?.disks ?? [];
    if (disks.length === 0) {
      return null;
    }

    return [...disks].sort((left, right) => right.usage_ratio - left.usage_ratio)[0];
  }, [system]);

  const workerHealth = useMemo(() => {
    const workers = overview?.workers ?? [];
    return workers.filter((worker) => !worker.status.startsWith("error") && worker.status !== "stopped").length;
  }, [overview]);

  const cpuHistory = useMemo(() => systemHistory.map((sample) => sample.cpu_usage_percent), [systemHistory]);
  const memoryRatioHistory = useMemo(
    () => systemHistory.map((sample) => toRatio(sample.used_memory_bytes, sample.total_memory_bytes)),
    [systemHistory]
  );
  const loadHistory = useMemo(() => systemHistory.map((sample) => sample.load_average.one), [systemHistory]);
  const diskHistory = useMemo(() => {
    if (!primaryDisk) {
      return systemHistory.map(() => 0);
    }

    return systemHistory.map((sample) => {
      const matched = sample.disks.find((disk) => disk.mount_point === primaryDisk.mount_point);
      return matched ? matched.usage_ratio * 100 : 0;
    });
  }, [primaryDisk, systemHistory]);

  const promptDirty = useMemo(
    () => promptDocument != null && promptDraft !== promptDocument.content,
    [promptDocument, promptDraft]
  );

  async function loadLive(options: LoadLiveOptions) {
    setLoadState((prev) => (prev === "idle" ? "loading" : prev));
    setError("");

    try {
      const tasks: Promise<void>[] = [];

      if (options.overview) {
        tasks.push(
          fetchJson<CockpitOverview>("/api/cockpit/overview").then((overviewData) => {
            setOverview(overviewData);
          })
        );
      }

      if (options.states) {
        tasks.push(
          fetchJson<StateRow[]>("/api/cockpit/states").then((stateData) => {
            setStates(stateData);
            if (!patchDimension && stateData.length > 0) {
              setPatchDimension(stateData[0].id);
            }
          })
        );
      }

      if (options.events) {
        tasks.push(
          fetchJson<CockpitEventView[]>("/api/cockpit/events?limit=80").then((eventData) => {
            setEvents(eventData);
          })
        );
      }

      if (options.prompts) {
        tasks.push(
          fetchJson<PromptEntry[]>("/api/cockpit/prompts").then((promptData) => {
            setPrompts(promptData);
            if (!selectedPromptId && promptData.length > 0) {
              setSelectedPromptId(promptData[0].id);
            }
          })
        );
      }

      if (options.history) {
        tasks.push(
          fetchJson<StateDeltaLog[]>("/api/cockpit/states/history?limit=120").then((historyData) => {
            setHistory(historyData);
          })
        );
      }

      if (options.memory) {
        tasks.push(
          fetchJson<MemoryOverview>("/api/cockpit/memory?limit=36").then((memoryData) => {
            setMemory(memoryData);
          })
        );
      }

      if (options.relationships) {
        tasks.push(
          fetchJson<RelationshipGraphSnapshot>("/api/cockpit/relationships").then((relationshipData) => {
            setRelationships(relationshipData);
          })
        );
      }

      if (options.system) {
        tasks.push(
          fetchJson<SystemSnapshot>("/api/cockpit/system").then((systemData) => {
            setSystem(systemData);
            setSystemHistory((prev) => {
              const last = prev.length > 0 ? prev[prev.length - 1] : null;
              if (last?.collected_at === systemData.collected_at) {
                return prev;
              }
              const next = [...prev, systemData];
              return next.slice(-120);
            });
          })
        );
      }

      await Promise.all(tasks);
      setLoadState("ready");
    } catch (err) {
      setLoadState("error");
      setError(err instanceof Error ? err.message : "Unknown error");
    }
  }

  async function loadEpisodic() {
    try {
      const episodicData = await fetchJson<EpisodicOverview>("/api/cockpit/episodic?limit=48");
      setEpisodic(episodicData);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load episodic memory");
    }
  }

  async function loadPromptDocument(id: string) {
    if (!id) {
      setPromptDocument(null);
      setPromptDraft("");
      return;
    }

    setPromptBusy(true);
    try {
      const url = `/api/cockpit/prompts/document?id=${encodeURIComponent(id)}`;
      const document = await fetchJson<PromptDocument>(url);
      setPromptDocument(document);
      setPromptDraft(document.content);
      setPromptStatus(`Loaded ${document.id}`);
    } catch (err) {
      setPromptStatus("");
      setError(err instanceof Error ? err.message : "Failed to load prompt");
    } finally {
      setPromptBusy(false);
    }
  }

  async function savePromptDocument() {
    if (!selectedPromptId) {
      return;
    }

    const payload: PromptUpdateRequest = {
      id: selectedPromptId,
      content: promptDraft
    };

    setPromptBusy(true);
    setError("");
    try {
      const response = await fetch("/api/cockpit/prompts/update", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify(payload)
      });

      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || "Prompt save failed");
      }

      const saved = (await response.json()) as PromptDocument;
      setPromptDocument(saved);
      setPromptDraft(saved.content);
      setPromptStatus(`Saved ${saved.id}`);
      await loadLive({ overview: true, prompts: true });
    } catch (err) {
      setPromptStatus("");
      setError(err instanceof Error ? err.message : "Prompt save failed");
    } finally {
      setPromptBusy(false);
    }
  }

  async function applyPatch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const numeric = Number(patchValue);
    if (!Number.isFinite(numeric)) {
      setError("Patch value must be a valid number.");
      return;
    }

    const payload: ManualPatchRequest = {
      dimension_id: patchDimension,
      value: numeric,
      reason: patchReason,
      actor: patchActor
    };

    setPatchBusy(true);
    setError("");
    try {
      const response = await fetch("/api/cockpit/state/patch", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify(payload)
      });

      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || "Patch failed");
      }

      await loadLive({ overview: true, states: true, history: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Patch failed");
    } finally {
      setPatchBusy(false);
    }
  }

  async function refreshActiveView(view: ViewId = activeView) {
    switch (view) {
      case "overview":
        await loadLive({ overview: true, events: true, states: true });
        break;
      case "metrics":
        await loadLive({ overview: true, system: true });
        break;
      case "memory":
        await loadLive({ overview: true, memory: true });
        break;
      case "episodic":
        await loadLive({ overview: true });
        await loadEpisodic();
        break;
      case "graph":
        await loadLive({ overview: true, relationships: true });
        break;
      case "prompts":
        await loadLive({ overview: true, prompts: true });
        break;
      case "state":
        await loadLive({ overview: true, states: true, history: true });
        break;
    }
  }

  useEffect(() => {
    void loadLive({
      overview: true,
      states: true,
      events: true,
      prompts: true,
      history: true,
      memory: true,
      system: true
    });
  }, []);

  useEffect(() => {
    void refreshActiveView(activeView);

    const intervalMs =
      activeView === "overview" || activeView === "metrics"
        ? 1000
        : activeView === "episodic" || activeView === "prompts"
          ? 15000
          : 2000;

    const timer = setInterval(() => {
      if (!isPageVisible()) {
        return;
      }
      void refreshActiveView(activeView);
    }, intervalMs);

    return () => clearInterval(timer);
  }, [activeView]);

  useEffect(() => {
    if (activeView !== "prompts" || !selectedPromptId) {
      return;
    }

    void loadPromptDocument(selectedPromptId);
  }, [activeView, selectedPromptId]);

  useEffect(() => {
    if (graphPairs.length === 0) {
      if (selectedPairId) {
        setSelectedPairId("");
      }
      return;
    }

    if (!graphPairs.some((pair) => pair.id === selectedPairId)) {
      setSelectedPairId(graphPairs[0].id);
    }
  }, [graphPairs, selectedPairId]);

  return (
    <main className="page-shell">
      <div className="cockpit-layout">
        <aside className="sidebar">
          <section className="sidebar-panel sidebar-brand">
            <div className="panel-kicker">Built-in cockpit</div>
            <h1>{overview?.identity.display_name ?? "Polyverse Agent Console"}</h1>
            <p>
              Operational surface for runtime, memory, graph, and state control.
              {overview?.identity.agent_id ? ` Profile: ${overview.identity.agent_id}.` : ""}
            </p>
          </section>

          <nav className="sidebar-panel sidebar-nav" aria-label="Cockpit sections">
            {NAV_ITEMS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={`toolbar-button ${activeView === item.id ? "is-active" : ""}`}
                onClick={() => setActiveView(item.id)}
              >
                {item.label}
              </button>
            ))}
          </nav>

          <section className="sidebar-panel sidebar-meta">
            <div className="sidebar-meta-row">
              <span>Status</span>
              <span className={`status-pill status-${loadState}`}>{loadState}</span>
            </div>
            <div className="sidebar-meta-row">
              <span>Sample</span>
              <strong>{system ? formatAgo(system.collected_at) : "-"}</strong>
            </div>
            <div className="sidebar-meta-row">
              <span>Workers</span>
              <strong>{workerHealth}/{overview?.workers.length ?? 0}</strong>
            </div>
            <div className="sidebar-meta-row">
              <span>Memory</span>
              <strong>{memory?.persisted_message_count ?? 0}</strong>
            </div>
            <div className="sidebar-meta-row">
              <span>Episodic</span>
              <strong>{episodic?.total_chunks ?? 0}</strong>
            </div>
          </section>
        </aside>

        <section className="content-shell">
          <header className="hero">
            <div className="hero-head">
              <div className="hero-copy">
                <div className="panel-kicker">Live command deck</div>
                <h2>{NAV_ITEMS.find((item) => item.id === activeView)?.label ?? "Overview"}</h2>
                <p>
                  {overview?.identity.display_name ?? "This agent"} local operational surface for runtime health, live metrics, memory layers, relationship graph, and state tuning.
                </p>
              </div>
              <div className="hero-actions">
                <button type="button" onClick={() => void refreshActiveView()}>
                  Refresh live data
                </button>
              </div>
            </div>
          </header>

          {error ? <div className="error-box">{error}</div> : null}

          <section className="summary-grid">
            <article className="summary-card">
              <div className="summary-label">Workers online</div>
              <div className="summary-value">{workerHealth}</div>
              <div className="summary-foot">{overview?.workers.length ?? 0} registered</div>
            </article>
            <article className="summary-card">
              <div className="summary-label">CPU</div>
              <div className="summary-value">{system?.cpu_usage_percent.toFixed(1) ?? "0.0"}%</div>
              <div className="summary-foot">sample {system ? formatAgo(system.collected_at) : "-"}</div>
            </article>
            <article className="summary-card">
              <div className="summary-label">Agent ID</div>
              <div className="summary-value">{overview?.identity.agent_id ?? "-"}</div>
              <div className="summary-foot">runtime profile</div>
            </article>
            <article className="summary-card">
              <div className="summary-label">Episodic chunks</div>
              <div className="summary-value">{episodic?.total_chunks ?? 0}</div>
              <div className="summary-foot">RAG entries</div>
            </article>
          </section>

      {activeView === "overview" ? (
        <>
          <section className="split-grid">
            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Runtime</div>
                  <h2>Overview</h2>
                </div>
                <div className="section-note">1-second live refresh</div>
              </div>
              <div className="stat-inline-grid">
                <div className="metric-row">
                  <span>Raw events</span>
                  <strong>{overview?.counters.raw_events ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>Responses</span>
                  <strong>{overview?.counters.response_events ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>Mentions</span>
                  <strong>{overview?.counters.mention_events ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>Bot turns</span>
                  <strong>{overview?.counters.bot_turns ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>Intents</span>
                  <strong>{overview?.counters.intent_events ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>Biology</span>
                  <strong>{overview?.counters.biology_events ?? 0}</strong>
                </div>
              </div>
              <div className="worker-list">
                {(overview?.workers ?? []).map((worker) => (
                  <div key={worker.name} className="worker-item">
                    <div className="mono">{worker.name}</div>
                    <span className={`worker-pill ${worker.status.startsWith("error") ? "is-error" : worker.status === "stopped" ? "is-muted" : "is-ok"}`}>
                      {worker.status}
                    </span>
                  </div>
                ))}
              </div>
            </article>

            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Trace</div>
                  <h2>Recent Events</h2>
                </div>
              </div>
              <div className="scroll-list tall-list">
                {events.map((entry, index) => (
                  <div key={`${entry.ts}-${index}`} className="history-item">
                    <div className="mono">[{entry.kind}]</div>
                    <div>{entry.summary}</div>
                    <div className="muted">{formatAgo(entry.ts)}</div>
                  </div>
                ))}
              </div>
            </article>
          </section>

          <section className="split-grid">
            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Registry</div>
                  <h2>Prompt Inventory</h2>
                </div>
              </div>
              <div className="prompt-grid">
                {prompts.map((prompt) => (
                  <div key={prompt.id} className="prompt-item">
                    <div className="mono">{prompt.id}</div>
                    <div>{prompt.path}</div>
                  </div>
                ))}
              </div>
            </article>

            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">System</div>
                  <h2>Current Snapshot</h2>
                </div>
              </div>
              <div className="metric-stack">
                <div className="metric-row">
                  <span>Memory</span>
                  <strong className="mono">
                    {system
                      ? `${formatBytes(system.used_memory_bytes)} / ${formatBytes(system.total_memory_bytes)}`
                      : "0 B / 0 B"}
                  </strong>
                </div>
                <div className="metric-row">
                  <span>Load avg</span>
                  <strong className="mono">
                    {system
                      ? `${system.load_average.one.toFixed(2)} / ${system.load_average.five.toFixed(2)} / ${system.load_average.fifteen.toFixed(2)}`
                      : "0.00 / 0.00 / 0.00"}
                  </strong>
                </div>
                <div className="metric-row">
                  <span>Primary disk</span>
                  <strong className="mono">
                    {primaryDisk ? `${primaryDisk.mount_point} ${(primaryDisk.usage_ratio * 100).toFixed(1)}%` : "n/a"}
                  </strong>
                </div>
                <div className="metric-row">
                  <span>Sensors</span>
                  <strong>{system?.temperatures.length ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>GPU devices</span>
                  <strong>{system?.gpus.length ?? 0}</strong>
                </div>
              </div>
            </article>
          </section>
        </>
      ) : null}

      {activeView === "metrics" ? (
        <>
          <section className="panel">
            <div className="panel-head">
              <div>
                <div className="panel-kicker">dstat-style live charts</div>
                <h2>Metrics</h2>
              </div>
              <div className="section-note">rolling window: {systemHistory.length} seconds</div>
            </div>
            <div className="chart-grid">
              <MetricChart
                title="CPU usage"
                note="global utilization"
                values={cpuHistory}
                formatter={(value) => `${value.toFixed(1)}%`}
                ceiling={100}
              />
              <MetricChart
                title="RAM usage"
                note="used / total memory"
                values={memoryRatioHistory}
                formatter={(value) => `${value.toFixed(1)}%`}
                ceiling={100}
              />
              <MetricChart
                title="Load average"
                note="1-minute load"
                values={loadHistory}
                formatter={(value) => value.toFixed(2)}
              />
              <MetricChart
                title="Disk pressure"
                note={primaryDisk ? primaryDisk.mount_point : "highest used mount"}
                values={diskHistory}
                formatter={(value) => `${value.toFixed(1)}%`}
                ceiling={100}
              />
            </div>
          </section>

          <section className="split-grid">
            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Resources</div>
                  <h2>Live System Detail</h2>
                </div>
              </div>
              <div className="metric-stack">
                <div className="metric-row">
                  <span>CPU threads</span>
                  <strong>{system?.cpu_count ?? 0}</strong>
                </div>
                <div className="metric-row">
                  <span>Available RAM</span>
                  <strong className="mono">{system ? formatBytes(system.available_memory_bytes) : "0 B"}</strong>
                </div>
              </div>
              <div className="table-shell compact-shell">
                <table>
                  <thead>
                    <tr>
                      <th>Disk</th>
                      <th>Used</th>
                      <th>Free</th>
                      <th>Ratio</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(system?.disks ?? []).map((disk) => (
                      <tr key={`${disk.name}-${disk.mount_point}`}>
                        <td className="mono">{disk.mount_point || disk.name}</td>
                        <td>{formatBytes(disk.used_bytes)}</td>
                        <td>{formatBytes(disk.available_bytes)}</td>
                        <td>{(disk.usage_ratio * 100).toFixed(1)}%</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </article>

            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Sensors</div>
                  <h2>Temperature and GPU</h2>
                </div>
              </div>
              <div className="scroll-list tall-list">
                {(system?.temperatures ?? []).length ? (
                  (system?.temperatures ?? []).map((sensor) => (
                    <div key={sensor.label} className="history-item">
                      <div className="mono">{sensor.label}</div>
                      <div>{sensor.temperature_celsius == null ? "n/a" : `${sensor.temperature_celsius.toFixed(1)} C`}</div>
                    </div>
                  ))
                ) : (
                  <div className="history-item">No temperature sensors available.</div>
                )}
                {(system?.gpus ?? []).map((gpu) => (
                  <div key={gpu.name} className="history-item">
                    <div className="mono">{gpu.name}</div>
                    <div>util {gpu.utilization_percent?.toFixed(1) ?? "n/a"}%</div>
                    <div>mem {gpu.memory_used_mb ?? 0}/{gpu.memory_total_mb ?? 0} MB</div>
                    <div>temp {gpu.temperature_celsius?.toFixed(1) ?? "n/a"} C</div>
                  </div>
                ))}
              </div>
            </article>
          </section>
        </>
      ) : null}

      {activeView === "memory" ? (
        <section className="split-grid">
          <article className="panel">
            <div className="panel-head">
              <div>
                <div className="panel-kicker">Working memory</div>
                <h2>Short-term and active sessions</h2>
              </div>
            </div>
            <div className="metric-stack">
              <div className="metric-row">
                <span>Active sessions</span>
                <strong>{memory?.active_session_count ?? 0}</strong>
              </div>
              <div className="metric-row">
                <span>Recent short-term messages</span>
                <strong>{memory?.active_recent_messages.length ?? 0}</strong>
              </div>
            </div>
            <div className="subpanel-grid">
              <div>
                <h3>Active Sessions</h3>
                <div className="scroll-list compact-list">
                  {(memory?.active_sessions ?? []).map((session) => (
                    <div key={session.conversation} className="history-item">
                      <div className="mono">{session.conversation}</div>
                      <div>{session.participants.join(", ") || "no participants"}</div>
                      <div>{session.message_count} messages</div>
                      <div className="muted">active {formatAgo(session.last_active)}</div>
                    </div>
                  ))}
                </div>
              </div>
              <div>
                <h3>Recent Short-Term</h3>
                <div className="scroll-list compact-list">
                  {(memory?.active_recent_messages ?? []).map((message) => (
                    <div key={message.id} className="history-item">
                      <div className="mono">{renderSpeaker(message)}</div>
                      <div>{message.content}</div>
                      <div className="muted">{message.channel_id} · {formatAgo(message.timestamp)}</div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </article>

          <article className="panel">
            <div className="panel-head">
              <div>
                <div className="panel-kicker">SQL memory</div>
                <h2>Persisted chat history</h2>
              </div>
              <div className="section-note">{memory?.persisted_message_count ?? 0} rows</div>
            </div>
            <div className="scroll-list tall-list">
              {(memory?.persisted_recent_messages ?? []).map((message) => (
                <div key={message.id} className="history-item">
                  <div className="mono">{renderSpeaker(message)}</div>
                  <div>{message.content}</div>
                  <div className="muted">{message.channel_id} · {formatAgo(message.timestamp)}</div>
                </div>
              ))}
            </div>
          </article>
        </section>
      ) : null}

      {activeView === "episodic" ? (
        <section className="panel">
          <div className="panel-head">
            <div>
              <div className="panel-kicker">RAG table</div>
              <h2>Episodic Memory</h2>
            </div>
            <div className="section-note">{episodic?.total_chunks ?? 0} total chunks</div>
          </div>
          <div className="table-shell">
            <table>
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Importance</th>
                  <th>Content</th>
                  <th>Metadata</th>
                </tr>
              </thead>
              <tbody>
                {(episodic?.recent_chunks ?? []).map((chunk) => (
                  <tr key={chunk.id}>
                    <td className="mono">{formatAgo(chunk.timestamp)}</td>
                    <td className="mono">{chunk.importance.toFixed(2)}</td>
                    <td>
                      <div className="content-cell">{chunk.content}</div>
                    </td>
                    <td>
                      <pre className="metadata-box">{chunk.metadata}</pre>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ) : null}

      {activeView === "graph" ? (
        <section className="panel">
          <div className="panel-head">
            <div>
              <div className="panel-kicker">Cognitive graph</div>
              <h2>Relationship Graph</h2>
            </div>
              <div className="graph-caption">
                Filter, focus, and inspect the live relationship surface around {relationships?.self_display_name ?? overview?.identity.display_name ?? "the agent"}.
              </div>
          </div>

          <div className="graph-toolbar">
            <label className="graph-search">
              <span className="graph-toolbar-label">Search node or pair</span>
              <input
                value={graphSearch}
                onChange={(event) => setGraphSearch(event.target.value)}
                placeholder="Search by label or relation kind"
              />
            </label>

            <div className="graph-toolbar-group">
              <span className="graph-toolbar-label">Layout</span>
              <div className="graph-segmented">
                <button
                  type="button"
                  className={`graph-segment ${graphLayout === "focus" ? "is-active" : ""}`}
                  onClick={() => setGraphLayout("focus")}
                >
                  Focus
                </button>
                <button
                  type="button"
                  className={`graph-segment ${graphLayout === "radial" ? "is-active" : ""}`}
                  onClick={() => setGraphLayout("radial")}
                >
                  Radial
                </button>
              </div>
            </div>

            <label className="graph-slider">
              <span className="graph-toolbar-label">Strength floor {graphMinStrength.toFixed(2)}</span>
              <input
                type="range"
                min="0"
                max="1"
                step="0.02"
                value={graphMinStrength}
                onChange={(event) => setGraphMinStrength(Number(event.target.value))}
              />
            </label>

            <label className="graph-slider">
              <span className="graph-toolbar-label">Zoom {graphZoom.toFixed(2)}x</span>
              <input
                type="range"
                min="0.85"
                max="1.35"
                step="0.05"
                value={graphZoom}
                onChange={(event) => setGraphZoom(Number(event.target.value))}
              />
            </label>
          </div>

          <div className="graph-kind-row">
            {GRAPH_KIND_META.map((kind) => (
              <button
                key={kind.id}
                type="button"
                className={`graph-kind-chip ${enabledKinds[kind.id] ? "is-active" : ""}`}
                onClick={() =>
                  setEnabledKinds((prev) => ({
                    ...prev,
                    [kind.id]: !prev[kind.id]
                  }))
                }
              >
                <span>{kind.label}</span>
              </button>
            ))}

            <div className="graph-legend">
              <span>{visiblePairs.length}/{graphPairs.length} pairs shown</span>
              <span>{visibleNodes.length} nodes</span>
            </div>
          </div>

          <div className="graph-layout">
            <div className="graph-shell">
              <svg
                viewBox={`0 0 ${graphPositions.width} ${graphPositions.height}`}
                className="relationship-graph"
                role="img"
                aria-label="Relationship graph"
              >
                <defs>
                  <radialGradient id="graphStageGlow" cx="50%" cy="50%" r="60%">
                    <stop offset="0%" stopColor="rgba(255,255,255,0.08)" />
                    <stop offset="100%" stopColor="rgba(255,255,255,0)" />
                  </radialGradient>
                </defs>

                <g className="graph-backdrop">
                  <rect
                    x="12"
                    y="12"
                    width={graphPositions.width - 24}
                    height={graphPositions.height - 24}
                    rx="22"
                    className="graph-stage"
                  />
                  <circle
                    cx={graphPositions.center.x}
                    cy={graphPositions.center.y}
                    r="180"
                    className="graph-orbit-ring"
                  />
                  <circle
                    cx={graphPositions.center.x}
                    cy={graphPositions.center.y}
                    r="120"
                    className="graph-orbit-ring is-inner"
                  />
                  <circle
                    cx={graphPositions.center.x}
                    cy={graphPositions.center.y}
                    r="210"
                    fill="url(#graphStageGlow)"
                    opacity="0.6"
                  />
                </g>

                <g
                  transform={`translate(${graphPositions.center.x - graphPositions.center.x * graphZoom} ${graphPositions.center.y - graphPositions.center.y * graphZoom}) scale(${graphZoom})`}
                >
                  {visiblePairs.map((pair) => {
                    const source = graphPositions.positions.get(pair.nodeA);
                    const target = graphPositions.positions.get(pair.nodeB);
                    if (!source || !target) {
                      return null;
                    }

                    const selected = selectedPair?.id === pair.id;
                    const hovered = hoveredPairId === pair.id;
                    const midpoint = {
                      x: (source.x + target.x) / 2,
                      y: (source.y + target.y) / 2
                    };
                    const strokeWidth = 2 + pair.strength * 5;
                    const opacity = selected ? 0.96 : hovered ? 0.82 : 0.26 + Math.min(0.32, pair.strength * 0.3);
                    const curvature = selected ? 28 : hovered ? 20 : 14;
                    const path = buildCurvePath(source, target, pair.id, curvature);

                    return (
                      <g
                        key={pair.id}
                        className="graph-pair"
                        role="button"
                        tabIndex={0}
                        aria-label={`Inspect relationship between ${nodeLabel(pair.nodeA, relationshipNodeMap)} and ${nodeLabel(pair.nodeB, relationshipNodeMap)}`}
                        onClick={() => setSelectedPairId(pair.id)}
                        onMouseEnter={() => setHoveredPairId(pair.id)}
                        onMouseLeave={() => setHoveredPairId("")}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            setSelectedPairId(pair.id);
                          }
                        }}
                      >
                        <path d={path} className="graph-hit-path" />
                        <path
                          d={path}
                          className={`graph-connector ${selected ? "is-selected" : hovered ? "is-hovered" : ""}`}
                          strokeWidth={strokeWidth + (selected ? 1.5 : 0)}
                          opacity={opacity}
                          strokeDasharray={
                            pair.forwardEdges.length === 0 || pair.reverseEdges.length === 0 ? "10 10" : undefined
                          }
                        />
                        <circle
                          cx={midpoint.x}
                          cy={midpoint.y}
                          r={selected ? 16 : hovered ? 14 : 12}
                          className={`graph-midpoint ${selected ? "is-selected" : ""}`}
                        />
                        <text
                          x={midpoint.x}
                          y={midpoint.y + 4}
                          textAnchor="middle"
                          className={`graph-badge-label ${selected ? "is-selected" : ""}`}
                        >
                          {pair.forwardEdges.length + pair.reverseEdges.length}
                        </text>
                      </g>
                    );
                  })}

                  {visibleNodes.map((node) => {
                  const point = graphPositions.positions.get(node.id);
                  if (!point) {
                    return null;
                  }

                  const selected =
                    selectedPair != null &&
                    (selectedPair.nodeA === node.id || selectedPair.nodeB === node.id);
                  const isAgent = node.kind === "agent";
                  const linked = visiblePairs.some(
                    (pair) => pair.nodeA === node.id || pair.nodeB === node.id
                  );

                  return (
                    <g
                      key={node.id}
                      transform={`translate(${point.x}, ${point.y})`}
                      className={`graph-node ${selected ? "is-selected" : linked ? "is-linked" : ""}`}
                    >
                      <circle
                        r={isAgent ? 38 : 28}
                        className={`graph-node-halo ${selected ? "is-selected" : ""}`}
                      />
                      <circle
                        r={isAgent ? 30 : 22}
                        className={`graph-node-body ${isAgent ? "is-agent" : ""} ${selected ? "is-selected" : ""}`}
                      />
                      <text
                        textAnchor="middle"
                        dy="0.3em"
                        className="graph-label"
                      >
                        {node.label}
                      </text>
                    </g>
                    );
                  })}
                </g>
              </svg>
            </div>

            <aside className="graph-detail">
              {selectedPair ? (
                <>
                  <div className="detail-header">
                    <div>
                      <div className="detail-kicker">Selected pair</div>
                      <h3>
                        {nodeLabel(selectedPair.nodeA, relationshipNodeMap)} ↔ {" "}
                        {nodeLabel(selectedPair.nodeB, relationshipNodeMap)}
                      </h3>
                    </div>
                    <span className="pair-status">{pairCoverage(selectedPair)}</span>
                  </div>

                  <div className="detail-summary">
                    <div className="detail-metric">
                      <span>Strength</span>
                      <strong>{selectedPair.strength.toFixed(2)}</strong>
                    </div>
                    <div className="detail-metric">
                      <span>Imbalance</span>
                      <strong>{Math.abs(selectedPair.forwardStrength - selectedPair.reverseStrength).toFixed(2)}</strong>
                    </div>
                    <div className="detail-metric">
                      <span>Reading</span>
                      <strong>{pairImbalance(selectedPair, relationshipNodeMap)}</strong>
                    </div>
                  </div>

                  <div className="direction-grid">
                    <section className="direction-card">
                      <div className="direction-head">
                        <h4>
                          {nodeLabel(selectedPair.nodeA, relationshipNodeMap)} → {" "}
                          {nodeLabel(selectedPair.nodeB, relationshipNodeMap)}
                        </h4>
                        <span className="mono">{selectedPair.forwardStrength.toFixed(2)}</span>
                      </div>
                      {selectedPair.forwardEdges.length > 0 ? (
                        selectedPair.forwardEdges.map((edge) => (
                          <div key={edge.id} className="direction-edge">
                            <div className="direction-edge-head">
                              <span>{edge.kind}</span>
                              <strong>{formatEdgeSummary(edge)}</strong>
                            </div>
                            <div className="metric-pills">
                              {relationshipMetrics(edge).map((metric) => (
                                <div key={`${edge.id}-${metric.label}`} className="metric-pill">
                                  <span>{metric.label}</span>
                                  <strong className="mono">{formatSigned(metric.value)}</strong>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))
                      ) : (
                        <div className="direction-empty">No directed edge recorded.</div>
                      )}
                    </section>

                    <section className="direction-card">
                      <div className="direction-head">
                        <h4>
                          {nodeLabel(selectedPair.nodeB, relationshipNodeMap)} → {" "}
                          {nodeLabel(selectedPair.nodeA, relationshipNodeMap)}
                        </h4>
                        <span className="mono">{selectedPair.reverseStrength.toFixed(2)}</span>
                      </div>
                      {selectedPair.reverseEdges.length > 0 ? (
                        selectedPair.reverseEdges.map((edge) => (
                          <div key={edge.id} className="direction-edge">
                            <div className="direction-edge-head">
                              <span>{edge.kind}</span>
                              <strong>{formatEdgeSummary(edge)}</strong>
                            </div>
                            <div className="metric-pills">
                              {relationshipMetrics(edge).map((metric) => (
                                <div key={`${edge.id}-${metric.label}`} className="metric-pill">
                                  <span>{metric.label}</span>
                                  <strong className="mono">{formatSigned(metric.value)}</strong>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))
                      ) : (
                        <div className="direction-empty">No directed edge recorded.</div>
                      )}
                    </section>
                  </div>
                </>
              ) : (
                <div className="graph-detail-empty">No relationship pair available yet.</div>
              )}
            </aside>
          </div>

          <div className="pair-list">
            {visiblePairs.map((pair) => (
              <button
                key={pair.id}
                type="button"
                className={`pair-chip ${selectedPair?.id === pair.id ? "is-selected" : ""}`}
                onClick={() => setSelectedPairId(pair.id)}
              >
                <span>
                  {nodeLabel(pair.nodeA, relationshipNodeMap)} ↔ {nodeLabel(pair.nodeB, relationshipNodeMap)}
                </span>
                <strong className="mono">{pair.strength.toFixed(2)}</strong>
              </button>
            ))}
          </div>
        </section>
      ) : null}

      {activeView === "prompts" ? (
        <section className="split-grid prompt-editor-layout">
          <article className="panel">
            <div className="panel-head">
              <div>
                <div className="panel-kicker">Registry</div>
                <h2>Prompt Files</h2>
              </div>
              <div className="section-note">{prompts.length} items</div>
            </div>
            <div className="scroll-list tall-list">
              {prompts.map((prompt) => (
                <button
                  key={prompt.id}
                  type="button"
                  className={`prompt-list-item ${selectedPromptId === prompt.id ? "is-selected" : ""}`}
                  onClick={() => {
                    setSelectedPromptId(prompt.id);
                    setPromptStatus("");
                  }}
                >
                  <span className="mono">{prompt.id}</span>
                  <span>{prompt.path}</span>
                </button>
              ))}
            </div>
          </article>

          <article className="panel">
            <div className="panel-head">
              <div>
                <div className="panel-kicker">Direct edit</div>
                <h2>{promptDocument?.id ?? "Prompt Editor"}</h2>
              </div>
              <div className="prompt-editor-actions">
                <span className="section-note">{promptStatus || (promptDirty ? "Unsaved changes" : "In sync")}</span>
                <button
                  type="button"
                  onClick={() => void loadPromptDocument(selectedPromptId)}
                  disabled={!selectedPromptId || promptBusy}
                >
                  Reload
                </button>
                <button
                  type="button"
                  onClick={() => void savePromptDocument()}
                  disabled={!selectedPromptId || promptBusy || !promptDirty}
                >
                  {promptBusy ? "Saving..." : "Save Prompt"}
                </button>
              </div>
            </div>

            <div className="metric-stack">
              <div className="metric-row">
                <span>Registry path</span>
                <strong className="mono">{promptDocument?.path ?? "-"}</strong>
              </div>
              <div className="metric-row">
                <span>Current id</span>
                <strong className="mono">{selectedPromptId || "-"}</strong>
              </div>
            </div>

            <label className="prompt-editor">
              <span className="graph-toolbar-label">Prompt content</span>
              <textarea
                value={promptDraft}
                onChange={(event) => {
                  setPromptDraft(event.target.value);
                  setPromptStatus("");
                }}
                placeholder="Select a prompt from the left panel."
                spellCheck={false}
                disabled={!selectedPromptId || promptBusy}
              />
            </label>
          </article>
        </section>
      ) : null}

      {activeView === "state" ? (
        <>
          <section className="panel">
            <div className="panel-head">
              <div>
                <div className="panel-kicker">State contract</div>
                <h2>State Viewer</h2>
              </div>
              <div className="filters">
                <input
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder="Search dimension"
                />
                <select value={filterDomain} onChange={(event) => setFilterDomain(event.target.value)}>
                  {domains.map((domain) => (
                    <option key={domain} value={domain}>
                      {domain}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            <div className="table-shell">
              <table>
                <thead>
                  <tr>
                    <th>ID</th>
                    <th>Domain</th>
                    <th>Mode</th>
                    <th>Value</th>
                    <th>Range</th>
                    <th>Source</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredStates.map((row) => (
                    <tr key={row.id}>
                      <td className="mono">{row.id}</td>
                      <td>{row.domain}</td>
                      <td>{row.update_mode}</td>
                      <td className="mono">{row.value.toFixed(4)}</td>
                      <td className="mono">
                        {row.range_min}..{row.range_max}
                      </td>
                      <td>{row.source}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          <section className="split-grid">
            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Override</div>
                  <h2>Manual Patch</h2>
                </div>
              </div>
              <form className="patch-form" onSubmit={applyPatch}>
                <label>
                  Dimension
                  <select value={patchDimension} onChange={(event) => setPatchDimension(event.target.value)}>
                    {states.map((row) => (
                      <option key={row.id} value={row.id}>
                        {row.id}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Value
                  <input value={patchValue} onChange={(event) => setPatchValue(event.target.value)} placeholder="0.0" />
                </label>
                <label>
                  Reason
                  <input value={patchReason} onChange={(event) => setPatchReason(event.target.value)} placeholder="manual tuning" />
                </label>
                <label>
                  Actor
                  <input value={patchActor} onChange={(event) => setPatchActor(event.target.value)} placeholder="owner" />
                </label>
                <button type="submit" disabled={patchBusy || !patchDimension}>
                  {patchBusy ? "Applying..." : "Apply Patch"}
                </button>
              </form>
            </article>

            <article className="panel">
              <div className="panel-head">
                <div>
                  <div className="panel-kicker">Audit</div>
                  <h2>Patch History</h2>
                </div>
              </div>
              <div className="scroll-list tall-list">
                {history.map((entry) => (
                  <div key={entry.sequence} className="history-item">
                    <div className="mono">#{entry.sequence} {entry.dimension_id}</div>
                    <div>
                      {entry.before.toFixed(4)} → {entry.after.toFixed(4)}
                    </div>
                    <div>{entry.reason}</div>
                    <div className="muted">{entry.actor} · {formatAgo(entry.timestamp)}</div>
                  </div>
                ))}
              </div>
            </article>
          </section>
        </>
      ) : null}
        </section>
      </div>
    </main>
  );
}
