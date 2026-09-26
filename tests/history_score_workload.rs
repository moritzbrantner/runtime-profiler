use std::{fs, path::Path};

use runtime_profiler::{contract::Target, load_scenario, validate_bundle};

#[test]
fn checked_in_history_fixture_is_valid_and_traces_to_source_scenario() {
    let source = load_scenario(Path::new("tests/fixtures/history-score-source.json"))
        .expect("history fixture source scenario");
    let evidence: serde_json::Value = serde_json::from_slice(
        &fs::read("tests/fixtures/history-score-bundle/scenario.json")
            .expect("history fixture scenario evidence"),
    )
    .expect("valid history fixture scenario evidence JSON");

    assert_eq!(
        evidence["digest"].as_str(),
        Some(source.digest.as_str()),
        "fixture scenario evidence must retain the source scenario digest"
    );

    let report = validate_bundle(Path::new("tests/fixtures/history-score-bundle"))
        .expect("validate history score fixture");
    assert!(
        report.valid,
        "history score fixture diagnostics: {:?}",
        report.diagnostics
    );
}

#[test]
fn history_scenario_profiles_runtime_profiler_score_path() {
    let loaded =
        load_scenario(Path::new("examples/history-score.yaml")).expect("history score scenario");
    assert_eq!(
        loaded.scenario.id,
        "runtime-profiler-history-self-score-v1"
    );

    let Target::Command { program, args, .. } = &loaded.scenario.target else {
        panic!("history score workload must remain a command target");
    };
    assert_eq!(program, "target/release/runtime-profiler");
    let expected = vec![
        "score".to_owned(),
        "--reference".to_owned(),
        "target/history-score-fixture".to_owned(),
        "--candidate".to_owned(),
        "target/history-score-fixture".to_owned(),
        "--json".to_owned(),
    ];
    assert_eq!(args, &expected);
    assert!(
        args.iter().all(|argument| !argument.contains("sleep")),
        "history workload must exercise runtime-profiler rather than scheduler sleep"
    );
}
