use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::bundle::validate_bundle;
use crate::contract::BundleManifest;
use crate::digest::sha256_file;

pub const AGENT_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_PROFILE_BUNDLE_KIND: &str = "runtime-profile-bundle";
const MAX_EVIDENCE_URI_BYTES: usize = 2_048;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentEvidenceReference {
    pub schema_version: u32,
    pub kind: String,
    pub uri: String,
    pub digest: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_size_bytes",
        skip_serializing_if = "Option::is_none"
    )]
    pub size_bytes: Option<serde_json::Number>,
}

fn deserialize_size_bytes<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<serde_json::Number>, D::Error>
where
    D: Deserializer<'de>,
{
    let size = Option::<serde_json::Number>::deserialize(deserializer)?;
    if let Some(number) = &size
        && !is_nonnegative_integer(number)
    {
        return Err(serde::de::Error::custom(
            "sizeBytes must be a non-negative integer",
        ));
    }
    Ok(size)
}

fn is_nonnegative_integer(number: &serde_json::Number) -> bool {
    let representation = number.to_string();
    let (coefficient, exponent) = representation
        .split_once(['e', 'E'])
        .unwrap_or((representation.as_str(), "0"));
    let unsigned_coefficient = coefficient.trim_start_matches('-');
    if unsigned_coefficient
        .bytes()
        .all(|byte| byte == b'0' || byte == b'.')
    {
        return true;
    }
    if coefficient.starts_with('-') {
        return false;
    }

    let exponent_is_negative = exponent.starts_with('-');
    let exponent_digits = exponent.trim_start_matches(['+', '-']);
    let exponent_magnitude = exponent_digits.parse::<usize>();
    let fractional_digits = coefficient
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let trailing_zeroes = coefficient
        .bytes()
        .rev()
        .filter(|byte| *byte != b'.')
        .take_while(|byte| *byte == b'0')
        .count();

    match exponent_magnitude {
        Ok(magnitude) if exponent_is_negative => fractional_digits
            .checked_add(magnitude)
            .is_some_and(|required| trailing_zeroes >= required),
        Ok(magnitude) => magnitude
            .checked_add(trailing_zeroes)
            .is_none_or(|shifted| shifted >= fractional_digits),
        Err(_) => !exponent_is_negative,
    }
}

pub fn build_agent_evidence_reference(
    bundle: &Path,
    uri: impl Into<String>,
) -> Result<AgentEvidenceReference> {
    let uri = uri.into();
    validate_evidence_uri(&uri)?;

    let validation = validate_bundle(bundle)?;
    ensure!(
        validation.valid,
        "bundle validation failed: {}",
        validation.diagnostics.join("; ")
    );

    let manifest_path = bundle.join("manifest.json");
    let manifest: BundleManifest =
        serde_json::from_slice(&fs::read(&manifest_path).with_context(|| {
            format!(
                "failed to read bundle manifest: {}",
                manifest_path.display()
            )
        })?)
        .with_context(|| format!("invalid bundle manifest: {}", manifest_path.display()))?;

    Ok(AgentEvidenceReference {
        schema_version: AGENT_EVIDENCE_SCHEMA_VERSION,
        kind: RUNTIME_PROFILE_BUNDLE_KIND.to_owned(),
        uri,
        digest: format!("sha256:{}", sha256_file(&manifest_path)?),
        created_at: format_created_at(manifest.created_unix_ms)?,
        media_type: None,
        size_bytes: None,
    })
}

fn validate_evidence_uri(uri: &str) -> Result<()> {
    ensure!(!uri.is_empty(), "evidence URI must not be empty");
    ensure!(
        uri.len() <= MAX_EVIDENCE_URI_BYTES,
        "evidence URI must be at most {MAX_EVIDENCE_URI_BYTES} UTF-8 bytes"
    );
    Ok(())
}

fn format_created_at(created_unix_ms: u128) -> Result<String> {
    let milliseconds = i128::try_from(created_unix_ms)
        .context("bundle creation time does not fit the supported timestamp range")?;
    let nanoseconds = milliseconds
        .checked_mul(1_000_000)
        .context("bundle creation time overflows nanosecond timestamp range")?;
    let timestamp = OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
        .context("bundle creation time is outside the supported RFC3339 range")?;
    timestamp
        .format(&Rfc3339)
        .context("failed to format bundle creation time as RFC3339")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bundle_time_deterministically() {
        assert_eq!(
            format_created_at(0).expect("epoch timestamp"),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            format_created_at(123).expect("millisecond timestamp"),
            "1970-01-01T00:00:00.123Z"
        );
    }

    #[test]
    fn serializes_agent_evidence_contract_field_names() {
        let reference = AgentEvidenceReference {
            schema_version: 1,
            kind: RUNTIME_PROFILE_BUNDLE_KIND.to_owned(),
            uri: ".agent-loop/evidence/runtime-profiler/bundle-1".to_owned(),
            digest: format!("sha256:{}", "a".repeat(64)),
            created_at: "2026-08-15T04:00:00Z".to_owned(),
            media_type: None,
            size_bytes: None,
        };
        let value = serde_json::to_value(reference).expect("serialize evidence reference");

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["kind"], RUNTIME_PROFILE_BUNDLE_KIND);
        assert!(value.get("schema_version").is_none());
        assert!(value.get("created_at").is_none());
        assert!(value.get("mediaType").is_none());
        assert!(value.get("sizeBytes").is_none());
    }

    #[test]
    fn deserializes_optional_agent_evidence_fields() {
        let reference: AgentEvidenceReference = serde_json::from_str(include_str!(
            "../examples/agent-evidence-reference-with-metadata.json"
        ))
        .expect("deserialize contract-valid evidence metadata");

        assert_eq!(reference.media_type.as_deref(), Some("application/json"));
        assert_eq!(
            reference.size_bytes.as_ref().map(ToString::to_string),
            Some("18446744073709551616".to_owned())
        );
    }

    fn reference_json_with_size(size: &str) -> String {
        format!(
            r#"{{"schemaVersion":1,"kind":"runtime-profile-bundle","uri":"bundle://valid","digest":"sha256:{digest}","createdAt":"2026-08-15T04:00:00Z","sizeBytes":{size}}}"#,
            digest = "a".repeat(64)
        )
    }

    #[test]
    fn rejects_contract_invalid_evidence_sizes() {
        for size in ["-1", "1.5", "1e-1", "100.01e-2"] {
            let result =
                serde_json::from_str::<AgentEvidenceReference>(&reference_json_with_size(size));
            assert!(result.is_err(), "size {size} should be rejected");
        }
    }

    #[test]
    fn accepts_schema_integer_number_forms() {
        for size in [
            "0",
            "-0",
            "1.0",
            "10e2",
            "100e-2",
            "100.00e-2",
            "18446744073709551616",
        ] {
            let result =
                serde_json::from_str::<AgentEvidenceReference>(&reference_json_with_size(size));
            assert!(
                result.is_ok(),
                "size {size} should be accepted: {:?}",
                result.err()
            );
        }
    }

    #[test]
    fn bounds_caller_provided_evidence_uris() {
        assert!(validate_evidence_uri("bundle://valid").is_ok());
        assert!(validate_evidence_uri("").is_err());
        assert!(validate_evidence_uri(&"a".repeat(MAX_EVIDENCE_URI_BYTES + 1)).is_err());
    }
}
