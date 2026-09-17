import { validateBundleUrl } from "../validator.mjs";
import {
  buildResultModel,
  formatResultValue,
  type DetailRow,
  type ResultModel,
} from "./model.js";

type TableRow = Record<string, any>;
interface TableColumn {
  label: string;
  numeric?: boolean;
  className?: string;
  status?: string;
  render: (row: TableRow) => string;
}

function mustElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing required element: ${selector}`);
  return element;
}

const form = mustElement<HTMLFormElement>("#manifest-form");
const input = mustElement<HTMLInputElement>("#manifest");
const status = mustElement<HTMLElement>("#status");
const reportElement = mustElement<HTMLElement>("#report");
const validationLink = mustElement<HTMLAnchorElement>("#validation-link");
const manifestLink = mustElement<HTMLAnchorElement>("#manifest-link");
let latestRun = 0;

const initial = new URL(location.href).searchParams.get("manifest");
if (initial) {
  input.value = initial;
  void run(initial);
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  void run(input.value);
});

async function run(rawManifestUrl: string): Promise<void> {
  const runId = ++latestRun;
  reportElement.hidden = true;
  setStatus("Loading, hashing, and validating public evidence…", "normal");

  try {
    const manifestUrl = normalizeManifestUrl(rawManifestUrl);
    const report = await validateBundleUrl(manifestUrl);
    if (runId !== latestRun) return;
    const model = buildResultModel(report);
    renderReport(model, manifestUrl);
    reportElement.hidden = false;
    history.replaceState(null, "", `?manifest=${encodeURIComponent(manifestUrl)}`);
    setStatus(
      model.valid
        ? `Validated ${model.verifiedFiles} integrity-checked artifact${model.verifiedFiles === 1 ? "" : "s"}.`
        : `Evidence rendered with ${model.diagnostics.length} validation diagnostic${model.diagnostics.length === 1 ? "" : "s"}; treat the bundle as invalid.`,
      model.valid ? "ok" : "error",
    );
  } catch (error: unknown) {
    if (runId !== latestRun) return;
    setStatus(error instanceof Error ? error.message : String(error), "error");
  }
}

function normalizeManifestUrl(value: string): string {
  const url = new URL(value);
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    throw new Error("Manifest URL must use HTTP or HTTPS.");
  }
  return url.href;
}

function setStatus(message: string, state: "normal" | "ok" | "error"): void {
  status.textContent = message;
  status.dataset.state = state;
}

function renderReport(model: ResultModel, manifestUrl: string): void {
  validationLink.href = `../validate.json/?manifest=${encodeURIComponent(manifestUrl)}`;
  manifestLink.href = manifestUrl;

  renderDetails(mustElement("#identity-details"), model.identity, (row) =>
    row.label === "Captured"
      ? formatResultValue(row.value, { timestamp: true })
      : formatResultValue(row.value),
  );
  renderDetails(mustElement("#environment-details"), model.environment);

  renderTable(
    mustElement("#measurements-table"),
    [
      textColumn("Metric", "id", "code"),
      textColumn("Direction", "preferredDirection"),
      numericColumn("Min", (row) => metricValue(row.minimum, row.unit)),
      numericColumn("Median", (row) => metricValue(row.median, row.unit)),
      numericColumn("Mean", (row) => metricValue(row.mean, row.unit)),
      numericColumn("p95", (row) => metricValue(row.p95, row.unit)),
      numericColumn("Max", (row) => metricValue(row.maximum, row.unit)),
      numericColumn("Samples", (row) => formatResultValue(row.sampleCount)),
    ],
    model.measurements,
    "No process metric aggregates are present in this bundle.",
  );

  renderTable(
    mustElement("#samples-table"),
    [
      numericColumn("Iteration", (row) => formatResultValue(row.iteration)),
      numericColumn("Duration", (row) => formatResultValue(row.durationMs, { unit: "ms" })),
      numericColumn("Max observed RSS", (row) => formatResultValue(row.maxRssKib, { unit: "KiB" })),
      numericColumn("Exit", (row) => formatResultValue(row.exitCode)),
      textColumn("Timed out", "timedOut"),
      statusColumn("Succeeded", "succeeded"),
    ],
    model.samples,
    "No process samples are present in this bundle.",
  );

  renderNative(model);
  renderBrowser(model);
  renderGuidance(model);
  renderDiagnostics(model.diagnostics);

  renderTable(
    mustElement("#artifacts-table"),
    [
      textColumn("Artifact", "path", "code"),
      textColumn("Media type", "mediaType"),
      statusColumn("Verified", "verified"),
      textColumn("SHA-256", "sha256", "code"),
      {
        label: "Diagnostics",
        render: (row) => (row.diagnostics as string[]).join("; ") || "—",
      },
    ],
    model.artifacts,
    "No declared artifacts were found in the manifest.",
  );
}

function renderNative(model: ResultModel): void {
  const section = mustElement<HTMLElement>("#native-profile");
  const collectors = model.identity.find((row) => row.label === "Collectors")?.value ?? "";
  const visible =
    String(collectors).split(", ").includes("native-perf") ||
    model.nativeProfile.status === "collected" ||
    model.nativeProfile.rows.length > 0;
  section.hidden = !visible;
  if (!visible) return;

  const profile = model.nativeProfile;
  mustElement<HTMLElement>("#native-note").textContent =
    profile.status === "collected"
      ? "Bounded source-level evidence emitted by the native collector."
      : profile.reason ?? "Native profiling evidence was not collected.";
  renderInlineDetails(mustElement("#native-details"), [
    ["collector", profile.collector],
    ["event", profile.event],
    ["metric", profile.metric],
    ["total weight", profile.totalWeight],
    ["total samples", profile.totalSamples],
    ["truncated", profile.truncated],
  ]);
  renderTable(
    mustElement("#hotspots-table"),
    [
      textColumn("Symbol", "symbol", "code"),
      {
        label: "Source",
        className: "code",
        render: (row) =>
          row.sourceFile ? `${String(row.sourceFile)}${row.line === null ? "" : `:${String(row.line)}`}` : "—",
      },
      numericColumn("Weight", (row) => formatResultValue(row.weight)),
      numericColumn("Samples", (row) => formatResultValue(row.samples)),
      textColumn("Confidence", "confidence"),
      textColumn("Evidence", "evidenceRef", "code"),
    ],
    profile.rows,
    profile.reason ?? "No native hotspots were emitted.",
  );
}

function renderBrowser(model: ResultModel): void {
  const section = mustElement<HTMLElement>("#browser-profile");
  const profile = model.browserProfile;
  section.hidden = profile === null;
  if (!profile) return;

  const runtime = profile.runtime ?? {};
  const summary = profile.summary;
  const viewport = runtime.viewport
    ? `${String(runtime.viewport.width)}×${String(runtime.viewport.height)}`
    : null;
  const truncated =
    summary.longTasksTruncated ||
    summary.hotPathsTruncated ||
    summary.hotPathDepthTruncated ||
    summary.boundaryMarkersTruncated;
  mustElement<HTMLElement>("#browser-note").textContent = truncated
    ? "Normalized renderer-main evidence. One or more bounded views are truncated; the indicators below identify which evidence is partial. Raw Chromium trace events remain in the immutable bundle."
    : "Normalized renderer-main evidence. Raw Chromium trace events remain in the immutable bundle.";
  renderInlineDetails(mustElement("#browser-details"), [
    ["browser", [runtime.browser_name, runtime.browser_version].filter(Boolean).join(" ") || null],
    ["viewport", viewport],
    ["trace events", summary.traceEventCount],
    ["top-level tasks", summary.topLevelTaskCount],
    ["top-level time", formatUs(summary.topLevelDurationUs)],
    ["long tasks", summary.longTaskCount],
    ["long task time", formatUs(summary.longTaskTotalDurationUs)],
    ["longest task", formatUs(summary.longestTaskUs)],
    ["long tasks truncated", summary.longTasksTruncated],
    ["hot paths", summary.hotPathCount],
    ["hot paths truncated", summary.hotPathsTruncated],
    ["hot-path depth truncated", summary.hotPathDepthTruncated],
    ["boundaries", summary.boundaryMarkerCount],
    ["boundaries truncated", summary.boundaryMarkersTruncated],
  ]);

  renderTable(
    mustElement("#runtime-table"),
    [
      textColumn("Runtime", "runtimeKind"),
      numericColumn("Inclusive time", (row) => formatUs(row.inclusiveDurationUs as number | null)),
      numericColumn("Events", (row) => formatResultValue(row.eventCount)),
    ],
    profile.runtimeAttribution,
    "No runtime-attribution rows were emitted.",
  );

  renderTable(
    mustElement("#long-tasks-table"),
    [
      textColumn("Task", "name", "code"),
      textColumn("Runtime", "runtimeKind"),
      numericColumn("Start", (row) => formatUs(row.startUs as number | null)),
      numericColumn("Duration", (row) => formatUs(row.durationUs as number | null)),
      textColumn("Evidence", "evidenceRef", "code"),
    ],
    profile.longTasks,
    "No bounded long-task rows were emitted.",
  );

  renderTable(
    mustElement("#browser-hotpaths-table"),
    [
      textColumn("Path", "frames", "code"),
      textColumn("Leaf runtime", "runtimeKind"),
      numericColumn("Total time", (row) => formatUs(row.totalDurationUs as number | null)),
      numericColumn("Max", (row) => formatUs(row.maxDurationUs as number | null)),
      numericColumn("Occurrences", (row) => formatResultValue(row.occurrences)),
      textColumn("Evidence", "evidenceRef", "code"),
    ],
    profile.hotPaths,
    "No bounded browser hot paths were emitted.",
  );

  renderTable(
    mustElement("#boundaries-table"),
    [
      textColumn("Direction", "direction"),
      textColumn("Label", "label", "code"),
      numericColumn("Total time", (row) => formatUs(row.totalDurationUs as number | null)),
      numericColumn("Max", (row) => formatUs(row.maxDurationUs as number | null)),
      numericColumn("Occurrences", (row) => formatResultValue(row.occurrences)),
      textColumn("Evidence", "evidenceRef", "code"),
    ],
    profile.boundaryMarkers,
    "No explicit JS/WASM boundary markers were emitted.",
  );

  renderListSection("#browser-limitations", profile.limitations);
}

function renderGuidance(model: ResultModel): void {
  const section = mustElement<HTMLElement>("#guidance");
  const visible = model.guidance.observations.length > 0 || model.guidance.constraints.length > 0;
  section.hidden = !visible;
  if (!visible) return;

  renderTable(
    mustElement("#observations-table"),
    [
      textColumn("Observation", "summary"),
      textColumn("Evidence", "evidenceRef", "code"),
    ],
    model.guidance.observations,
    "No bounded observations were emitted.",
  );
  renderListSection("#constraints", model.guidance.constraints);
}

function renderDiagnostics(diagnostics: readonly string[]): void {
  const section = mustElement<HTMLElement>("#diagnostics");
  section.hidden = diagnostics.length === 0;
  if (section.hidden) return;
  const list = section.querySelector<HTMLUListElement>("ul");
  if (!list) throw new Error("Missing diagnostics list");
  renderList(list, diagnostics);
}

function renderDetails(
  container: Element,
  rows: readonly DetailRow[],
  formatter: (row: DetailRow) => string = (row) => formatResultValue(row.value),
): void {
  container.replaceChildren();
  for (const row of rows) {
    const wrapper = document.createElement("div");
    const term = document.createElement("dt");
    const value = document.createElement("dd");
    term.textContent = row.label;
    value.textContent = formatter(row);
    wrapper.append(term, value);
    container.append(wrapper);
  }
}

function renderInlineDetails(
  container: Element,
  entries: ReadonlyArray<readonly [string, unknown]>,
): void {
  container.replaceChildren();
  for (const [label, value] of entries) {
    const item = document.createElement("span");
    const strong = document.createElement("strong");
    strong.textContent = `${label}: `;
    item.append(strong, document.createTextNode(formatResultValue(value)));
    container.append(item);
  }
}

function renderListSection(selector: string, rows: readonly string[]): void {
  const container = mustElement<HTMLElement>(selector);
  container.hidden = rows.length === 0;
  if (!container.hidden) {
    const list = container.querySelector<HTMLUListElement>("ul");
    if (!list) throw new Error(`Missing list in ${selector}`);
    renderList(list, rows);
  }
}

function renderList(container: HTMLUListElement, rows: readonly string[]): void {
  container.replaceChildren();
  for (const row of rows) {
    const item = document.createElement("li");
    item.textContent = row;
    container.append(item);
  }
}

function renderTable(
  container: Element,
  columns: readonly TableColumn[],
  rows: readonly object[],
  emptyText: string,
): void {
  container.replaceChildren();
  if (rows.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = emptyText;
    container.append(empty);
    return;
  }

  const shell = document.createElement("div");
  shell.className = "table-shell";
  const table = document.createElement("table");
  const head = document.createElement("thead");
  const headRow = document.createElement("tr");
  for (const column of columns) {
    const th = document.createElement("th");
    th.textContent = column.label;
    if (column.numeric) th.classList.add("numeric");
    headRow.append(th);
  }
  head.append(headRow);

  const body = document.createElement("tbody");
  for (const sourceRow of rows) {
    const row = sourceRow as TableRow;
    const tr = document.createElement("tr");
    for (const column of columns) {
      const td = document.createElement("td");
      td.textContent = column.render(row);
      if (column.numeric) td.classList.add("numeric");
      if (column.className) td.classList.add(column.className);
      if (column.status) {
        td.classList.add(row[column.status] === true ? "status-ok" : "status-error");
      }
      tr.append(td);
    }
    body.append(tr);
  }
  table.append(head, body);
  shell.append(table);
  container.append(shell);
}

function textColumn(label: string, key: string, className = ""): TableColumn {
  return {
    label,
    className,
    render: (row) => formatResultValue(row[key]),
  };
}

function numericColumn(label: string, render: (row: TableRow) => string): TableColumn {
  return { label, numeric: true, render };
}

function statusColumn(label: string, key: string): TableColumn {
  return {
    label,
    status: key,
    render: (row) => formatResultValue(row[key]),
  };
}

function metricValue(value: unknown, unit: unknown): string {
  return formatResultValue(value, { unit: typeof unit === "string" ? unit : null });
}

function formatUs(value: number | null): string {
  if (value === null) return "—";
  return formatResultValue(value / 1000, { unit: "ms" });
}
