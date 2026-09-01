use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

#[cfg(unix)]
use signal_hook::SigId;
#[cfg(unix)]
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::contract::{
    METRICS_SCHEMA_V1, MeasurementSample, MetricSummary, MetricsDocument,
    PROCESS_MAX_OBSERVED_RSS_ID, PROCESS_MAX_RSS_V1_ID, PreferredDirection, Statistics, Target,
};
use crate::scenario::LoadedScenario;

pub fn capture_metrics(loaded: &LoadedScenario) -> Result<MetricsDocument> {
    for warmup in 0..loaded.scenario.run.warmup_iterations {
        let result = execute_once(loaded, warmup + 1)
            .with_context(|| format!("warm-up iteration {} failed to execute", warmup + 1))?;
        if !result.succeeded {
            bail!(
                "warm-up iteration {} did not succeed (exit_code={:?}, timed_out={})",
                warmup + 1,
                result.exit_code,
                result.timed_out
            );
        }
    }

    let mut samples = Vec::with_capacity(loaded.scenario.run.measurement_iterations as usize);
    for iteration in 1..=loaded.scenario.run.measurement_iterations {
        samples.push(
            execute_once(loaded, iteration)
                .with_context(|| format!("measurement iteration {iteration} failed to execute"))?,
        );
    }

    let durations: Vec<f64> = samples.iter().map(|sample| sample.duration_ms).collect();
    let successes: Vec<f64> = samples
        .iter()
        .map(|sample| if sample.succeeded { 1.0 } else { 0.0 })
        .collect();
    let mut metrics = vec![
        MetricSummary {
            id: "process.wall_time".to_owned(),
            unit: "ms".to_owned(),
            preferred_direction: PreferredDirection::Lower,
            statistics: statistics(&durations),
        },
        MetricSummary {
            id: "process.success_rate".to_owned(),
            unit: "ratio".to_owned(),
            preferred_direction: PreferredDirection::Higher,
            statistics: statistics(&successes),
        },
    ];

    metrics.extend(memory_metric_summaries(&samples));

    Ok(MetricsDocument {
        schema_version: METRICS_SCHEMA_V1.to_owned(),
        scenario_id: loaded.scenario.id.clone(),
        samples,
        metrics,
    })
}

fn memory_metric_summaries(samples: &[MeasurementSample]) -> Vec<MetricSummary> {
    let resident_memory: Vec<f64> = samples
        .iter()
        .filter_map(|sample| sample.max_rss_kib.map(|value| value as f64))
        .collect();
    if resident_memory.is_empty() {
        Vec::new()
    } else {
        let summary = statistics(&resident_memory);
        [PROCESS_MAX_RSS_V1_ID, PROCESS_MAX_OBSERVED_RSS_ID]
            .into_iter()
            .map(|id| MetricSummary {
                id: id.to_owned(),
                unit: "KiB".to_owned(),
                preferred_direction: PreferredDirection::Lower,
                statistics: summary.clone(),
            })
            .collect()
    }
}

