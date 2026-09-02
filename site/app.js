import { validateBundleUrl } from "./validator.mjs";

const form = document.querySelector("form");
const input = document.querySelector("#manifest");
const status = document.querySelector("#status");
const result = document.querySelector("#result");
const output = document.querySelector("#output");
const summary = document.querySelector("#summary");
const machineLink = document.querySelector("#machine-link");

const initial = new URL(location.href).searchParams.get("manifest");
if (initial) {
  input.value = initial;
  void run(initial);
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  void run(input.value);
});

async function run(manifestUrl) {
  status.textContent = "Loading and verifying public evidence…";
  status.dataset.state = "normal";
  result.hidden = true;
  try {
    const report = await validateBundleUrl(manifestUrl);
    const json = `${JSON.stringify(report, null, 2)}\n`;
    output.textContent = json;
    summary.replaceChildren(
      metric("Valid", report.valid ? "yes" : "no"),
      metric("Verified files", report.verified_files),
      metric("Metrics", report.summary.metric_count),
      metric("Samples", report.summary.sample_count),
    );
    machineLink.href = `./validate.json/?manifest=${encodeURIComponent(manifestUrl)}`;
    result.hidden = false;
    status.textContent = report.valid
      ? "Bundle satisfies the browser-verifiable v1 contract."
      : `Bundle has ${report.diagnostics.length} validation diagnostic(s).`;
    history.replaceState(null, "", `?manifest=${encodeURIComponent(manifestUrl)}`);
  } catch (error) {
    status.dataset.state = "error";
    status.textContent = error instanceof Error ? error.message : String(error);
  }
}

function metric(label, value) {
  const element = document.createElement("div");
  const strong = document.createElement("strong");
  strong.textContent = String(value);
  const span = document.createElement("span");
  span.textContent = label;
  element.append(strong, span);
  return element;
}
