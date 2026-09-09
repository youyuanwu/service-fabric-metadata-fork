use std::collections::BTreeMap;
use std::path::Path;

use windows_metadata::reader::{Attribute, Index, TypeDef};
use windows_metadata::{HasAttributes, Type, TypeName, Value};

const OMITTED_DUPLICATES: [&str; 3] = [
    "IFabricClientConnectionEventHandler0000",
    "IFabricClientConnectionEventHandler0001",
    "IFabricServiceNotificationEventHandler0000",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    types: BTreeMap<String, TypeRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TypeRecord {
    category: String,
    flags: u32,
    extends: Option<String>,
    interfaces: Vec<String>,
    layout: Option<(u16, u32)>,
    fields: Vec<FieldRecord>,
    methods: Vec<MethodRecord>,
    attributes: Vec<String>,
    agile: bool,
    guid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FieldRecord {
    name: String,
    ty: String,
    flags: u16,
    constant: Option<String>,
    attributes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MethodRecord {
    name: String,
    flags: u16,
    impl_flags: u16,
    calling_convention: String,
    signature: String,
    parameters: Vec<ParameterRecord>,
    return_parameter: Option<ParameterRecord>,
    attributes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParameterRecord {
    flags: u16,
    direction: String,
    optional: bool,
    retval: bool,
    attributes: Vec<String>,
}

pub fn load(path: &Path) -> Result<Snapshot, String> {
    std::fs::metadata(path)
        .map_err(|error| format!("failed to read metadata from {}: {error}", path.display()))?;
    let index = Index::read(path)
        .ok_or_else(|| format!("failed to read metadata from {}", path.display()))?;
    let mut types = BTreeMap::new();

    for def in index.types() {
        let key = qualified_name(def.namespace(), def.name());
        let record = type_record(def)?;
        if types.insert(key.clone(), record).is_some() {
            return Err(format!("duplicate type definition: {key}"));
        }
    }

    Ok(Snapshot { types })
}

pub fn compare(baseline: &Snapshot, candidate: &Snapshot) -> Vec<String> {
    let mut differences = Vec::new();

    for (name, expected) in &baseline.types {
        let Some(actual) = candidate.types.get(name) else {
            differences.push(format!("missing type: {name}"));
            continue;
        };

        if expected != actual {
            describe_type_difference(name, expected, actual, &mut differences);
        }
    }

    for name in candidate.types.keys() {
        if !baseline.types.contains_key(name) {
            differences.push(format!("unexpected type: {name}"));
        }
    }

    for (name, record) in &candidate.types {
        let expected_agile = record.category == "Interface"
            && is_ifabric(short_name(name))
            && !OMITTED_DUPLICATES.contains(&short_name(name));
        if record.agile != expected_agile {
            differences.push(format!(
                "agility mismatch for {name}: expected {expected_agile}, found {}",
                record.agile
            ));
        }
    }

    differences
}

pub fn compare_migration(baseline: &Snapshot, candidate: &Snapshot) -> Vec<String> {
    let mut differences = Vec::new();
    let baseline_interfaces = baseline
        .types
        .iter()
        .filter(|(name, record)| record.category == "Interface" && is_ifabric(short_name(name)))
        .filter(|(name, _)| !OMITTED_DUPLICATES.contains(&short_name(name)))
        .collect::<BTreeMap<_, _>>();
    let candidate_interfaces = candidate
        .types
        .iter()
        .filter(|(name, record)| record.category == "Interface" && is_ifabric(short_name(name)))
        .collect::<BTreeMap<_, _>>();

    for (name, expected) in &baseline_interfaces {
        let Some(actual) = candidate_interfaces.get(name) else {
            differences.push(format!("missing interface: {name}"));
            continue;
        };
        if expected.guid != actual.guid {
            differences.push(format!(
                "{name}: GUID changed from {:?} to {:?}",
                expected.guid, actual.guid
            ));
        }

        let expected_methods = expected
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>();
        let actual_methods = actual
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>();
        if expected_methods != actual_methods {
            differences.push(format!("{name}: method names or order changed"));
        }
    }

    for name in candidate_interfaces.keys() {
        if !baseline_interfaces.contains_key(name) {
            differences.push(format!("unexpected interface: {name}"));
        }
    }

    for (name, record) in candidate_interfaces {
        if !record.agile {
            differences.push(format!("missing agility marker: {name}"));
        }
    }

    differences
}

pub fn type_count(snapshot: &Snapshot) -> usize {
    snapshot.types.len()
}

fn type_record(def: TypeDef<'_>) -> Result<TypeRecord, String> {
    let extends = def
        .extends()
        .map(|base| qualified_name(base.namespace(), base.name()));
    let mut interfaces = def
        .interface_impls()
        .map(|implementation| canonical_type(&implementation.interface(&[])))
        .collect::<Vec<_>>();
    interfaces.sort();

    let layout = def
        .class_layout()
        .map(|layout| (layout.packing_size(), layout.class_size()));

    let fields = def
        .fields()
        .map(|field| FieldRecord {
            name: field.name().to_string(),
            ty: canonical_type(&field.ty()),
            flags: field.flags().0,
            constant: field
                .constant()
                .map(|constant| format!("{:?}", constant.value())),
            attributes: canonical_attributes(field.attributes()),
        })
        .collect();

    let methods = def
        .methods()
        .map(|method| {
            let signature = method.signature(&[]);
            let parameter_map = method
                .params_by_sequence(signature.types.len())
                .map_err(|error| format!("{}.{}: {error}", def.name(), method.name()))?;
            let parameters = parameter_map
                .params()
                .iter()
                .map(|parameter| parameter.map(parameter_record).unwrap_or_default())
                .collect();
            let return_parameter = parameter_map.return_param().map(parameter_record);

            Ok(MethodRecord {
                name: method.name().to_string(),
                flags: method.flags().0,
                impl_flags: method.impl_flags().0,
                calling_convention: method.calling_convention().to_string(),
                signature: canonical_signature(&signature),
                parameters,
                return_parameter,
                attributes: canonical_attributes(method.attributes()),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(TypeRecord {
        category: format!("{:?}", def.category()),
        flags: def.flags().0,
        extends,
        interfaces,
        layout,
        fields,
        methods,
        attributes: canonical_attributes(def.attributes()),
        agile: has_agility(def.attributes()),
        guid: attribute_value(def.attributes(), "GuidAttribute"),
    })
}

fn parameter_record(parameter: windows_metadata::reader::MethodParam<'_>) -> ParameterRecord {
    ParameterRecord {
        flags: parameter.flags().0,
        direction: format!("{:?}", parameter.direction()),
        optional: parameter.is_optional(),
        retval: parameter.is_retval_attribute(),
        attributes: canonical_attributes(parameter.attributes()),
    }
}

impl Default for ParameterRecord {
    fn default() -> Self {
        Self {
            flags: 0,
            direction: "Unspecified".to_string(),
            optional: false,
            retval: false,
            attributes: Vec::new(),
        }
    }
}

fn canonical_signature(signature: &windows_metadata::Signature) -> String {
    format!(
        "{}({})->{}",
        signature.flags.0,
        signature
            .types
            .iter()
            .map(canonical_type)
            .collect::<Vec<_>>()
            .join(","),
        canonical_type(&signature.return_type)
    )
}

fn canonical_type(ty: &Type) -> String {
    match ty {
        Type::Void => "void".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Char => "char".to_string(),
        Type::I8 => "i8".to_string(),
        Type::U8 => "u8".to_string(),
        Type::I16 => "i16".to_string(),
        Type::U16 => "u16".to_string(),
        Type::I32 => "i32".to_string(),
        Type::U32 => "u32".to_string(),
        Type::I64 => "i64".to_string(),
        Type::U64 => "u64".to_string(),
        Type::F32 => "f32".to_string(),
        Type::F64 => "f64".to_string(),
        Type::ISize => "isize".to_string(),
        Type::USize => "usize".to_string(),
        Type::String => "string".to_string(),
        Type::Object => "object".to_string(),
        Type::ClassName(name) => format!("class:{}", canonical_type_name(name)),
        Type::ValueName(name) => format!("value:{}", canonical_type_name(name)),
        Type::Array(ty) => format!("array:{}", canonical_type(ty)),
        Type::Generic(name, index) => format!("generic:{name}:{index}"),
        Type::RefMut(ty) => format!("ref-mut:{}", canonical_type(ty)),
        Type::RefConst(ty) => format!("ref-const:{}", canonical_type(ty)),
        Type::PtrMut(ty, depth) => format!("ptr-mut:{depth}:{}", canonical_type(ty)),
        Type::PtrConst(ty, depth) => format!("ptr-const:{depth}:{}", canonical_type(ty)),
        Type::ArrayFixed(ty, count) => format!("array-fixed:{count}:{}", canonical_type(ty)),
    }
}

fn canonical_type_name(name: &TypeName) -> String {
    let qualified = qualified_name(&name.namespace, &name.name);

    if name.generics.is_empty() {
        qualified
    } else {
        format!(
            "{qualified}<{}>",
            name.generics
                .iter()
                .map(canonical_type)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn canonical_attributes<'a>(attributes: impl Iterator<Item = Attribute<'a>>) -> Vec<String> {
    let mut result = attributes
        .map(|attribute| {
            format!(
                "{}={:?}",
                qualified_name(attribute.namespace(), attribute.name()),
                attribute.value()
            )
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

fn attribute_value<'a>(
    mut attributes: impl Iterator<Item = Attribute<'a>>,
    name: &str,
) -> Option<String> {
    attributes
        .find(|attribute| attribute.name() == name)
        .map(|attribute| format!("{:?}", attribute.value()))
}

fn has_agility<'a>(attributes: impl Iterator<Item = Attribute<'a>>) -> bool {
    let markers = attributes
        .filter(|attribute| attribute.name() == "MarshalingBehaviorAttribute")
        .map(|attribute| {
            attribute.value().iter().any(|(_, value)| match value {
                Value::I32(2) => true,
                Value::EnumValue(_, inner) => matches!(inner.as_ref(), Value::I32(2)),
                _ => false,
            })
        })
        .collect::<Vec<_>>();
    markers == [true]
}

fn describe_type_difference(
    name: &str,
    expected: &TypeRecord,
    actual: &TypeRecord,
    differences: &mut Vec<String>,
) {
    if expected.category != actual.category {
        differences.push(format!(
            "{name}: category changed from {} to {}",
            expected.category, actual.category
        ));
    }
    if expected.flags != actual.flags {
        differences.push(format!(
            "{name}: type flags changed from {:#x} to {:#x}",
            expected.flags, actual.flags
        ));
    }
    if expected.extends != actual.extends {
        differences.push(format!(
            "{name}: base type changed from {:?} to {:?}",
            expected.extends, actual.extends
        ));
    }
    if expected.interfaces != actual.interfaces {
        differences.push(format!("{name}: implemented interfaces changed"));
    }
    if expected.layout != actual.layout {
        differences.push(format!(
            "{name}: layout changed from {:?} to {:?}",
            expected.layout, actual.layout
        ));
    }
    if expected.fields != actual.fields {
        differences.push(format!("{name}: fields changed"));
    }
    if expected.methods != actual.methods {
        differences.push(format!("{name}: methods changed"));
    }
    if expected.attributes != actual.attributes {
        differences.push(format!("{name}: attributes changed"));
    }
    if expected.guid != actual.guid {
        differences.push(format!(
            "{name}: GUID changed from {:?} to {:?}",
            expected.guid, actual.guid
        ));
    }
}

fn qualified_name(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}.{name}")
    }
}

fn short_name(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified)
}

fn is_ifabric(name: &str) -> bool {
    name.len() > "IFabric".len()
        && name.starts_with("IFabric")
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> TypeRecord {
        TypeRecord {
            category: "Interface".to_string(),
            flags: 0,
            extends: None,
            interfaces: Vec::new(),
            layout: None,
            fields: Vec::new(),
            methods: Vec::new(),
            attributes: Vec::new(),
            agile: true,
            guid: Some("guid".to_string()),
        }
    }

    fn snapshot(entries: &[(&str, TypeRecord)]) -> Snapshot {
        Snapshot {
            types: entries
                .iter()
                .map(|(name, record)| ((*name).to_string(), record.clone()))
                .collect(),
        }
    }

    #[test]
    fn identical_snapshots_pass() {
        let input = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", record())]);
        assert!(compare(&input, &input).is_empty());
    }

    #[test]
    fn migration_allows_legacy_duplicate_to_be_absent() {
        let baseline = snapshot(&[(
            "Microsoft.ServiceFabric.IFabricClientConnectionEventHandler0000",
            record(),
        )]);
        assert!(compare_migration(&baseline, &snapshot(&[])).is_empty());
        assert!(!compare(&baseline, &snapshot(&[])).is_empty());
    }

    #[test]
    fn non_allowlisted_type_changes_fail() {
        let baseline = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", record())]);
        let mut changed = record();
        changed.methods.push(MethodRecord {
            name: "Changed".to_string(),
            flags: 0,
            impl_flags: 0,
            calling_convention: String::new(),
            signature: "0()->void".to_string(),
            parameters: Vec::new(),
            return_parameter: None,
            attributes: Vec::new(),
        });
        let candidate = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", changed)]);
        assert_eq!(
            compare(&baseline, &candidate),
            ["Microsoft.ServiceFabric.IFabricClient: methods changed"]
        );
    }

    #[test]
    fn missing_or_extra_agility_fails() {
        let mut missing = record();
        missing.agile = false;
        let baseline = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", record())]);
        let candidate = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", missing)]);
        assert!(
            compare(&baseline, &candidate)
                .iter()
                .any(|difference| difference.contains("agility mismatch"))
        );

        let extra = snapshot(&[("Microsoft.ServiceFabric.IExtentLogicalLog", record())]);
        assert!(
            compare(&extra, &extra)
                .iter()
                .any(|difference| difference.contains("agility mismatch"))
        );
    }

    #[test]
    fn migration_ignores_non_interface_ifabric_names() {
        let mut class = record();
        class.category = "Class".to_string();
        class.agile = false;
        let input = snapshot(&[("Microsoft.ServiceFabric.IFabricFactory", class)]);
        assert!(compare_migration(&input, &input).is_empty());
    }

    #[test]
    fn strict_comparison_rejects_all_added_types() {
        let baseline = snapshot(&[]);
        let mut helper = record();
        helper.agile = false;
        let allowed = snapshot(&[(
            "Microsoft.ServiceFabric.FabricTypes.FILETIME",
            helper.clone(),
        )]);
        assert_eq!(
            compare(&baseline, &allowed),
            ["unexpected type: Microsoft.ServiceFabric.FabricTypes.FILETIME"]
        );

        let unexpected = snapshot(&[("Microsoft.ServiceFabric.Other", helper)]);
        assert_eq!(
            compare(&baseline, &unexpected),
            ["unexpected type: Microsoft.ServiceFabric.Other"]
        );
    }

    #[test]
    fn migration_checks_guid_and_method_order() {
        let baseline = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", record())]);
        let mut changed = record();
        changed.guid = Some("different".to_string());
        changed.methods.push(MethodRecord {
            name: "Unexpected".to_string(),
            flags: 0,
            impl_flags: 0,
            calling_convention: String::new(),
            signature: "0()->void".to_string(),
            parameters: Vec::new(),
            return_parameter: None,
            attributes: Vec::new(),
        });
        let candidate = snapshot(&[("Microsoft.ServiceFabric.IFabricClient", changed)]);
        let differences = compare_migration(&baseline, &candidate);
        assert!(
            differences
                .iter()
                .any(|difference| difference.contains("GUID changed"))
        );
        assert!(
            differences
                .iter()
                .any(|difference| difference.contains("method names or order changed"))
        );
    }

    #[test]
    fn strict_comparison_checks_every_record_dimension() {
        let name = "Microsoft.ServiceFabric.IFabricClient";
        let baseline_record = record();
        let baseline = snapshot(&[(name, baseline_record.clone())]);

        let mut changed = baseline_record.clone();
        changed.category = "Struct".to_string();
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("category changed"));

        let mut changed = baseline_record.clone();
        changed.flags = 0x10;
        assert!(
            compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("type flags changed")
        );

        let mut changed = baseline_record.clone();
        changed.extends = Some("Microsoft.ServiceFabric.IBase".to_string());
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("base type changed"));

        let mut changed = baseline_record.clone();
        changed.interfaces.push("class:System.IUnknown".to_string());
        assert!(
            compare(&baseline, &snapshot(&[(name, changed)]))[0]
                .contains("implemented interfaces changed")
        );

        let mut changed = baseline_record.clone();
        changed.layout = Some((8, 16));
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("layout changed"));

        let mut changed = baseline_record.clone();
        changed.fields.push(FieldRecord {
            name: "Value".to_string(),
            ty: "u32".to_string(),
            flags: 0,
            constant: None,
            attributes: Vec::new(),
        });
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("fields changed"));

        let mut changed = baseline_record.clone();
        changed.methods.push(MethodRecord {
            name: "Method".to_string(),
            flags: 0,
            impl_flags: 0,
            calling_convention: String::new(),
            signature: "0()->void".to_string(),
            parameters: Vec::new(),
            return_parameter: None,
            attributes: Vec::new(),
        });
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("methods changed"));

        let mut changed = baseline_record.clone();
        changed.attributes.push("Example.Attribute=[]".to_string());
        assert!(
            compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("attributes changed")
        );

        let mut changed = baseline_record;
        changed.guid = Some("different".to_string());
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("GUID changed"));
    }

    #[test]
    fn strict_comparison_checks_field_method_and_parameter_details() {
        let name = "Microsoft.ServiceFabric.IFabricClient";
        let mut baseline_record = record();
        baseline_record.fields.push(FieldRecord {
            name: "Value".to_string(),
            ty: "u32".to_string(),
            flags: 1,
            constant: Some("I32(1)".to_string()),
            attributes: vec!["Test.Field=[]".to_string()],
        });
        baseline_record.methods.push(MethodRecord {
            name: "Call".to_string(),
            flags: 1,
            impl_flags: 2,
            calling_convention: "system".to_string(),
            signature: "32(u32)->i32".to_string(),
            parameters: vec![ParameterRecord {
                flags: 1,
                direction: "Input".to_string(),
                optional: false,
                retval: false,
                attributes: vec!["Test.Param=[]".to_string()],
            }],
            return_parameter: Some(ParameterRecord {
                flags: 2,
                direction: "Output".to_string(),
                optional: false,
                retval: true,
                attributes: Vec::new(),
            }),
            attributes: vec!["Test.Method=[]".to_string()],
        });
        let baseline = snapshot(&[(name, baseline_record.clone())]);

        let mut changed = baseline_record.clone();
        changed.fields[0].ty = "u64".to_string();
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("fields changed"));

        let mut changed = baseline_record.clone();
        changed.fields[0].constant = Some("I32(2)".to_string());
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("fields changed"));

        let mut changed = baseline_record.clone();
        changed.methods[0].signature = "32(u64)->i32".to_string();
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("methods changed"));

        let mut changed = baseline_record.clone();
        changed.methods[0].parameters[0].direction = "Output".to_string();
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("methods changed"));

        let mut changed = baseline_record;
        changed.methods[0].return_parameter.as_mut().unwrap().retval = false;
        assert!(compare(&baseline, &snapshot(&[(name, changed)]))[0].contains("methods changed"));
    }
}
