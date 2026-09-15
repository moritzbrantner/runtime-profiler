use std::fs;

use serde_json::Value;

#[test]
fn browser_runtime_schema_accepts_comparison_identity_fields() {
    let schema: Value = serde_json::from_str(
        &fs::read_to_string("schemas/browser-runtime.schema.json").expect("browser runtime schema"),
    )
    .expect("valid browser runtime schema JSON");
    let properties = schema["properties"]
        .as_object()
        .expect("browser runtime schema properties");

    for field in ["adapter_digest", "normalizer_digest", "journey_digest"] {
        let property = properties
            .get(field)
            .unwrap_or_else(|| panic!("browser runtime schema must describe {field}"));
        assert_eq!(
            property["pattern"],
            Value::String("^sha256:[a-f0-9]{64}$".to_owned()),
            "{field} must be a prefixed lowercase SHA-256 digest"
        );
    }
}
