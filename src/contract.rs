use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const SCENARIO_SCHEMA_V1: &str = "runtime-profiler/scenario/v1";
pub const SCENARIO_EVIDENCE_SCHEMA_V1: &str = "runtime-profiler/scenario-evidence/v1";
pub const MANIFEST_SCHEMA_V1: &str = "runtime-profiler/bundle-manifest/v1";
pub const ENVIRONMENT_SCHEMA_V1: &str = "runtime-profiler/environment/v1";
pub const METRICS_SCHEMA_V1: &str = "runtime-profiler/metrics/v1";
pub const PROCESS_MAX_RSS_V1_ID: &str = "process.max_rss";
pub const PROCESS_MAX_OBSERVED_RSS_ID: &str = "process.max_observed_rss";
pub const HOTSPOTS_SCHEMA_V1: &str = "runtime-profiler/hotspots/v1";
pub const GUIDANCE_SCHEMA_V1: &str = "runtime-profiler/agent-guidance/v1";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema_version: String,
    pub id: String,
    pub target: Target,
    #[serde(default)]
    pub run: RunConfig,
    #[serde(default = "default_collectors")]
    pub collectors: Vec<Collector>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Target {
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_directory: Option<PathBuf>,
        #[serde(default)]
        inherit_env: Vec<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    #[serde(default = "default_warmup_iterations")]
    pub warmup_iterations: u32,
    #[serde(default = "default_measurement_iterations")]
    pub measurement_iterations: u32,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: default_warmup_iterations(),
            measurement_iterations: default_measurement_iterations(),
            timeout_seconds: default_timeout_seconds(),
        }
    }
}

const fn default_warmup_iterations() -> u32 {
    1
}

const fn default_measurement_iterations() -> u32 {
    5
}

const fn default_timeout_seconds() -> u64 {
    30
}

fn default_collectors() -> Vec<Collector> {
    vec![Collector::Process]
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Collector {
    Process,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturePlan {
    pub schema_version: String,
    pub scenario_id: String,
    pub scenario_digest: String,
    pub target_type: String,
    pub collectors: Vec<CollectorPlan>,
    pub warmup_iterations: u32,
    pub measurement_iterations: u32,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollectorPlan {
    pub id: String,
    pub supported: bool,
    pub measurements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioEvidence {
    pub schema_version: String,
    pub id: String,
    pub digest: String,
    pub target: TargetEvidence,
    pub run: RunConfig,
    pub collectors: Vec<Collector>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetEvidence {
    pub target_type: String,
    pub program: String,
    pub argument_count: usize,
    pub working_directory_set: bool,
    pub inherited_environment_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDocument {
    pub schema_version: String,
    pub fingerprint: String,
    pub operating_system: String,
    pub architecture: String,
    pub kernel_release: Option<String>,
    pub logical_cpu_count: usize,
    pub source: SourceIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceIdentity {
    pub git_sha: Option<String>,
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsDocument {
    pub schema_version: String,
    pub scenario_id: String,
    pub samples: Vec<MeasurementSample>,
    pub metrics: Vec<MetricSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeasurementSample {
    pub iteration: u32,
    pub duration_ms: f64,
    pub max_rss_kib: Option<u64>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub succeeded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricSummary {
    pub id: String,
    pub unit: String,
    pub preferred_direction: PreferredDirection,
    pub statistics: Statistics,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreferredDirection {
    Lower,
    Higher,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Statistics {
    pub sample_count: usize,
    pub minimum: f64,
    pub maximum: f64,
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HotspotsDocument {
    pub schema_version: String,
    pub status: String,
    pub reason: String,
    pub hotspots: Vec<Hotspot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hotspot {
    pub symbol: String,
    pub source_file: Option<String>,
    pub line: Option<u32>,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentGuidance {
    pub schema_version: String,
    pub scenario_id: String,
    pub observations: Vec<AgentObservation>,
    pub constraints: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentObservation {
    pub id: String,
    pub summary: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleManifest {
    pub schema_version: String,
    pub bundle_id: String,
    pub created_unix_ms: u128,
    pub scenario_id: String,
    pub scenario_digest: String,
    pub environment_fingerprint: String,
    pub source: SourceIdentity,
    pub files: Vec<ArtifactEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactEntry {
    pub path: String,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationReport {
    pub schema_version: String,
    pub bundle_id: String,
    pub valid: bool,
    pub verified_files: usize,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectionReport {
    pub schema_version: String,
    pub platform: String,
    pub collectors: BTreeMap<String, Detection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Detection {
    pub available: bool,
    pub reason: String,
}
