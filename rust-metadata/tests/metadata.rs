use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use windows_metadata::reader::TypeCategory;
use windows_metadata::{HasAttributes, Type, Value};

fn committed_winmd() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".windows")
        .join("winmd")
        .join("Microsoft.ServiceFabric.winmd")
}

#[test]
fn uses_microsoft_root_and_external_filetime() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();

    assert!(index.contains(
        "Microsoft.ServiceFabric.FabricTypes",
        "FABRIC_APPLICATION_PARAMETER"
    ));
    assert!(!index.contains("Microsoft.ServiceFabric.FabricTypes", "FILETIME"));
    assert!(
        index
            .types()
            .filter(|definition| definition
                .namespace()
                .starts_with("Microsoft.ServiceFabric."))
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
            .starts_with("Microsoft.ServiceFabric.")
    }));
    for namespace in [
        "FabricTypes",
        "FabricCommon",
        "FabricClient",
        "FabricRuntime",
        "FabricTransport",
        "Metadata",
    ] {
        assert!(index.contains_namespace(&format!("Microsoft.ServiceFabric.{namespace}")));
    }
}

#[test]
fn uses_canonical_string_aliases_and_preserves_record_aliases() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();

    assert!(!index.contains("Microsoft.ServiceFabric.FabricTypes", "LPCWSTR"));
    let uri = index.expect("Microsoft.ServiceFabric.FabricTypes", "FABRIC_URI");
    assert!(matches!(
        uri.underlying_type(),
        Some(Type::ValueName(name))
            if name.namespace == "Windows.Win32" && name.name == "PCWSTR"
    ));

    let pair = index.expect("Microsoft.ServiceFabric.FabricTypes", "FABRIC_STRING_PAIR");
    assert!(matches!(
        pair.underlying_type(),
        Some(Type::ValueName(name))
            if name.namespace == "Microsoft.ServiceFabric.FabricTypes"
                && name.name == "FABRIC_APPLICATION_PARAMETER"
    ));
}

#[test]
fn preserves_idl_type_spelling() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();
    assert!(index.contains(
        "Microsoft.ServiceFabric.FabricTypes",
        "FABRIC_AAD_ClAIMS_RETRIEVAL_METADATA"
    ));
    assert!(!index.contains(
        "Microsoft.ServiceFabric.FabricTypes",
        "FABRIC_AAD_CLAIMS_RETRIEVAL_METADATA"
    ));
}

#[test]
fn type_definitions_are_unique_and_complete() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();
    let definitions = index.types().collect::<Vec<_>>();
    assert_eq!(definitions.len(), 1_278);

    let mut names = BTreeSet::new();
    for definition in definitions {
        assert!(names.insert(format!("{}.{}", definition.namespace(), definition.name())));
    }
}

#[test]
fn all_ifabric_interfaces_are_agile() {
    let index = windows_metadata::reader::Index::read(committed_winmd()).unwrap();
    let interfaces = index
        .types()
        .filter(|definition| definition.category() == TypeCategory::Interface)
        .filter(|definition| {
            definition.name().len() > "IFabric".len()
                && definition.name().starts_with("IFabric")
                && definition
                    .name()
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .collect::<Vec<_>>();
    assert_eq!(interfaces.len(), 272);

    for interface in interfaces {
        let markers = interface
            .attributes()
            .filter(|attribute| attribute.name() == "MarshalingBehaviorAttribute")
            .collect::<Vec<_>>();
        assert_eq!(
            markers.len(),
            1,
            "{}.{} must have one agility marker",
            interface.namespace(),
            interface.name()
        );
        assert!(markers[0].value().iter().any(|(_, value)| match value {
            Value::I32(2) => true,
            Value::EnumValue(_, inner) => matches!(inner.as_ref(), Value::I32(2)),
            _ => false,
        }));
    }
}

#[test]
fn external_win32_references_have_assembly_scope() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("reference_scopes.ps1");
    let status = Command::new("pwsh")
        .args(["-NoProfile", "-File"])
        .arg(script)
        .arg("-WinmdPath")
        .arg(committed_winmd())
        .status()
        .expect("PowerShell 7 is required by the metadata generation toolchain");
    assert!(status.success());
}
