import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import test from "node:test";

import { isSafeRelativePath, sha256, validateBundleUrl } from "../site/validator.mjs";

const encoder = new TextEncoder();
const manifestUrl = "https://example.test/bundle/manifest.json";

test("browser validator accepts a coherent v1 bundle", async () => {
  const fixture = await bundleFixture();
  const report = await validateBundleUrl(manifestUrl, {
    fetchImpl: fakeFetch(fixture.responses),
    cryptoImpl: webcrypto,
  });

  assert.equal(report.valid, true);
  assert.equal(report.verified_files, 5);
  assert.equal(report.summary.metric_count, 1);
  assert.equal(report.summary.sample_count, 1);
  assert.deepEqual(report.diagnostics, []);
});

test("browser validator accepts optional native perf evidence", async () => {
  const fixture = await bundleFixture({ nativePerf: true });
  const report = await validateBundleUrl(manifestUrl, {
    fetchImpl: fakeFetch(fixture.responses),
    cryptoImpl: webcrypto,
  });

  assert.equal(report.valid, true);
  assert.equal(report.verified_files, 6);
  assert.equal(report.evidence.hotspots.collector, "native-perf");
  assert.equal(report.evidence.hotspots.sample_period, 100000);
  assert.equal(report.evidence.hotspots.target_toolchain_kind, "rustc");
  assert.equal(report.evidence.hotspots.hotspots[0].symbol, "example::work");
  assert.deepEqual(report.diagnostics, []);
});

test("browser validator accepts Chromium journey evidence", async () => {
  const fixture = await bundleFixture({ browser: true });
  const report = await validateBundleUrl(manifestUrl, {
    fetchImpl: fakeFetch(fixture.responses),
    cryptoImpl: webcrypto,
  });

  assert.equal(report.valid, true);
  assert.equal(report.verified_files, 8);
  assert.equal(report.summary.metric_count, 0);
  assert.equal(report.summary.sample_count, 0);
  assert.equal(report.evidence.browser_runtime.browser_name, "chromium");
  assert.equal(report.evidence.chromium_trace_summary.long_task_count, 1);
  assert.deepEqual(report.diagnostics, []);
});

test("browser validator rejects partial native toolchain identity", async () => {
  const fixture = await bundleFixture({ nativePerf: true, partialToolchain: true });
  const report = await validateBundleUrl(manifestUrl, {
    fetchImpl: fakeFetch(fixture.responses),
    cryptoImpl: webcrypto,
  });

  assert.equal(report.valid, false);
  assert.ok(
    report.diagnostics.includes("native-perf target toolchain identity is only partially recorded"),
  );
});

test("browser validator reports artifact corruption", async () => {
  const fixture = await bundleFixture();
  fixture.responses.set(
    "https://example.test/bundle/metrics.json",
    JSON.stringify({ schema_version: "runtime-profiler/metrics/v1", scenario_id: "example" }),
  );

  const report = await validateBundleUrl(manifestUrl, {
    fetchImpl: fakeFetch(fixture.responses),
    cryptoImpl: webcrypto,
  });

  assert.equal(report.valid, false);
  assert.equal(report.verified_files, 4);
  assert.ok(report.diagnostics.includes("digest mismatch: metrics.json"));
});

test("browser validator rejects escaping artifact paths", () => {
  assert.equal(isSafeRelativePath("metrics.json"), true);
  assert.equal(isSafeRelativePath("nested/metrics.json"), true);
  assert.equal(isSafeRelativePath("../metrics.json"), false);
  assert.equal(isSafeRelativePath("/metrics.json"), false);
  assert.equal(isSafeRelativePath("nested\\metrics.json"), false);
});

