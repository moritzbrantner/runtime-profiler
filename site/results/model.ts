import type { BundleValidationReport, JsonRecord } from "../validator.mjs";

export interface DetailRow {
  label: string;
  value: unknown;
}

export interface MeasurementRow {
  id: string | null;
  unit: string | null;
  preferredDirection: string | null;
  sampleCount: number | null;
  minimum: number | null;
  median: number | null;
  mean: number | null;
  p95: number | null;
  maximum: number | null;
}

export interface SampleRow {
  iteration: number | null;
  durationMs: number | null;
  maxRssKib: number | null;
  exitCode: number | null;
  timedOut: boolean;
  succeeded: boolean;
}

export interface NativeHotspotRow {
  symbol: string | null;
  sourceFile: string | null;
  line: number | null;
  weight: number | null;
  samples: number | null;
  confidence: string | null;
  evidenceRef: string | null;
}

export interface NativeProfile {
  status: string;
  reason: string | null;
  collector: string | null;
  event: string | null;
  metric: string | null;
  unit: string | null;
  totalWeight: number | null;
  totalSamples: number | null;
  truncated: boolean;
  rows: NativeHotspotRow[];
}

export interface BrowserSummary {
  traceEventCount: number | null;
  topLevelTaskCount: number | null;
  topLevelDurationUs: number | null;
  longTaskCount: number | null;
  longTaskTotalDurationUs: number | null;
  longestTaskUs: number | null;
  longTasksTruncated: boolean;
  hotPathCount: number | null;
  hotPathsTruncated: boolean;
  hotPathDepthTruncated: boolean;
  boundaryMarkerCount: number | null;
  boundaryMarkersTruncated: boolean;
}

export interface BrowserLongTaskRow {
  name: string | null;
  category: string | null;
  startUs: number | null;
  durationUs: number | null;
  runtimeKind: string | null;
  evidenceRef: string | null;
}

export interface BrowserHotPathRow {
  frames: string;
  runtimeKind: string | null;
  totalDurationUs: number | null;
  maxDurationUs: number | null;
  occurrences: number | null;
  evidenceRef: string | null;
}

export interface RuntimeAttributionRow {
  runtimeKind: string | null;
  inclusiveDurationUs: number | null;
  eventCount: number | null;
}

export interface BoundaryMarkerRow {
  direction: string | null;
  label: string | null;
  totalDurationUs: number | null;
  maxDurationUs: number | null;
  occurrences: number | null;
  evidenceRef: string | null;
}

export interface BrowserProfile {
  runtime: JsonRecord | null;
  summary: BrowserSummary;
  longTasks: BrowserLongTaskRow[];
  hotPaths: BrowserHotPathRow[];
  runtimeAttribution: RuntimeAttributionRow[];
  boundaryMarkers: BoundaryMarkerRow[];
  limitations: string[];
}

export interface GuidanceObservation {
  id: string | null;
  summary: string | null;
  evidenceRef: string | null;
}

export interface ArtifactRow {
  path: string;
  mediaType: string | null;
  sha256: string | null;
  verified: boolean;
  diagnostics: string[];
}

export interface ResultModel {
  valid: boolean;
  verifiedFiles: number;
  diagnostics: string[];
  identity: DetailRow[];
  environment: DetailRow[];
  measurements: MeasurementRow[];
  samples: SampleRow[];
  nativeProfile: NativeProfile;
  browserProfile: BrowserProfile | null;
  guidance: {
    observations: GuidanceObservation[];
    constraints: string[];
  };
  artifacts: ArtifactRow[];
}

