use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::contract::{
    CapturePlan, Collector, CollectorPlan, SCENARIO_EVIDENCE_SCHEMA_V1, SCENARIO_SCHEMA_V1,
    Scenario, ScenarioEvidence, Target, TargetEvidence,
};
use crate::digest::sha256_bytes;

#[derive(Debug, Clone)]
pub struct LoadedScenario {
    pub scenario: Scenario,
    pub source_path: PathBuf,
    pub digest: String,
}

impl LoadedScenario {
    #[must_use]
    pub fn plan(&self) -> CapturePlan {
        CapturePlan {
            schema_version: "runtime-profiler/plan/v1".to_owned(),
            scenario_id: self.scenario.id.clone(),
            scenario_digest: self.digest.clone(),
            target_type: "command".to_owned(),
            collectors: self
                .scenario
                .collectors
                .iter()
                .map(|collector| match collector {
                    Collector::Process => CollectorPlan {
                        id: "process".to_owned(),
                        supported: true,
                        measurements: vec![
                            "process.wall_time".to_owned(),
                            "process.success_rate".to_owned(),
                            "process.max_rss".to_owned(),
                        ],
                    },
                })
                .collect(),
            warmup_iterations: self.scenario.run.warmup_iterations,
            measurement_iterations: self.scenario.run.measurement_iterations,
            timeout_seconds: self.scenario.run.timeout_seconds,
        }
    }

    #[must_use]
    pub fn evidence(&self) -> ScenarioEvidence {
        let Target::Command {
            program,
            args,
            working_directory,
            inherit_env,
        } = &self.scenario.target;

        ScenarioEvidence {
            schema_version: SCENARIO_EVIDENCE_SCHEMA_V1.to_owned(),
            id: self.scenario.id.clone(),
            digest: self.digest.clone(),
            target: TargetEvidence {
                target_type: "command".to_owned(),
                program: program.clone(),
                argument_count: args.len(),
                working_directory_set: working_directory.is_some(),
                inherited_environment_names: inherit_env.clone(),
            },
            run: self.scenario.run.clone(),
            collectors: self.scenario.collectors.clone(),
        }
    }
}

pub fn load_scenario(path: &Path) -> Result<LoadedScenario> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read scenario: {}", path.display()))?;
    let scenario: Scenario = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON scenario: {}", path.display()))?,
        Some("yaml" | "yml") => serde_yaml::from_str(&contents)
            .with_context(|| format!("invalid YAML scenario: {}", path.display()))?,
        _ => match serde_json::from_str(&contents) {
            Ok(value) => value,
            Err(json_error) => serde_yaml::from_str(&contents).with_context(|| {
                format!(
                    "scenario is neither valid JSON ({json_error}) nor valid YAML: {}",
                    path.display()
                )
            })?,
        },
    };

    validate_scenario(&scenario)?;
    let normalized = serde_json::to_vec(&scenario).context("failed to normalize scenario")?;

    Ok(LoadedScenario {
        scenario,
        source_path: path.to_path_buf(),
        digest: sha256_bytes(&normalized),
    })
}

pub fn validate_scenario(scenario: &Scenario) -> Result<()> {
    ensure!(
        scenario.schema_version == SCENARIO_SCHEMA_V1,
        "unsupported scenario schema: {}",
        scenario.schema_version
    );
    ensure!(!scenario.id.trim().is_empty(), "scenario id must not be empty");
    ensure!(
        scenario.id.len() <= 128,
        "scenario id must be at most 128 characters"
    );
    ensure!(
        scenario
            .id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')),
        "scenario id may contain only ASCII letters, digits, dash, underscore, and dot"
    );
    ensure!(
        scenario.run.warmup_iterations <= 100,
        "warmup_iterations must be at most 100"
    );
    ensure!(
        (1..=10_000).contains(&scenario.run.measurement_iterations),
        "measurement_iterations must be between 1 and 10000"
    );
    ensure!(
        (1..=86_400).contains(&scenario.run.timeout_seconds),
        "timeout_seconds must be between 1 and 86400"
    );
    ensure!(!scenario.collectors.is_empty(), "at least one collector is required");

    let Target::Command {
        program,
        args,
        inherit_env,
        ..
    } = &scenario.target;
    ensure!(!program.trim().is_empty(), "target program must not be empty");
    ensure!(!program.contains('\0'), "target program contains a null byte");
    if args.iter().any(|argument| argument.contains('\0')) {
        bail!("target argument contains a null byte");
    }
    for name in inherit_env {
        ensure!(!name.is_empty(), "inherited environment name must not be empty");
        ensure!(
            !name.contains('=') && !name.contains('\0'),
            "invalid inherited environment name: {name}"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{RunConfig, Target};

    fn valid_scenario() -> Scenario {
        Scenario {
            schema_version: SCENARIO_SCHEMA_V1.to_owned(),
            id: "unit-test".to_owned(),
            target: Target::Command {
                program: "true".to_owned(),
                args: Vec::new(),
                working_directory: None,
                inherit_env: Vec::new(),
            },
            run: RunConfig::default(),
            collectors: vec![Collector::Process],
        }
    }

    #[test]
    fn accepts_valid_scenario() {
        assert!(validate_scenario(&valid_scenario()).is_ok());
    }

    #[test]
    fn rejects_unsafe_id() {
        let mut scenario = valid_scenario();
        scenario.id = "../../escape".to_owned();
        assert!(validate_scenario(&scenario).is_err());
    }

    #[test]
    fn redacts_arguments_from_evidence() {
        let mut scenario = valid_scenario();
        let Target::Command { args, .. } = &mut scenario.target;
        args.push("secret-value".to_owned());
        let normalized = serde_json::to_vec(&scenario).expect("serialize scenario");
        let loaded = LoadedScenario {
            scenario,
            source_path: PathBuf::from("scenario.json"),
            digest: sha256_bytes(&normalized),
        };

        let evidence = serde_json::to_string(&loaded.evidence()).expect("serialize evidence");
        assert!(!evidence.contains("secret-value"));
        assert!(evidence.contains("argument_count"));
    }
}
