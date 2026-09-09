use std::path::Path;
use std::process::Command;

use sf_winmd_gen::validation;

fn committed_winmd() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".windows")
        .join("winmd")
        .join("Microsoft.ServiceFabric.winmd")
}

#[test]
fn parses_and_compares_real_winmd() {
    let path = committed_winmd();
    let snapshot = validation::load(&path).expect("committed winmd should be readable");
    assert!(validation::type_count(&snapshot) > 1_000);
    assert!(validation::compare(&snapshot, &snapshot).is_empty());
}

#[test]
fn command_reports_success_for_identical_winmds() {
    let path = committed_winmd();
    let output = Command::new(env!("CARGO_BIN_EXE_validate_winmd"))
        .arg(&path)
        .arg(&path)
        .output()
        .expect("validator should run");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("winmd validation passed"));
}

#[test]
fn command_rejects_an_unreadable_baseline() {
    let output = Command::new(env!("CARGO_BIN_EXE_validate_winmd"))
        .arg("missing-baseline.winmd")
        .arg(committed_winmd())
        .output()
        .expect("validator should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read metadata"));
}