function records(value: unknown): JsonRecord[] {
  return Array.isArray(value)
    ? value.filter((item): item is JsonRecord => Boolean(item) && typeof item === "object")
    : [];
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function stringOrNull(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function numberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function valueOrNull(value: unknown): unknown | null {
  return value === undefined ? null : value;
}

function targetDetail(target: unknown): string | null {
  if (!target || typeof target !== "object") return null;
  const value = target as JsonRecord;
  if (value.target_type === "browser-journey") return stringOrNull(value.module);
  if (value.target_type === "command") return stringOrNull(value.program);
  return null;
}

function artifactRows(report: BundleValidationReport, manifest: JsonRecord): ArtifactRow[] {
  const statusByPath = new Map(report.artifacts.map((artifact) => [artifact.path, artifact]));
  return records(manifest.files).map((artifact) => {
    const path = stringOrNull(artifact.path) ?? "";
    const status = statusByPath.get(path);
    return {
      path,
      mediaType: stringOrNull(artifact.media_type),
      sha256: stringOrNull(artifact.sha256),
      verified: status?.verified === true,
      diagnostics: status?.diagnostics ?? [],
    };
  });
}

export function buildResultModel(report: BundleValidationReport): ResultModel {
  const evidence = report.evidence;
  const manifest = evidence.manifest;
  const scenario = evidence.scenario ?? {};
  const environment = evidence.environment ?? {};
  const metrics = evidence.metrics ?? {};
  const hotspots = evidence.hotspots ?? {};
  const browserSummary = evidence.chromium_trace_summary;
  const browserRuntime = evidence.browser_runtime;
  const guidance = evidence.agent_guidance ?? {};
  const source = (manifest.source ?? {}) as JsonRecord;
  const environmentSource = (environment.source ?? {}) as JsonRecord;
  const target = (scenario.target ?? {}) as JsonRecord;

  return {
    valid: report.valid,
    verifiedFiles: report.verified_files,
    diagnostics: report.diagnostics,
    identity: [
      { label: "Scenario", value: scenario.id ?? report.scenario_id },
      { label: "Target", value: target.target_type ?? null },
      { label: "Target detail", value: targetDetail(target) },
      { label: "Collectors", value: strings(scenario.collectors).join(", ") || null },
      { label: "Source revision", value: source.git_sha ?? environmentSource.git_sha ?? null },
      { label: "Working tree dirty", value: valueOrNull(source.dirty ?? environmentSource.dirty) },
      { label: "Captured", value: valueOrNull(manifest.created_unix_ms) },
      { label: "Bundle id", value: manifest.bundle_id ?? report.bundle_id },
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
    measurements: records(metrics.metrics).map((metric) => {
      const statistics = (metric.statistics ?? {}) as JsonRecord;
      return {
        id: stringOrNull(metric.id),
        unit: stringOrNull(metric.unit),
        preferredDirection: stringOrNull(metric.preferred_direction),
        sampleCount: numberOrNull(statistics.sample_count),
        minimum: numberOrNull(statistics.minimum),
        median: numberOrNull(statistics.median),
        mean: numberOrNull(statistics.mean),
        p95: numberOrNull(statistics.p95),
        maximum: numberOrNull(statistics.maximum),
      };
    }),
    samples: records(metrics.samples).map((sample) => ({
      iteration: numberOrNull(sample.iteration),
      durationMs: numberOrNull(sample.duration_ms),
      maxRssKib: numberOrNull(sample.max_rss_kib),
      exitCode: numberOrNull(sample.exit_code),
      timedOut: sample.timed_out === true,
      succeeded: sample.succeeded === true,
    })),
    nativeProfile: {
      status: stringOrNull(hotspots.status) ?? "not-collected",
      reason: stringOrNull(hotspots.reason),
      collector: stringOrNull(hotspots.collector),
      event: stringOrNull(hotspots.event),
      metric: stringOrNull(hotspots.metric),
      unit: stringOrNull(hotspots.unit),
      totalWeight: numberOrNull(hotspots.total_weight),
      totalSamples: numberOrNull(hotspots.total_samples),
      truncated: hotspots.truncated === true,
      rows: records(hotspots.hotspots).map((hotspot) => ({
        symbol: stringOrNull(hotspot.symbol),
        sourceFile: stringOrNull(hotspot.source_file),
        line: numberOrNull(hotspot.line),
        weight: numberOrNull(hotspot.weight),
        samples: numberOrNull(hotspot.samples),
        confidence: stringOrNull(hotspot.confidence),
        evidenceRef: stringOrNull(hotspot.evidence_ref),
      })),
    },
    browserProfile: browserSummary
      ? {
          runtime: browserRuntime,
          summary: {
            traceEventCount: numberOrNull(browserSummary.trace_event_count),
            topLevelTaskCount: numberOrNull(browserSummary.top_level_task_count),
            topLevelDurationUs: numberOrNull(browserSummary.top_level_duration_us),
            longTaskCount: numberOrNull(browserSummary.long_task_count),
            longTaskTotalDurationUs: numberOrNull(browserSummary.long_task_total_duration_us),
            longestTaskUs: numberOrNull(browserSummary.longest_task_us),
            longTasksTruncated: browserSummary.long_tasks_truncated === true,
            hotPathCount: numberOrNull(browserSummary.hot_path_count),
            hotPathsTruncated: browserSummary.hot_paths_truncated === true,
            hotPathDepthTruncated: browserSummary.hot_path_depth_truncated === true,
            boundaryMarkerCount: numberOrNull(browserSummary.boundary_marker_count),
            boundaryMarkersTruncated: browserSummary.boundary_markers_truncated === true,
          },
          longTasks: records(browserSummary.long_tasks).map((task) => ({
            name: stringOrNull(task.name),
            category: stringOrNull(task.category),
            startUs: numberOrNull(task.start_us),
            durationUs: numberOrNull(task.duration_us),
            runtimeKind: stringOrNull(task.runtime_kind),
            evidenceRef: stringOrNull(task.evidence_ref),
          })),
          hotPaths: records(browserSummary.hot_paths).map((path) => ({
            frames: records(path.frames)
              .map((frame) => stringOrNull(frame.name))
              .filter((name): name is string => name !== null)
              .join(" › "),
            runtimeKind: stringOrNull(path.leaf_runtime_kind),
            totalDurationUs: numberOrNull(path.total_duration_us),
            maxDurationUs: numberOrNull(path.max_duration_us),
            occurrences: numberOrNull(path.occurrences),
            evidenceRef: stringOrNull(path.evidence_ref),
          })),
          runtimeAttribution: records(browserSummary.runtime_attribution).map((row) => ({
            runtimeKind: stringOrNull(row.runtime_kind),
            inclusiveDurationUs: numberOrNull(row.inclusive_duration_us),
            eventCount: numberOrNull(row.event_count),
          })),
          boundaryMarkers: records(browserSummary.boundary_markers).map((marker) => ({
            direction: stringOrNull(marker.direction),
            label: stringOrNull(marker.label),
            totalDurationUs: numberOrNull(marker.total_duration_us),
            maxDurationUs: numberOrNull(marker.max_duration_us),
            occurrences: numberOrNull(marker.occurrences),
            evidenceRef: stringOrNull(marker.evidence_ref),
          })),
          limitations: strings(browserSummary.limitations),
        }
      : null,
    guidance: {
      observations: records(guidance.observations).map((observation) => ({
        id: stringOrNull(observation.id),
        summary: stringOrNull(observation.summary),
        evidenceRef: stringOrNull(observation.evidence_ref),
      })),
      constraints: strings(guidance.constraints),
    },
    artifacts: artifactRows(report, manifest),
  };
}

export interface FormatResultOptions {
  timestamp?: boolean;
  unit?: string | null;
}

export function formatResultValue(value: unknown, options: FormatResultOptions = {}): string {
  if (value === null || value === undefined || value === "") return "—";
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (options.timestamp && typeof value === "number" && Number.isFinite(value)) {
    return new Date(value).toLocaleString();
  }
  if (typeof value === "number") {
    const formatted = Number.isInteger(value)
      ? value.toLocaleString()
      : value.toLocaleString(undefined, { maximumFractionDigits: 3 });
    return options.unit ? `${formatted} ${options.unit}` : formatted;
  }
  return String(value);
}
