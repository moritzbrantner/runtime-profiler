use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use runtime_profiler::capture::install_cli_interruption_handlers;
use runtime_profiler::contract::{Detection, DetectionReport, MetricsDocument};
use runtime_profiler::{
    HotspotComparabilityReport, HotspotComparabilityStatus, RuntimeScoreDocument,
    build_agent_evidence_reference, capture_bundle, compare_hotspot_bundles, load_scenario,
    render_agent_guidance, score_bundles, summarize_bundle, validate_bundle,
};

#[derive(Debug, Parser)]
#[command(
    name = "runtime-profiler",
    version,
    about = "Capture deterministic runtime evidence for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Detect implemented collectors and future adapter prerequisites.
    Detect,
    /// Validate a scenario and print the deterministic capture plan.
    Plan {
        #[arg(long)]
        scenario: PathBuf,
    },
    /// Execute a scenario and write an immutable evidence bundle.
    Capture {
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify integrity and compatibility of an evidence bundle.
    Validate {
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Print the measurements contained in a valid evidence bundle.
    Summarize {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Score a candidate bundle relative to a comparable reference bundle.
    Score {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Report whether two hotspot bundles have enough compatible identity to compare.
    CompareHotspots {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Render bounded Markdown guidance from a valid evidence bundle.
    RenderAgentGuidance {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Emit an agent.evidence/v1 reference to a valid immutable bundle.
    EvidenceReference {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        uri: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Detect => print_json(&detect()),
        Commands::Plan { scenario } => {
            let loaded = load_scenario(&scenario)?;
            print_json(&loaded.plan());
        }
        Commands::Capture { scenario, output } => {
            install_cli_interruption_handlers()?;
            let manifest = capture_bundle(&scenario, &output)?;
            print_json(&manifest);
        }
        Commands::Validate { bundle } => {
            let report = validate_bundle(&bundle)?;
            print_json(&report);
            ensure!(report.valid, "bundle validation failed");
        }
        Commands::Summarize { bundle, json } => {
            let metrics = summarize_bundle(&bundle)?;
            if json {
                print_json(&metrics);
            } else {
                print_summary(&metrics);
            }
        }
        Commands::Score {
            reference,
            candidate,
            json,
        } => {
            let score = score_bundles(&reference, &candidate)?;
            if json {
                print_json(&score);
            } else {
                print_score(&score);
            }
        }
        Commands::CompareHotspots {
            reference,
            candidate,
            json,
        } => {
            let report = compare_hotspot_bundles(&reference, &candidate)?;
            if json {
                print_json(&report);
            } else {
                print_hotspot_comparability(&report);
            }
        }
        Commands::RenderAgentGuidance { bundle, output } => {
            let rendered = render_agent_guidance(&bundle)?;
            if let Some(path) = output {
                ensure!(
                    !path.exists(),
                    "refusing to overwrite existing guidance: {}",
                    path.display()
                );
                fs::write(&path, rendered)
                    .with_context(|| format!("failed to write guidance: {}", path.display()))?;
            } else {
                print!("{rendered}");
            }
        }
        Commands::EvidenceReference { bundle, uri } => {
            print_json(&build_agent_evidence_reference(&bundle, uri)?);
        }
    }
    Ok(())
}

fn detect() -> DetectionReport {
    let mut collectors = BTreeMap::new();
    collectors.insert(
        "process".to_owned(),
        Detection {
            available: true,
            reason: "implemented: wall time, exit state, timeout, and Linux maximum observed RSS"
                .to_owned(),
            tool_version: None,
        },
    );
    collectors.insert(
        "dotnet-eventpipe".to_owned(),
        planned_detection("dotnet", "planned .NET EventPipe adapter"),
    );
    collectors.insert(
        runtime_profiler::native_perf::COLLECTOR_ID.to_owned(),
        runtime_profiler::native_perf::detect_native_perf(),
    );
    collectors.insert(
        "browser-playwright".to_owned(),
        planned_detection("bun", "planned Bun and Playwright journey adapter"),
    );

    DetectionReport {
        schema_version: "runtime-profiler/detection/v1".to_owned(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        collectors,
    }
}

fn planned_detection(program: &str, adapter: &str) -> Detection {
    let prerequisite = program_available(program);
    Detection {
        available: false,
        reason: format!(
            "not implemented in 0.1: {adapter}; prerequisite `{program}` {}",
            if prerequisite {
                "detected"
            } else {
                "not detected"
            }
        ),
        tool_version: None,
    }
}

fn program_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn print_json<T: serde::Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serializable CLI response")
    );
}

fn print_summary(metrics: &MetricsDocument) {
    println!("Scenario: {}", metrics.scenario_id);
    for metric in &metrics.metrics {
        println!(
            "{}: median={:.3} {}, p95={:.3} {}, mean={:.3} {}, samples={}",
            metric.id,
            metric.statistics.median,
            metric.unit,
            metric.statistics.p95,
            metric.unit,
            metric.statistics.mean,
            metric.unit,
            metric.statistics.sample_count
        );
    }
}

fn print_score(score: &RuntimeScoreDocument) {
    println!("Scenario: {}", score.scenario_id);
    println!("Runtime score: {}/100 ({:?})", score.score, score.rating);
    for metric in &score.metrics {
        println!("{}: {}/100", metric.id, metric.score);
        for statistic in &metric.statistics {
            let change = statistic
                .change_percent
                .map(|value| format!("{value:+.3}%"))
                .unwrap_or_else(|| "n/a".to_owned());
            println!(
                "  {}: reference={:.3}, candidate={:.3}, change={}, score={}/100",
                statistic.statistic,
                statistic.reference,
                statistic.candidate,
                change,
                statistic.score
            );
        }
    }
}

fn print_hotspot_comparability(report: &HotspotComparabilityReport) {
    let status = match report.status {
        HotspotComparabilityStatus::Comparable => "comparable",
        HotspotComparabilityStatus::Incomparable => "incomparable",
        HotspotComparabilityStatus::InsufficientEvidence => "insufficient-evidence",
    };
    println!("Hotspot comparability: {status}");
    for reason in &report.reasons {
        println!("- {reason}");
    }
}
