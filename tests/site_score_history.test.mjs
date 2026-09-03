import assert from "node:assert/strict";
import test from "node:test";

import {
  chartPoints,
  HISTORY_SCHEMA,
  latestEntry,
  metricChange,
  normalizeHistory,
  shortCommit,
} from "../site/score/model.js";

const history = {
  schema_version: HISTORY_SCHEMA,
  repository: "moritzbrantner/runtime-profiler",
  entries: [
    {
      commit: "bbbbbbbb22222222",
      parent_commit: "aaaaaaaa11111111",
      timestamp: "2026-09-03T12:00:00Z",
      score: 96,
      average_change_percent: -1.5,
      metrics: [{ id: "process.wall_time", average_change_percent: -1.5 }],
    },
    {
      commit: "aaaaaaaa11111111",
      parent_commit: "9999999999999999",
      timestamp: "2026-09-02T12:00:00Z",
      score: 100,
      average_change_percent: 2,
      metrics: [],
    },
  ],
};

test("runtime dashboard normalizes chronological score history", () => {
  const normalized = normalizeHistory(history);

  assert.deepEqual(
    normalized.entries.map((entry) => entry.score),
    [100, 96],
  );
  assert.equal(latestEntry(normalized).score, 96);
  assert.equal(shortCommit(latestEntry(normalized).commit), "bbbbbbbb");
  assert.equal(metricChange(latestEntry(normalized).metrics[0]), -1.5);
});

test("runtime dashboard chart keeps 100 above regressed scores", () => {
  const points = chartPoints(normalizeHistory(history).entries);

  assert.equal(points.length, 2);
  assert.ok(points[0].y < points[1].y);
});

test("runtime dashboard rejects incompatible history schemas", () => {
  assert.throws(
    () => normalizeHistory({ ...history, schema_version: "other/v1" }),
    new RegExp(HISTORY_SCHEMA.replaceAll("/", "\\/")),
  );
});