async function bundleFixture({
  nativePerf = false,
  partialToolchain = false,
  browser = false,
} = {}) {
  const nativeHotspots = nativePerf
    ? {
        schema_version: "runtime-profiler/hotspots/v1",
        status: "collected",
        reason: "fixture",
        collector: "native-perf",
        tool_version: "perf version test",
        event: "cycles:u",
        metric: "native-perf.period",
        unit: "event-count",
        sample_period: 100000,
        symbolization_mode: "perf-report-srcline",
        target_toolchain_kind: "rustc",
        target_toolchain_fingerprint_schema_version:
          "runtime-profiler/target-toolchain-fingerprint/rustc-v1",
        target_toolchain_fingerprint:
          "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        total_weight: 100000,
        total_samples: 1,
        truncated: false,
        hotspots: [
          {
            id: "hotspot-fixture",
            symbol: "example::work",
            source_file: "src/lib.rs",
            line: 10,
            dso: "example",
            metric: "native-perf.period",
            unit: "event-count",
            weight: 100000,
            samples: 1,
            confidence: "source-location",
            evidence_ref: "hotspots.json#hotspot-fixture",
          },
        ],
      }
    : {
        schema_version: "runtime-profiler/hotspots/v1",
        status: "not-collected",
        reason: "fixture",
        hotspots: [],
      };
  if (partialToolchain && nativePerf) {
    delete nativeHotspots.target_toolchain_fingerprint;
  }

  const scenario = browser
    ? {
        schema_version: "runtime-profiler/scenario-evidence/v1",
        id: "example",
        digest: "scenario-digest",
        target: {
          target_type: "browser-journey",
          module: "profiles/journey.mjs",
          working_directory_set: true,
          inherited_environment_names: [],
        },
        run: { warmup_iterations: 0, measurement_iterations: 1, timeout_seconds: 30 },
        collectors: ["browser-chromium"],
      }
    : {
        schema_version: "runtime-profiler/scenario-evidence/v1",
        id: "example",
        digest: "scenario-digest",
        target: { target_type: "command" },
        run: { warmup_iterations: 1, measurement_iterations: 1, timeout_seconds: 30 },
        collectors: nativePerf ? ["process", "native-perf"] : ["process"],
      };

  const metrics = browser
    ? {
        schema_version: "runtime-profiler/metrics/v1",
        scenario_id: "example",
        samples: [],
        metrics: [],
      }
    : {
        schema_version: "runtime-profiler/metrics/v1",
        scenario_id: "example",
        samples: [
          {
            iteration: 0,
            duration_ms: 10,
            max_rss_kib: 100,
            exit_code: 0,
            timed_out: false,
            succeeded: true,
          },
        ],
        metrics: [
          {
            id: "process.wall_time",
            unit: "ms",
            preferred_direction: "lower",
            statistics: {
              sample_count: 1,
              minimum: 10,
              maximum: 10,
              mean: 10,
              median: 10,
              p95: 10,
            },
          },
        ],
      };

  const documents = {
    "scenario.json": scenario,
    "environment.json": {
      schema_version: "runtime-profiler/environment/v1",
      environment_fingerprint_schema_version: "runtime-profiler/environment-fingerprint/v1",
      fingerprint: "environment-digest",
      operating_system: "linux",
      architecture: "x86_64",
      kernel_release: null,
      logical_cpu_count: 8,
      source: { git_sha: "abc123", dirty: false },
    },
    "metrics.json": metrics,
    "hotspots.json": nativeHotspots,
    "agent-guidance.json": {
      schema_version: "runtime-profiler/agent-guidance/v1",
      scenario_id: "example",
      observations: [],
      constraints: [],
      evidence_refs: ["metrics.json"],
    },
  };

  if (browser) {
    documents["chromium-trace-summary.json"] = {
      schema_version: "runtime-profiler/chromium-trace-summary/v1",
      trace_event_count: 3,
      main_thread: { process_id: 1, thread_id: 2, name: "CrRendererMain" },
      top_level_task_count: 1,
      top_level_duration_us: 60000,
      long_task_count: 1,
      long_task_total_duration_us: 60000,
      longest_task_us: 60000,
      long_tasks_truncated: false,
      long_tasks: [],
      hot_path_count: 0,
      hot_paths_truncated: false,
      hot_path_depth_truncated: false,
      hot_paths: [],
      runtime_attribution: [],
      boundary_marker_count: 0,
      boundary_markers_truncated: false,
      boundary_markers: [],
      limitations: [],
    };
    documents["browser-runtime.json"] = {
      schema_version: "runtime-profiler/browser-runtime/v1",
      adapter_version: "runtime-profiler/playwright-driver/v1",
      node_version: "v24.0.0",
      playwright_version: "1.58.0",
      browser_name: "chromium",
      browser_version: "140.0.0",
      viewport: { width: 1280, height: 720 },
      trace_categories: ["devtools.timeline", "v8", "v8.execute", "blink.user_timing"],
    };
  }

  const responses = new Map();
  const files = [];
  for (const [path, document] of Object.entries(documents)) {
    const text = `${JSON.stringify(document)}\n`;
    responses.set(`https://example.test/bundle/${path}`, text);
    files.push({
      path,
      media_type: "application/json",
      sha256: await sha256(encoder.encode(text), webcrypto),
    });
  }

  if (nativePerf) {
    const path = "native-perf-report.tsv";
    const text = "1\t100000\tsrc/lib.rs:10\texample::work\texample\n";
    responses.set(`https://example.test/bundle/${path}`, text);
    files.push({
      path,
      media_type: "text/tab-separated-values; charset=utf-8",
      sha256: await sha256(encoder.encode(text), webcrypto),
    });
  }

  if (browser) {
    const path = "chromium-trace.json";
    const text = '{"traceEvents":[]}\n';
    responses.set(`https://example.test/bundle/${path}`, text);
    files.push({
      path,
      media_type: "application/json",
      sha256: await sha256(encoder.encode(text), webcrypto),
    });
  }

  const manifest = {
    schema_version: "runtime-profiler/bundle-manifest/v1",
    bundle_id: "bundle-id",
    created_unix_ms: 1,
    scenario_id: "example",
    scenario_digest: "scenario-digest",
    environment_fingerprint_schema_version: "runtime-profiler/environment-fingerprint/v1",
    environment_fingerprint: "environment-digest",
    source: { git_sha: "abc123", dirty: false },
    files,
  };
  responses.set(manifestUrl, `${JSON.stringify(manifest)}\n`);
  return { responses };
}

function fakeFetch(responses) {
  return async (url) => {
    const text = responses.get(String(url));
    if (text === undefined) {
      return { ok: false, status: 404 };
    }
    return {
      ok: true,
      status: 200,
      async text() {
        return text;
      },
      async arrayBuffer() {
        const bytes = encoder.encode(text);
        return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      },
    };
  };
}
