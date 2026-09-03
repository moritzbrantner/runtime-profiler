import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const SCORE_HISTORY_SCHEMA_V1 = "runtime-profiler/score-history/v1";
export const SCORE_HISTORY_RETENTION = 1000;

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function rounded(value) {
  return Math.round(value * 1000) / 1000;
}

function average(values) {
  const finite = values.filter(Number.isFinite);
  if (finite.length === 0) return null;
  return rounded(finite.reduce((sum, value) => sum + value, 0) / finite.length);
}

function metricSnapshot(metric) {
  const statistics = (Array.isArray(metric.statistics) ? metric.statistics : []).map((statistic) => ({
    statistic: statistic.statistic,
    score: statistic.score,
    change_percent: Number.isFinite(statistic.change_percent) ? statistic.change_percent : null,
  }));
  return {
    id: metric.id,
    score: metric.score,
    average_change_percent: average(statistics.map((statistic) => statistic.change_percent)),
    statistics,
  };
}

export function appendRuntimeScoreHistory(existing, score, metadata) {
  if (!metadata?.repository || !metadata?.commit || !metadata?.parent_commit || !metadata?.timestamp) {
    throw new Error("repository, commit, parent_commit, and timestamp metadata are required");
  }
  if (score && score.schema_version !== "runtime-profiler/score/v1") {
    throw new Error("score report is not runtime-profiler/score/v1");
  }

  const history = existing ?? {
    schema_version: SCORE_HISTORY_SCHEMA_V1,
    repository: metadata.repository,
    semantics: "parent-relative-retention",
    score_schema_version: "runtime-profiler/score/v1",
    retention: SCORE_HISTORY_RETENTION,
    entries: [],
  };
  if (history.schema_version !== SCORE_HISTORY_SCHEMA_V1) {
    throw new Error(`unsupported score history schema: ${history.schema_version}`);
  }
  if (history.repository !== metadata.repository) {
    throw new Error(`score history belongs to ${history.repository}, not ${metadata.repository}`);
  }

  const metrics = score ? score.metrics.map(metricSnapshot) : [];
  const entry = {
    commit: metadata.commit,
    parent_commit: metadata.parent_commit,
    timestamp: metadata.timestamp,
    status: score ? "scored" : "unavailable",
    score: score?.score ?? null,
    rating: score?.rating ?? "unavailable",
    average_change_percent: average(metrics.map((metric) => metric.average_change_percent)),
    scenario_id: score?.scenario_id ?? null,
    environment_fingerprint_schema_version:
      score?.environment_fingerprint_schema_version ?? null,
    metrics,
    excluded_metrics: score?.excluded_metrics ?? [],
    reason: score ? null : metadata.reason ?? "runtime comparison was unavailable",
  };

  const entries = [...(Array.isArray(history.entries) ? history.entries : [])]
    .filter((candidate) => candidate?.commit !== entry.commit)
    .concat(entry)
    .sort((left, right) => String(left.timestamp).localeCompare(String(right.timestamp)))
    .slice(-SCORE_HISTORY_RETENTION);

  return {
    ...history,
    semantics: "parent-relative-retention",
    score_schema_version: "runtime-profiler/score/v1",
    retention: SCORE_HISTORY_RETENTION,
    entries,
  };
}

function option(argv, name) {
  const index = argv.indexOf(`--${name}`);
  return index >= 0 ? argv[index + 1] : undefined;
}

export function main(argv = process.argv.slice(2)) {
  const historyPath = option(argv, "history");
  const scorePath = option(argv, "score");
  const reasonPath = option(argv, "reason-file");
  const repository = option(argv, "repository");
  const commit = option(argv, "commit");
  const parentCommit = option(argv, "parent");
  const timestamp = option(argv, "timestamp");
  if (!historyPath || !repository || !commit || !parentCommit || !timestamp) {
    throw new Error(
      "Usage: append-score-history --history <path> [--score <path> | --reason-file <path>] --repository <owner/repo> --commit <sha> --parent <sha> --timestamp <iso>",
    );
  }

  const absoluteHistory = resolve(historyPath);
  const existing = existsSync(absoluteHistory) ? readJson(absoluteHistory) : null;
  const score = scorePath && existsSync(resolve(scorePath)) ? readJson(resolve(scorePath)) : null;
  const reason = reasonPath && existsSync(resolve(reasonPath))
    ? readFileSync(resolve(reasonPath), "utf8").trim()
    : undefined;
  const updated = appendRuntimeScoreHistory(existing, score, {
    repository,
    commit,
    parent_commit: parentCommit,
    timestamp,
    reason,
  });
  writeFileSync(absoluteHistory, `${JSON.stringify(updated, null, 2)}\n`, "utf8");
}

const entry = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (import.meta.url === entry) main();
