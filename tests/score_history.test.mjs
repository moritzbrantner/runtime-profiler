import assert from "node:assert/strict";
import test from "node:test";

import {
  appendRuntimeScoreHistory,
  SCORE_HISTORY_RETENTION,
  SCORE_HISTORY_SCHEMA_V1,
} from "../scripts/append-score-history.mjs";

const metadata = {
  repository: "moritzbrantner/runtime-profiler",
  commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  parent_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  timestamp: "2026-09-03T18:00:00Z",
};

function score(value = 96) {
  return {
    schema_version: "runtime-profiler/score/v1",
    scenario_id: "history-smoke",
    scenario_digest: "scenario",
    environment_fingerprint_schema_version: "runtime-profiler/environment-fingerprint/v1",
    environment_fingerprint: "environment",
    reference_bundle_id: "reference",
    candidate_bundle_id: "candidate",
    reference_source: {},
    candidate_source: {},
    score: value,
    rating: value >= 90 ? "good" : "needs-improvement",
    metrics: [
      {
        id: "process.wall_time",
        unit: "ms",
        preferred_direction: "lower",
        score: value,
        statistics: [
          { statistic: "median", reference: 10, candidate: 10.5, change_percent: -5, score: 95 },
          { statistic: "p95", reference: 12, candidate: 12, change_percent: 0, score: 100 },
        ],
      },
      {
        id: "process.success_rate",
        unit: "ratio",
        preferred_direction: "higher",
        score: 100,
        statistics: [
          { statistic: "mean", reference: 1, candidate: 1, change_percent: 0, score: 100 },
        ],
      },
    ],
    excluded_metrics: [],
    notes: [],
  };
}

test("runtime history stores parent-relative score and signed changes", () => {
  const history = appendRuntimeScoreHistory(null, score(), metadata);

  assert.equal(history.schema_version, SCORE_HISTORY_SCHEMA_V1);
  assert.equal(history.retention, SCORE_HISTORY_RETENTION);
  assert.equal(history.semantics, "parent-relative-retention");
  assert.equal(history.entries.length, 1);
  assert.deepEqual(history.entries[0], {
    commit: metadata.commit,
    parent_commit: metadata.parent_commit,
    timestamp: metadata.timestamp,
    status: "scored",
    score: 96,
    rating: "good",
    average_change_percent: -1.25,
    scenario_id: "history-smoke",
    environment_fingerprint_schema_version: "runtime-profiler/environment-fingerprint/v1",
    metrics: [
      {
        id: "process.wall_time",
        score: 96,
        average_change_percent: -2.5,
        statistics: [
          { statistic: "median", score: 95, change_percent: -5 },
          { statistic: "p95", score: 100, change_percent: 0 },
        ],
      },
      {
        id: "process.success_rate",
        score: 100,
        average_change_percent: 0,
        statistics: [{ statistic: "mean", score: 100, change_percent: 0 }],
      },
    ],
    excluded_metrics: [],
    reason: null,
  });
});

test("runtime history records an unavailable comparison instead of inventing a score", () => {
  const history = appendRuntimeScoreHistory(null, null, {
    ...metadata,
    reason: "scenario fingerprints differ",
  });

  assert.equal(history.entries[0].status, "unavailable");
  assert.equal(history.entries[0].score, null);
  assert.equal(history.entries[0].reason, "scenario fingerprints differ");
});

test("rerunning the same commit replaces the existing history entry", () => {
  const first = appendRuntimeScoreHistory(null, score(90), metadata);
  const rerun = appendRuntimeScoreHistory(first, score(100), metadata);

  assert.equal(rerun.entries.length, 1);
  assert.equal(rerun.entries[0].score, 100);
});
