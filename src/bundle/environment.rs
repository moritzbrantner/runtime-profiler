use std::env;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::contract::{
    ENVIRONMENT_FINGERPRINT_SCHEMA_V1, ENVIRONMENT_SCHEMA_V1, EnvironmentDocument, SourceIdentity,
};
use crate::digest::sha256_bytes;

#[derive(Serialize)]
struct EnvironmentFingerprintInput<'a> {
    schema_version: &'static str,
    operating_system: &'a str,
    architecture: &'a str,
    kernel_release: &'a Option<String>,
    logical_cpu_count: usize,
}

pub(super) fn detect_environment() -> Result<EnvironmentDocument> {
    let source = SourceIdentity {
        git_sha: command_output("git", &["rev-parse", "HEAD"]),
        dirty: command_output("git", &["status", "--porcelain"]).map(|output| !output.is_empty()),
    };
    let mut environment = EnvironmentDocument {
        schema_version: ENVIRONMENT_SCHEMA_V1.to_owned(),
        environment_fingerprint_schema_version: ENVIRONMENT_FINGERPRINT_SCHEMA_V1.to_owned(),
        fingerprint: String::new(),
        operating_system: env::consts::OS.to_owned(),
        architecture: env::consts::ARCH.to_owned(),
        kernel_release: command_output("uname", &["-r"]),
        logical_cpu_count: thread_count(),
        source,
    };
    environment.fingerprint = environment_fingerprint(&environment)?;
    Ok(environment)
}

fn environment_fingerprint(environment: &EnvironmentDocument) -> Result<String> {
    let input = EnvironmentFingerprintInput {
        schema_version: ENVIRONMENT_FINGERPRINT_SCHEMA_V1,
        operating_system: &environment.operating_system,
        architecture: &environment.architecture,
        kernel_release: &environment.kernel_release,
        logical_cpu_count: environment.logical_cpu_count,
    };
    let normalized = serde_json::to_vec(&input).context("failed to fingerprint environment")?;
    Ok(sha256_bytes(&normalized))
}

fn thread_count() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_environment(source: SourceIdentity) -> EnvironmentDocument {
        EnvironmentDocument {
            schema_version: ENVIRONMENT_SCHEMA_V1.to_owned(),
            environment_fingerprint_schema_version: ENVIRONMENT_FINGERPRINT_SCHEMA_V1.to_owned(),
            fingerprint: String::new(),
            operating_system: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            kernel_release: Some("6.12.0".to_owned()),
            logical_cpu_count: 8,
            source,
        }
    }

    #[test]
    fn environment_fingerprint_ignores_source_identity() {
        let baseline = test_environment(SourceIdentity {
            git_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
            dirty: Some(false),
        });
        let candidate = test_environment(SourceIdentity {
            git_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned()),
            dirty: Some(true),
        });

        assert_eq!(
            environment_fingerprint(&baseline).expect("baseline fingerprint"),
            environment_fingerprint(&candidate).expect("candidate fingerprint")
        );
    }

    #[test]
    fn environment_fingerprint_changes_with_execution_environment() {
        let baseline = test_environment(SourceIdentity {
            git_sha: None,
            dirty: None,
        });
        let mut candidate = baseline.clone();
        candidate.logical_cpu_count = 16;

        assert_ne!(
            environment_fingerprint(&baseline).expect("baseline fingerprint"),
            environment_fingerprint(&candidate).expect("candidate fingerprint")
        );
    }
}
