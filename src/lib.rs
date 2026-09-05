#![forbid(unsafe_code)]

pub mod bundle;
pub mod capture;
pub mod contract;
pub mod digest;
pub mod evidence;
pub mod hotspot_compare;
pub mod http;
pub mod native_perf;
pub mod scenario;
pub mod score;

pub use bundle::{capture_bundle, render_agent_guidance, summarize_bundle, validate_bundle};
pub use evidence::{AgentEvidenceReference, build_agent_evidence_reference};
pub use hotspot_compare::{
    HotspotComparabilityReport, HotspotComparabilityStatus, compare_hotspot_bundles,
};
pub use scenario::{LoadedScenario, load_scenario};
pub use score::{RuntimeScoreDocument, score_bundles};
