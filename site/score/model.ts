export const HISTORY_SCHEMA = "runtime-profiler/score-history/v1";

export interface ScoreStatistic {
  statistic: string | null;
  change_percent: number | null;
  score: number | null;
}

export interface ScoreMetric {
  id: string | null;
  score: number | null;
  average_change_percent: number | null;
  statistics: ScoreStatistic[];
}

export interface ScoreEntry {
  commit: string;
  parent_commit: string;
  timestamp: string;
  score: number | null;
  average_change_percent: number | null;
  status: string | null;
  rating: string | null;
  reason: string | null;
  metrics: ScoreMetric[];
}

export interface ScoreHistory {
  schema_version: typeof HISTORY_SCHEMA;
  repository: string | null;
  entries: ScoreEntry[];
}

export interface ChartPoint extends ScoreEntry {
  x: number;
  y: number;
}

type JsonRecord = Record<string, unknown>;

function record(value: unknown): JsonRecord | null {
  return Boolean(value) && typeof value === "object" ? (value as JsonRecord) : null;
}

function records(value: unknown): JsonRecord[] {
  return Array.isArray(value)
    ? value.map(record).filter((item): item is JsonRecord => item !== null)
    : [];
}

function stringOrNull(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function numberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function normalizeMetric(value: JsonRecord): ScoreMetric {
  return {
    id: stringOrNull(value.id),
    score: numberOrNull(value.score),
    average_change_percent: numberOrNull(value.average_change_percent),
    statistics: records(value.statistics).map((statistic) => ({
      statistic: stringOrNull(statistic.statistic),
      change_percent: numberOrNull(statistic.change_percent),
      score: numberOrNull(statistic.score),
    })),
  };
}

export function normalizeHistory(document: unknown): ScoreHistory {
  const value = record(document);
  if (!value || value.schema_version !== HISTORY_SCHEMA) {
    throw new Error(`Expected ${HISTORY_SCHEMA}`);
  }
  const entries = records(value.entries)
    .filter((entry) => typeof entry.commit === "string")
    .map((entry): ScoreEntry => ({
      commit: String(entry.commit),
      parent_commit: stringOrNull(entry.parent_commit) ?? "",
      timestamp: stringOrNull(entry.timestamp) ?? "",
      score: numberOrNull(entry.score),
      average_change_percent: numberOrNull(entry.average_change_percent),
      status: stringOrNull(entry.status),
      rating: stringOrNull(entry.rating),
      reason: stringOrNull(entry.reason),
      metrics: records(entry.metrics).map(normalizeMetric),
    }))
    .sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  return {
    schema_version: HISTORY_SCHEMA,
    repository: stringOrNull(value.repository),
    entries,
  };
}

export function latestEntry(history: ScoreHistory): ScoreEntry | null {
  return history.entries.at(-1) ?? null;
}

export function shortCommit(commit: string): string {
  return commit.slice(0, 8);
}

export function chartPoints(
  entries: ScoreEntry[],
  width = 720,
  height = 240,
  padding = 24,
): ChartPoint[] {
  const scored = entries.filter(
    (entry): entry is ScoreEntry & { score: number } => entry.score !== null,
  );
  if (scored.length === 0) return [];
  const usableWidth = width - padding * 2;
  const usableHeight = height - padding * 2;
  return scored.map((entry, index) => ({
    ...entry,
    x: scored.length === 1 ? width / 2 : padding + (index / (scored.length - 1)) * usableWidth,
    y: padding + ((100 - entry.score) / 100) * usableHeight,
  }));
}

export function metricChange(metric: ScoreMetric | null | undefined): number | null {
  return metric?.average_change_percent ?? null;
}
