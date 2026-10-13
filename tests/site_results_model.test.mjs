import assert from "node:assert/strict";
import test from "node:test";

import { buildResultModel, formatResultValue } from "../site/results/model.js";

test("result explorer projects process and native evidence without inventing policy", () => {
  const report = {
    valid: true,
    verified_files: 6,
    diagnostics: [],
    artifacts: [
      { path: "metrics.json", verified: true, diagnostics: [] },
      { path: "hotspots.json", verified: true, diagnostics: [] },
    ],
    evidence: {
      manifest: {
        bundle_id: "bundle-digest",
        created_unix_ms: 1_700_000_000_000,
        environment_fingerprint: "environment-digest",
        environment_fingerprint_schema_version: "runtime-profiler/environment-fingerprint/v1",
        source: { git_sha: "abc123", dirty: false },
        files: [
          { path: "metrics.json", media_type: "application/json", sha256: "metric-digest" },
          { path: "hotspots.json", media_type: "application/json", sha256: "hotspot-digest" },
        ],
      },
      scenario: {
        id: "example",
        target: { target_type: "command", program: "example-bin" },
        collectors: ["process", "native-perf"],
      },
      environment: {
        operating_system: "linux",
        architecture: "x86_64",
        logical_cpu_count: 8,
        fingerprint: "environment-digest",
      },
      metrics: {
        metrics: [
          {
            id: "process.wall_time",
            unit: "ms",
            preferred_direction: "lower",
            statistics: {
              sample_count: 2,
              minimum: 9,
              median: 10,
              mean: 10.5,
              p95: 12,
              maximum: 12,
            },
          },
        ],
        samples: [
          {
            iteration: 1,
            duration_ms: 9,
            max_rss_kib: 256,
            exit_code: 0,
            timed_out: false,
            succeeded: true,
          },
        ],
      },
      hotspots: {
        status: "collected",
        collector: "native-perf",
        event: "cycles:u",
        metric: "native-perf.period",
        total_weight: 100,
        total_samples: 2,
        truncated: false,
        hotspots: [
          {
            symbol: "example::work",
            source_file: "src/lib.rs",
            line: 12,
            weight: 90,
            samples: 2,
            confidence: "source-location",
            evidence_ref: "hotspots.json#work",
          },
        ],
      },
      agent_guidance: {
        observations: [
          { id: "obs", summary: "work dominates sampled evidence", evidence_ref: "hotspots.json#work" },
        ],
        constraints: ["descriptive evidence only"],
      },
    },
  };

  const model = buildResultModel(report);
  assert.equal(model.valid, true);
  assert.equal(model.measurements[0].median, 10);
  assert.equal(model.samples[0].maxRssKib, 256);
  assert.equal(model.nativeProfile.rows[0].symbol, "example::work");
  assert.equal(model.guidance.observations[0].evidenceRef, "hotspots.json#work");
  assert.equal(model.artifacts[0].verified, true);
  assert.equal(model.browserProfile, null);
});

test("result explorer projects bounded Chromium evidence and truncation", () => {
  const model = buildResultModel({
    valid: true,
    evidence: {
      manifest: { files: [] },
      scenario: {
        id: "browser",
        target: { target_type: "browser-journey", module: "profiles/journey.mjs" },
        collectors: ["browser-chromium"],
      },
      chromium_trace_summary: {
        trace_event_count: 20,
        top_level_task_count: 3,
        top_level_duration_us: 150_000,
        long_task_count: 1,
        long_task_total_duration_us: 75_000,
        longest_task_us: 75_000,
        long_tasks_truncated: true,
        long_tasks: [
          {
            name: "RunTask",
            category: "devtools.timeline",
            start_us: 100,
            duration_us: 75_000,
            runtime_kind: "javascript",
            evidence_ref: "chromium-trace-summary.json#task",
          },
        ],
        hot_path_count: 1,
        hot_paths_truncated: true,
        hot_path_depth_truncated: true,
        hot_paths: [
          {
            frames: [{ name: "RunTask" }, { name: "app::tick" }],
            leaf_runtime_kind: "wasm",
            total_duration_us: 50_000,
            max_duration_us: 30_000,
            occurrences: 2,
            evidence_ref: "chromium-trace-summary.json#path",
          },
        ],
        runtime_attribution: [
          { runtime_kind: "wasm", inclusive_duration_us: 50_000, event_count: 2 },
        ],
        boundary_marker_count: 1,
        boundary_markers_truncated: true,
        boundary_markers: [
          {
            direction: "js-to-wasm",
            label: "step",
            total_duration_us: 5_000,
            max_duration_us: 3_000,
            occurrences: 2,
            evidence_ref: "chromium-trace-summary.json#boundary",
          },
        ],
        limitations: ["bounded trace summary"],
      },
      browser_runtime: {
        browser_name: "chromium",
        browser_version: "140",
        viewport: { width: 1280, height: 720 },
      },
    },
  });

  assert.equal(model.browserProfile.summary.longTaskCount, 1);
  assert.equal(model.browserProfile.summary.longTasksTruncated, true);
  assert.equal(model.browserProfile.summary.hotPathsTruncated, true);
  assert.equal(model.browserProfile.summary.hotPathDepthTruncated, true);
  assert.equal(model.browserProfile.summary.boundaryMarkersTruncated, true);
  assert.equal(model.browserProfile.hotPaths[0].frames, "RunTask › app::tick");
  assert.equal(model.browserProfile.runtimeAttribution[0].runtimeKind, "wasm");
  assert.equal(model.browserProfile.boundaryMarkers[0].direction, "js-to-wasm");
  assert.deepEqual(model.browserProfile.limitations, ["bounded trace summary"]);
});

test("result value formatting keeps missing values explicit", () => {
  assert.equal(formatResultValue(null), "—");
  assert.equal(formatResultValue(false), "no");
  assert.equal(formatResultValue(12, { unit: "ms" }), "12 ms");
});
