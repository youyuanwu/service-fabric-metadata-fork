use std::path::Path;
use std::process::Command;

use sf_winmd_gen::validation;
use windows_metadata::Type;

fn committed_winmd() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".windows")
        .join("winmd")
        .join("Windows.ServiceFabric.winmd")
}

#[test]
fn parses_and_compares_real_winmd() {
    let path = committed_winmd();
    let snapshot = validation::load(&path).expect("committed winmd should be readable");
    assert!(validation::type_count(&snapshot) > 1_000);
    assert!(validation::compare(&snapshot, &snapshot).is_empty());
}

#[test]
fn uses_windows_root_and_external_filetime() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();

    assert!(index.contains(
        "Windows.ServiceFabric.FabricTypes",
        "FABRIC_APPLICATION_PARAMETER"
    ));
    assert!(!index.contains("Windows.ServiceFabric.FabricTypes", "FILETIME"));
    assert!(
        index
            .types()
            .filter(|definition| definition.namespace().starts_with("Windows.ServiceFabric."))
            .flat_map(|definition| definition.fields())
            .any(|field| matches!(
                field.ty(),
                Type::ValueName(ref name)
                    if name.namespace == "Windows.Win32" && name.name == "FILETIME"
            ))
    );
    assert!(index.types().all(|definition| {
        definition
            .namespace()
            .starts_with("Windows.ServiceFabric.")
    }));
    for namespace in [
        "FabricTypes",
        "FabricCommon",
        "FabricClient",
        "FabricRuntime",
        "FabricTransport",
        "Metadata",
    ] {
        assert!(index.contains_namespace(&format!("Windows.ServiceFabric.{namespace}")));
    }
}

#[test]
fn uses_canonical_string_aliases_and_preserves_record_aliases() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();

    assert!(!index.contains("Windows.ServiceFabric.FabricTypes", "LPCWSTR"));
    let uri = index.expect("Windows.ServiceFabric.FabricTypes", "FABRIC_URI");
    assert!(matches!(
        uri.underlying_type(),
        Some(Type::ValueName(name))
            if name.namespace == "Windows.Win32" && name.name == "PCWSTR"
    ));

    let pair = index.expect("Windows.ServiceFabric.FabricTypes", "FABRIC_STRING_PAIR");
    assert!(matches!(
        pair.underlying_type(),
        Some(Type::ValueName(name))
            if name.namespace == "Windows.ServiceFabric.FabricTypes"
                && name.name == "FABRIC_APPLICATION_PARAMETER"
    ));
}

#[test]
fn preserves_idl_type_spelling() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();
    assert!(index.contains(
        "Windows.ServiceFabric.FabricTypes",
        "FABRIC_AAD_ClAIMS_RETRIEVAL_METADATA"
    ));
    assert!(!index.contains(
        "Windows.ServiceFabric.FabricTypes",
        "FABRIC_AAD_CLAIMS_RETRIEVAL_METADATA"
    ));
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
