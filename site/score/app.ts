import {
  chartPoints,
  latestEntry,
  metricChange,
  normalizeHistory,
  shortCommit,
  type ScoreEntry,
} from "./model.js";

const HISTORY_URL =
  "https://raw.githubusercontent.com/moritzbrantner/runtime-profiler/score-history/history.json";

function mustElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing required element: ${selector}`);
  return element;
}

const status = mustElement<HTMLElement>("#status");
const latest = mustElement<HTMLElement>("#latest-score");
const rating = mustElement<HTMLElement>("#latest-rating");
const averageChange = mustElement<HTMLElement>("#average-change");
const commitLink = mustElement<HTMLAnchorElement>("#latest-commit");
const parentLink = mustElement<HTMLAnchorElement>("#parent-commit");
const chart = mustElement<SVGSVGElement>("#score-chart");
const metrics = mustElement<HTMLElement>("#metrics");
const historyBody = mustElement<HTMLTableSectionElement>("#history-body");

function scoreText(value: number | null): string {
  return value === null ? "—" : String(value);
}

function signedPercent(value: number | null): string {
  if (value === null) return "—";
  if (value === 0) return "±0.000%";
  return `${value > 0 ? "+" : ""}${value.toFixed(3)}%`;
}

function ratingText(value: string | null): string {
  return String(value ?? "unavailable").replaceAll("-", " ");
}

function renderChart(entries: ScoreEntry[]): void {
  const points = chartPoints(entries.slice(-60));
  if (points.length === 0) {
    chart.innerHTML = '<text x="360" y="120" text-anchor="middle">No comparable commits yet</text>';
    return;
  }
  const guides = [0, 25, 50, 75, 100]
    .map((score) => {
      const y = 24 + ((100 - score) / 100) * 192;
      return `<g class="guide"><line x1="24" y1="${y}" x2="696" y2="${y}"/><text x="2" y="${y + 4}">${score}</text></g>`;
    })
    .join("");
  const line = points.map((point) => `${point.x},${point.y}`).join(" ");
  const dots = points
    .map(
      (point) =>
        `<circle cx="${point.x}" cy="${point.y}" r="4"><title>${shortCommit(point.commit)} · ${point.score}/100 vs parent</title></circle>`,
    )
    .join("");
  chart.innerHTML = `${guides}<polyline class="series" points="${line}"/>${dots}`;
}

function renderMetrics(entry: ScoreEntry): void {
  const values = entry.metrics;
  metrics.innerHTML = values.length
    ? values
        .map(
          (metric) => `
            <article class="metric-card">
              <div>
                <span>${metric.id ?? "—"}</span>
                <strong>${scoreText(metric.score)}</strong>
              </div>
              <div class="metric-change">
                <span>average change</span>
                <strong>${signedPercent(metricChange(metric))}</strong>
              </div>
              <div class="statistics">
                ${metric.statistics
                  .map(
                    (statistic) => `
                      <div>
                        <span>${statistic.statistic ?? "—"}</span>
                        <strong>${signedPercent(statistic.change_percent)}</strong>
                        <small>${scoreText(statistic.score)}/100</small>
                      </div>`,
                  )
                  .join("")}
              </div>
            </article>`,
        )
        .join("")
    : '<p class="muted">No comparable metric evidence for the latest commit.</p>';
}

function renderTable(entries: ScoreEntry[]): void {
  historyBody.innerHTML = entries
    .slice(-20)
    .reverse()
    .map(
      (entry) => `
        <tr>
          <td><a href="https://github.com/moritzbrantner/runtime-profiler/commit/${entry.commit}">${shortCommit(entry.commit)}</a></td>
          <td><a href="https://github.com/moritzbrantner/runtime-profiler/commit/${entry.parent_commit}">${shortCommit(entry.parent_commit)}</a></td>
          <td>${new Date(entry.timestamp).toLocaleString()}</td>
          <td><strong>${scoreText(entry.score)}</strong></td>
          <td>${signedPercent(entry.average_change_percent)}</td>
          <td>${entry.status ?? "—"}</td>
        </tr>`,
    )
    .join("");
}

async function load(): Promise<void> {
  try {
    const response = await fetch(`${HISTORY_URL}?t=${Date.now()}`, { cache: "no-store" });
    if (!response.ok) throw new Error(`history request failed with ${response.status}`);
    const history = normalizeHistory(await response.json());
    const entry = latestEntry(history);
    if (!entry) {
      status.textContent = "The history branch exists, but no runtime comparison has been published yet.";
      return;
    }

    latest.textContent = scoreText(entry.score);
    rating.textContent =
      entry.status === "scored"
        ? `${ratingText(entry.rating)} · retention versus first parent`
        : `comparison unavailable · ${entry.reason ?? "no reason recorded"}`;
    averageChange.textContent = signedPercent(entry.average_change_percent);
    commitLink.textContent = shortCommit(entry.commit);
    commitLink.href = `https://github.com/moritzbrantner/runtime-profiler/commit/${entry.commit}`;
    parentLink.textContent = shortCommit(entry.parent_commit);
    parentLink.href = `https://github.com/moritzbrantner/runtime-profiler/commit/${entry.parent_commit}`;
    renderChart(history.entries);
    renderMetrics(entry);
    renderTable(history.entries);
    status.textContent = `${history.entries.length} commit comparison${history.entries.length === 1 ? "" : "s"} retained.`;
  } catch (error: unknown) {
    status.textContent = `Runtime score history is not available yet: ${error instanceof Error ? error.message : String(error)}`;
  }
}

void load();
