use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::bundle::validate_bundle;
use crate::contract::{
    BundleManifest, MetricSummary, MetricsDocument, PROCESS_MAX_OBSERVED_RSS_ID,
    PROCESS_MAX_RSS_V1_ID, PreferredDirection, SourceIdentity,
};

pub const RUNTIME_SCORE_SCHEMA_V1: &str = "runtime-profiler/score/v1";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreRating {
    Good,
    NeedsImprovement,
    Poor,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeScoreDocument {
    pub schema_version: String,
    pub scenario_id: String,
    pub scenario_digest: String,
    pub environment_fingerprint_schema_version: String,
    pub environment_fingerprint: String,
    pub reference_bundle_id: String,
    pub candidate_bundle_id: String,
    pub reference_source: SourceIdentity,
    pub candidate_source: SourceIdentity,
    pub score: u8,
    pub rating: ScoreRating,
    pub metrics: Vec<RuntimeMetricScore>,
    pub excluded_metrics: Vec<ExcludedMetric>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeMetricScore {
    pub id: String,
    pub unit: String,
    pub preferred_direction: PreferredDirection,
    pub score: u8,
    pub statistics: Vec<RuntimeStatisticScore>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeStatisticScore {
    pub statistic: String,
    pub reference: f64,
    pub candidate: f64,
    pub change_percent: Option<f64>,
    pub score: u8,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExcludedMetric {
    pub id: String,
    pub reason: String,
}

#[derive(Debug)]
struct CanonicalMetrics {
    metrics: BTreeMap<String, MetricSummary>,
    excluded: BTreeSet<ExcludedMetric>,
}

pub fn score_bundles(reference: &Path, candidate: &Path) -> Result<RuntimeScoreDocument> {
    validate(reference, "reference")?;
    validate(candidate, "candidate")?;

    let reference_manifest: BundleManifest = read_json(&reference.join("manifest.json"))?;
    let candidate_manifest: BundleManifest = read_json(&candidate.join("manifest.json"))?;
    ensure!(
        reference_manifest.scenario_digest == candidate_manifest.scenario_digest,
        "runtime score requires identical scenario digests (reference={}, candidate={})",
        reference_manifest.scenario_digest,
        candidate_manifest.scenario_digest
    );
    ensure!(
        reference_manifest.environment_fingerprint_schema_version
            == candidate_manifest.environment_fingerprint_schema_version,
        "runtime score requires identical environment fingerprint schema versions"
    );
    ensure!(
        reference_manifest.environment_fingerprint == candidate_manifest.environment_fingerprint,
        "runtime score requires identical environment fingerprints"
    );

    let reference_metrics: MetricsDocument = read_json(&reference.join("metrics.json"))?;
    let candidate_metrics: MetricsDocument = read_json(&candidate.join("metrics.json"))?;
    let (metrics, excluded_metrics) = score_metrics(&reference_metrics, &candidate_metrics)?;
    let score = average_score(metrics.iter().map(|metric| metric.score));

    Ok(RuntimeScoreDocument {
        schema_version: RUNTIME_SCORE_SCHEMA_V1.to_owned(),
        scenario_id: reference_manifest.scenario_id.clone(),
        scenario_digest: reference_manifest.scenario_digest.clone(),
        environment_fingerprint_schema_version: reference_manifest
            .environment_fingerprint_schema_version
            .clone(),
        environment_fingerprint: reference_manifest.environment_fingerprint.clone(),
        reference_bundle_id: reference_manifest.bundle_id,
        candidate_bundle_id: candidate_manifest.bundle_id,
        reference_source: reference_manifest.source,
        candidate_source: candidate_manifest.source,
        score,
        rating: rating(score),
        metrics,
        excluded_metrics,
        notes: vec![
            "The score is reference-relative: 100 means the candidate meets or beats the reference on the scored runtime evidence.".to_owned(),
            "Positive change_percent values are improvements and negative values are regressions; improvements remain visible even though component scores are capped at 100.".to_owned(),
            "Wall-time and memory-like metrics score median and p95 behavior; process.success_rate scores its mean so intermittent failures stay visible.".to_owned(),
            "Only bundles with identical scenario and environment fingerprints are comparable; the process.max_rss compatibility alias is never double-weighted.".to_owned(),
        ],
    })
}

fn validate(bundle: &Path, label: &str) -> Result<()> {
    let report = validate_bundle(bundle)
        .with_context(|| format!("failed to validate {label} bundle: {}", bundle.display()))?;
    ensure!(
        report.valid,
        "{label} bundle is invalid: {}",
        report.diagnostics.join("; ")
    );
    Ok(())
}

fn score_metrics(
    reference: &MetricsDocument,
    candidate: &MetricsDocument,
) -> Result<(Vec<RuntimeMetricScore>, Vec<ExcludedMetric>)> {
    ensure!(
        reference.scenario_id == candidate.scenario_id,
        "metrics scenario ids differ (reference={}, candidate={})",
        reference.scenario_id,
        candidate.scenario_id
    );
    let reference = canonical_metrics(reference)?;
    let candidate = canonical_metrics(candidate)?;
    ensure!(
        reference.metrics.keys().eq(candidate.metrics.keys()),
        "runtime score requires identical canonical metric sets"
    );

    let mut scores = Vec::with_capacity(reference.metrics.len());
    for (id, reference_metric) in &reference.metrics {
        let candidate_metric = candidate
            .metrics
            .get(id)
            .expect("canonical metric sets were checked for equality");
        ensure!(
            reference_metric.unit == candidate_metric.unit,
            "metric {id} unit differs (reference={}, candidate={})",
            reference_metric.unit,
            candidate_metric.unit
        );
        ensure!(
            reference_metric.preferred_direction == candidate_metric.preferred_direction,
            "metric {id} preferred direction differs"
        );

        let statistics = if id == "process.success_rate" {
            vec![score_statistic(
                "mean",
                reference_metric.statistics.mean,
                candidate_metric.statistics.mean,
                reference_metric.preferred_direction,
            )?]
        } else {
            vec![
                score_statistic(
                    "median",
                    reference_metric.statistics.median,
                    candidate_metric.statistics.median,
                    reference_metric.preferred_direction,
                )?,
                score_statistic(
                    "p95",
                    reference_metric.statistics.p95,
                    candidate_metric.statistics.p95,
                    reference_metric.preferred_direction,
                )?,
            ]
        };
        let score = average_score(statistics.iter().map(|statistic| statistic.score));
        scores.push(RuntimeMetricScore {
            id: id.clone(),
            unit: reference_metric.unit.clone(),
            preferred_direction: reference_metric.preferred_direction,
            score,
            statistics,
        });
    }

    let excluded_metrics = reference
        .excluded
        .union(&candidate.excluded)
        .cloned()
        .collect();
    Ok((scores, excluded_metrics))
}

fn canonical_metrics(document: &MetricsDocument) -> Result<CanonicalMetrics> {
    let observed_rss = document
        .metrics
        .iter()
        .find(|metric| metric.id == PROCESS_MAX_OBSERVED_RSS_ID);
    let legacy_rss = document
        .metrics
        .iter()
        .find(|metric| metric.id == PROCESS_MAX_RSS_V1_ID);
    if let (Some(observed), Some(legacy)) = (observed_rss, legacy_rss) {
        ensure!(
            observed.unit == legacy.unit
                && observed.preferred_direction == legacy.preferred_direction
                && observed.statistics == legacy.statistics,
            "{PROCESS_MAX_RSS_V1_ID} compatibility alias differs from {PROCESS_MAX_OBSERVED_RSS_ID}"
        );
    }

    let mut metrics = BTreeMap::new();
    let mut excluded = BTreeSet::new();
    for metric in &document.metrics {
        if metric.id == PROCESS_MAX_RSS_V1_ID && observed_rss.is_some() {
            excluded.insert(ExcludedMetric {
                id: PROCESS_MAX_RSS_V1_ID.to_owned(),
                reason: format!(
                    "compatibility alias of {PROCESS_MAX_OBSERVED_RSS_ID}; excluded to avoid double-weighting"
                ),
            });
            continue;
        }
        let mut metric = metric.clone();
        if metric.id == PROCESS_MAX_RSS_V1_ID {
            metric.id = PROCESS_MAX_OBSERVED_RSS_ID.to_owned();
        }
        ensure!(
            metrics.insert(metric.id.clone(), metric).is_none(),
            "duplicate canonical metric id in metrics document"
        );
    }
    ensure!(!metrics.is_empty(), "metrics document has no scoreable metrics");
    Ok(CanonicalMetrics { metrics, excluded })
}

fn score_statistic(
    statistic: &str,
    reference: f64,
    candidate: f64,
    direction: PreferredDirection,
) -> Result<RuntimeStatisticScore> {
    ensure!(reference.is_finite(), "reference {statistic} is not finite");
    ensure!(candidate.is_finite(), "candidate {statistic} is not finite");
    ensure!(
        reference >= 0.0 && candidate >= 0.0,
        "runtime score v1 requires non-negative {statistic} values"
    );

    Ok(RuntimeStatisticScore {
        statistic: statistic.to_owned(),
        reference,
        candidate,
        change_percent: change_percent(reference, candidate, direction),
        score: retention_score(reference, candidate, direction),
    })
}

fn retention_score(reference: f64, candidate: f64, direction: PreferredDirection) -> u8 {
    if reference == candidate {
        return 100;
    }
    let ratio = match direction {
        PreferredDirection::Lower => {
            if candidate == 0.0 {
                1.0
            } else if reference == 0.0 {
                0.0
            } else {
                reference / candidate
            }
        }
        PreferredDirection::Higher => {
            if reference == 0.0 {
                1.0
            } else if candidate == 0.0 {
                0.0
            } else {
                candidate / reference
            }
        }
    };
    (ratio.clamp(0.0, 1.0) * 100.0).round() as u8
}

fn change_percent(reference: f64, candidate: f64, direction: PreferredDirection) -> Option<f64> {
    if reference == 0.0 {
        return (candidate == 0.0).then_some(0.0);
    }
    let value = match direction {
        PreferredDirection::Lower => (reference - candidate) / reference * 100.0,
        PreferredDirection::Higher => (candidate - reference) / reference * 100.0,
    };
    Some(round_three(value))
}

fn average_score(scores: impl Iterator<Item = u8>) -> u8 {
    let scores: Vec<u8> = scores.collect();
    debug_assert!(!scores.is_empty());
    (scores.iter().map(|score| u32::from(*score)).sum::<u32>() as f64 / scores.len() as f64)
        .round() as u8
}

fn rating(score: u8) -> ScoreRating {
    if score >= 90 {
        ScoreRating::Good
    } else if score >= 50 {
        ScoreRating::NeedsImprovement
    } else {
        ScoreRating::Poor
    }
}

fn round_three(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{MeasurementSample, Statistics};

    fn metric(
        id: &str,
        direction: PreferredDirection,
        mean: f64,
        median: f64,
        p95: f64,
    ) -> MetricSummary {
        MetricSummary {
            id: id.to_owned(),
            unit: if id == "process.success_rate" { "ratio" } else { "ms" }.to_owned(),
            preferred_direction: direction,
            statistics: Statistics {
                sample_count: 5,
                minimum: median,
                maximum: p95,
                mean,
                median,
                p95,
            },
        }
    }

    fn document(metrics: Vec<MetricSummary>) -> MetricsDocument {
        MetricsDocument {
            schema_version: "runtime-profiler/metrics/v1".to_owned(),
            scenario_id: "example".to_owned(),
            samples: vec![MeasurementSample {
                iteration: 1,
                duration_ms: 1.0,
                max_rss_kib: None,
                exit_code: Some(0),
                timed_out: false,
                succeeded: true,
            }],
            metrics,
        }
    }

    #[test]
    fn equal_runtime_evidence_scores_one_hundred() {
        let reference = document(vec![
            metric("process.wall_time", PreferredDirection::Lower, 100.0, 100.0, 120.0),
            metric("process.success_rate", PreferredDirection::Higher, 1.0, 1.0, 1.0),
        ]);
        let (metrics, _) = score_metrics(&reference, &reference).expect("score");
        assert!(metrics.iter().all(|metric| metric.score == 100));
    }

    #[test]
    fn regression_is_proportional_and_improvements_cap_at_one_hundred() {
        assert_eq!(retention_score(100.0, 125.0, PreferredDirection::Lower), 80);
        assert_eq!(retention_score(100.0, 80.0, PreferredDirection::Lower), 100);
        assert_eq!(retention_score(1.0, 0.8, PreferredDirection::Higher), 80);
        assert_eq!(change_percent(100.0, 125.0, PreferredDirection::Lower), Some(-25.0));
        assert_eq!(change_percent(100.0, 80.0, PreferredDirection::Lower), Some(20.0));
    }

    #[test]
    fn success_rate_uses_mean_instead_of_binary_median() {
        let reference = document(vec![
            metric("process.wall_time", PreferredDirection::Lower, 100.0, 100.0, 100.0),
            metric("process.success_rate", PreferredDirection::Higher, 1.0, 1.0, 1.0),
        ]);
        let candidate = document(vec![
            metric("process.wall_time", PreferredDirection::Lower, 100.0, 100.0, 100.0),
            metric("process.success_rate", PreferredDirection::Higher, 0.8, 1.0, 1.0),
        ]);
        let (metrics, _) = score_metrics(&reference, &candidate).expect("score");
        let success = metrics
            .iter()
            .find(|metric| metric.id == "process.success_rate")
            .expect("success rate");
        assert_eq!(success.score, 80);
        assert_eq!(success.statistics.len(), 1);
        assert_eq!(success.statistics[0].statistic, "mean");
    }

    #[test]
    fn rss_compatibility_alias_is_not_double_weighted() {
        let observed = metric(
            PROCESS_MAX_OBSERVED_RSS_ID,
            PreferredDirection::Lower,
            10.0,
            10.0,
            12.0,
        );
        let mut legacy = observed.clone();
        legacy.id = PROCESS_MAX_RSS_V1_ID.to_owned();
        let canonical = canonical_metrics(&document(vec![observed, legacy])).expect("canonical");
        assert_eq!(canonical.metrics.len(), 1);
        assert_eq!(canonical.excluded.len(), 1);
    }

    #[test]
    fn metric_set_mismatch_is_not_silently_excluded() {
        let reference = document(vec![metric(
            "process.wall_time",
            PreferredDirection::Lower,
            100.0,
            100.0,
            100.0,
        )]);
        let candidate = document(vec![
            metric("process.wall_time", PreferredDirection::Lower, 100.0, 100.0, 100.0),
            metric("process.success_rate", PreferredDirection::Higher, 1.0, 1.0, 1.0),
        ]);
        assert!(score_metrics(&reference, &candidate).is_err());
    }
}
