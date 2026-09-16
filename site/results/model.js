function array(value) {
  return Array.isArray(value) ? value : [];
}

function valueOrNull(value) {
  return value === undefined ? null : value;
}

function targetDetail(target) {
  if (!target || typeof target !== "object") return null;
  if (target.target_type === "browser-journey") return target.module ?? null;
  if (target.target_type === "command") return target.program ?? null;
  return null;
}

function artifactRows(report, manifest) {
  const statusByPath = new Map(
    array(report?.artifacts).map((artifact) => [artifact.path, artifact]),
  );
  return array(manifest?.files).map((artifact) => {
    const status = statusByPath.get(artifact.path);
    return {
      path: artifact.path,
      mediaType: artifact.media_type,
      sha256: artifact.sha256,
      verified: status?.verified === true,
      diagnostics: array(status?.diagnostics),
    };
  });
}

export function buildResultModel(report) {
  const evidence = report?.evidence ?? {};
  const manifest = evidence.manifest ?? {};
  const scenario = evidence.scenario ?? {};
  const environment = evidence.environment ?? {};
  const metrics = evidence.metrics ?? {};
  const hotspots = evidence.hotspots ?? {};
  const browserSummary = evidence.chromium_trace_summary ?? null;
  const browserRuntime = evidence.browser_runtime ?? null;
  const guidance = evidence.agent_guidance ?? {};

  return {
    valid: report?.valid === true,
    verifiedFiles: Number.isInteger(report?.verified_files) ? report.verified_files : 0,
    diagnostics: array(report?.diagnostics),
    identity: [
      { label: "Scenario", value: scenario.id ?? report?.scenario_id ?? null },
      { label: "Target", value: scenario.target?.target_type ?? null },
      { label: "Target detail", value: targetDetail(scenario.target) },
      { label: "Collectors", value: array(scenario.collectors).join(", ") || null },
      { label: "Source revision", value: manifest.source?.git_sha ?? environment.source?.git_sha ?? null },
      { label: "Working tree dirty", value: valueOrNull(manifest.source?.dirty ?? environment.source?.dirty) },
      { label: "Captured", value: valueOrNull(manifest.created_unix_ms) },
      { label: "Bundle id", value: manifest.bundle_id ?? report?.bundle_id ?? null },
    ],
    environment: [
      { label: "Operating system", value: environment.operating_system ?? null },
      { label: "Architecture", value: environment.architecture ?? null },
      { label: "Kernel", value: environment.kernel_release ?? null },
      { label: "Logical CPUs", value: valueOrNull(environment.logical_cpu_count) },
      { label: "Fingerprint", value: environment.fingerprint ?? manifest.environment_fingerprint ?? null },
      {
        label: "Fingerprint schema",
        value:
          environment.environment_fingerprint_schema_version ??
          manifest.environment_fingerprint_schema_version ??
          null,
      },
    ],
    measurements: array(metrics.metrics).map((metric) => ({
      id: metric.id,
      unit: metric.unit,
      preferredDirection: metric.preferred_direction,
      sampleCount: valueOrNull(metric.statistics?.sample_count),
      minimum: valueOrNull(metric.statistics?.minimum),
      median: valueOrNull(metric.statistics?.median),
      mean: valueOrNull(metric.statistics?.mean),
      p95: valueOrNull(metric.statistics?.p95),
      maximum: valueOrNull(metric.statistics?.maximum),
    })),
    samples: array(metrics.samples).map((sample) => ({
      iteration: valueOrNull(sample.iteration),
      durationMs: valueOrNull(sample.duration_ms),
      maxRssKib: valueOrNull(sample.max_rss_kib),
      exitCode: valueOrNull(sample.exit_code),
      timedOut: sample.timed_out === true,
      succeeded: sample.succeeded === true,
    })),
    nativeProfile: {
      status: hotspots.status ?? "not-collected",
      reason: hotspots.reason ?? null,
      collector: hotspots.collector ?? null,
      event: hotspots.event ?? null,
      metric: hotspots.metric ?? null,
      unit: hotspots.unit ?? null,
      totalWeight: valueOrNull(hotspots.total_weight),
      totalSamples: valueOrNull(hotspots.total_samples),
      truncated: hotspots.truncated === true,
      rows: array(hotspots.hotspots).map((hotspot) => ({
        symbol: hotspot.symbol,
        sourceFile: hotspot.source_file ?? null,
        line: valueOrNull(hotspot.line),
        weight: valueOrNull(hotspot.weight),
        samples: valueOrNull(hotspot.samples),
        confidence: hotspot.confidence ?? null,
        evidenceRef: hotspot.evidence_ref ?? null,
      })),
    },
    browserProfile: browserSummary
      ? {
          runtime: browserRuntime,
          summary: {
            traceEventCount: valueOrNull(browserSummary.trace_event_count),
            topLevelTaskCount: valueOrNull(browserSummary.top_level_task_count),
            topLevelDurationUs: valueOrNull(browserSummary.top_level_duration_us),
            longTaskCount: valueOrNull(browserSummary.long_task_count),
            longTaskTotalDurationUs: valueOrNull(browserSummary.long_task_total_duration_us),
            longestTaskUs: valueOrNull(browserSummary.longest_task_us),
            longTasksTruncated: browserSummary.long_tasks_truncated === true,
            hotPathCount: valueOrNull(browserSummary.hot_path_count),
            hotPathsTruncated: browserSummary.hot_paths_truncated === true,
            boundaryMarkerCount: valueOrNull(browserSummary.boundary_marker_count),
            boundaryMarkersTruncated: browserSummary.boundary_markers_truncated === true,
          },
          longTasks: array(browserSummary.long_tasks).map((task) => ({
            name: task.name,
            category: task.category,
            startUs: valueOrNull(task.start_us),
            durationUs: valueOrNull(task.duration_us),
            runtimeKind: task.runtime_kind,
            evidenceRef: task.evidence_ref,
          })),
          hotPaths: array(browserSummary.hot_paths).map((path) => ({
            frames: array(path.frames).map((frame) => frame.name).join(" › "),
            runtimeKind: path.leaf_runtime_kind,
            totalDurationUs: valueOrNull(path.total_duration_us),
            maxDurationUs: valueOrNull(path.max_duration_us),
            occurrences: valueOrNull(path.occurrences),
            evidenceRef: path.evidence_ref,
          })),
          runtimeAttribution: array(browserSummary.runtime_attribution).map((row) => ({
            runtimeKind: row.runtime_kind,
            inclusiveDurationUs: valueOrNull(row.inclusive_duration_us),
            eventCount: valueOrNull(row.event_count),
          })),
          boundaryMarkers: array(browserSummary.boundary_markers).map((marker) => ({
            direction: marker.direction,
            label: marker.label,
            totalDurationUs: valueOrNull(marker.total_duration_us),
            maxDurationUs: valueOrNull(marker.max_duration_us),
            occurrences: valueOrNull(marker.occurrences),
            evidenceRef: marker.evidence_ref,
          })),
          limitations: array(browserSummary.limitations),
        }
      : null,
    guidance: {
      observations: array(guidance.observations).map((observation) => ({
        id: observation.id,
        summary: observation.summary,
        evidenceRef: observation.evidence_ref,
      })),
      constraints: array(guidance.constraints),
    },
    artifacts: artifactRows(report, manifest),
  };
}

export function formatResultValue(value, options = {}) {
  if (value === null || value === undefined || value === "") return "—";
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (options.timestamp && Number.isFinite(value)) return new Date(value).toLocaleString();
  if (typeof value === "number") {
    const formatted = Number.isInteger(value)
      ? value.toLocaleString()
      : value.toLocaleString(undefined, { maximumFractionDigits: 3 });
    return options.unit ? `${formatted} ${options.unit}` : formatted;
  }
  return String(value);
}