fn execute_once(loaded: &LoadedScenario, iteration: u32) -> Result<MeasurementSample> {
    let Target::Command {
        program,
        args,
        working_directory,
        inherit_env,
    } = &loaded.scenario.target;

    let mut command = Command::new(program);
    command.args(args);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env_clear();
    if let Some(path) = env::var_os("PATH") {
        command.env("PATH", path);
    }
    for name in inherit_env {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(directory) = resolve_working_directory(loaded, working_directory.as_deref()) {
        command.current_dir(directory);
    }
    #[cfg(unix)]
    command.process_group(0);

    #[cfg(unix)]
    let interruption = InterruptionGuard::install()?;

    let start = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start target program: {program}"))?;
    let timeout = Duration::from_secs(loaded.scenario.run.timeout_seconds);
    let mut max_observed_rss_kib = read_resident_memory_kib(child.id());
    let mut timed_out = false;

    let status = loop {
        #[cfg(unix)]
        if interruption.received() {
            terminate_process(&mut child)?;
            child
                .wait()
                .context("failed to reap interrupted target process")?;
            bail!("capture interrupted");
        }

        if let Some(status) = child.try_wait().context("failed to poll target process")? {
            break status;
        }

        max_observed_rss_kib =
            max_optional(max_observed_rss_kib, read_resident_memory_kib(child.id()));
        if start.elapsed() >= timeout {
            timed_out = true;
            terminate_process(&mut child)?;
            break child
                .wait()
                .context("failed to reap timed-out target process")?;
        }

        thread::sleep(Duration::from_millis(5));
    };

    let duration_ms = start.elapsed().as_secs_f64() * 1_000.0;
    Ok(sample_from_status(
        iteration,
        duration_ms,
        max_observed_rss_kib,
        status,
        timed_out,
    ))
}

fn terminate_process(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let group_kill = Command::new("kill")
            .args(["-KILL", "--", &process_group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(group_kill, Ok(status) if status.success()) {
            return Ok(());
        }
    }

    child.kill().context("failed to terminate target process")
}

#[cfg(unix)]
struct InterruptionGuard {
    received: Arc<AtomicBool>,
    registrations: Vec<SigId>,
}

#[cfg(unix)]
impl InterruptionGuard {
    fn install() -> Result<Self> {
        let received = Arc::new(AtomicBool::new(false));
        let mut guard = Self {
            received,
            registrations: Vec::new(),
        };
        for signal in [SIGINT, SIGTERM, SIGHUP] {
            guard.registrations.push(
                signal_hook::flag::register_conditional_default(
                    signal,
                    Arc::clone(&guard.received),
                )
                .with_context(|| format!("failed to register signal {signal} fallback"))?,
            );
            guard.registrations.push(
                signal_hook::flag::register(signal, Arc::clone(&guard.received))
                    .with_context(|| format!("failed to register signal {signal} handler"))?,
            );
        }
        Ok(guard)
    }

    fn received(&self) -> bool {
        self.received.load(Ordering::SeqCst)
    }
}

#[cfg(unix)]
impl Drop for InterruptionGuard {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
    }
}

fn resolve_working_directory(
    loaded: &LoadedScenario,
    configured: Option<&Path>,
) -> Option<PathBuf> {
    configured.map(|path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            loaded
                .source_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(path)
        }
    })
}

fn sample_from_status(
    iteration: u32,
    duration_ms: f64,
    max_rss_kib: Option<u64>,
    status: ExitStatus,
    timed_out: bool,
) -> MeasurementSample {
    MeasurementSample {
        iteration,
        duration_ms,
        max_rss_kib,
        exit_code: status.code(),
        timed_out,
        succeeded: status.success() && !timed_out,
    }
}

fn max_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn read_resident_memory_kib(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse().ok()
    })
}

pub(crate) fn resident_memory_sampling_supported() -> bool {
    read_resident_memory_kib(std::process::id()).is_some()
}

#[must_use]
pub fn statistics(values: &[f64]) -> Statistics {
    assert!(!values.is_empty(), "statistics require at least one sample");
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let sample_count = sorted.len();
    let sum: f64 = sorted.iter().sum();
    let median = if sample_count % 2 == 0 {
        (sorted[sample_count / 2 - 1] + sorted[sample_count / 2]) / 2.0
    } else {
        sorted[sample_count / 2]
    };
    let p95_index = ((sample_count as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sample_count - 1);

    Statistics {
        sample_count,
        minimum: sorted[0],
        maximum: sorted[sample_count - 1],
        mean: sum / sample_count as f64,
        median,
        p95: sorted[p95_index],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_statistics() {
        let result = statistics(&[4.0, 1.0, 3.0, 2.0]);
        assert_eq!(result.minimum, 1.0);
        assert_eq!(result.maximum, 4.0);
        assert_eq!(result.mean, 2.5);
        assert_eq!(result.median, 2.5);
        assert_eq!(result.p95, 4.0);
    }

    #[test]
    fn memory_metric_is_explicitly_observed_rss() {
        let samples = [MeasurementSample {
            iteration: 1,
            duration_ms: 1.0,
            max_rss_kib: Some(128),
            exit_code: Some(0),
            timed_out: false,
            succeeded: true,
        }];
        let metrics = memory_metric_summaries(&samples);

        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].id, PROCESS_MAX_RSS_V1_ID);
        assert_eq!(metrics[1].id, PROCESS_MAX_OBSERVED_RSS_ID);
        assert_eq!(metrics[0].statistics, metrics[1].statistics);
        assert_eq!(metrics[1].unit, "KiB");
        assert_eq!(metrics[1].preferred_direction, PreferredDirection::Lower);
    }
}
