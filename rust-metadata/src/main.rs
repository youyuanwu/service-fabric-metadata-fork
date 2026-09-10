use std::path::{Path, PathBuf};
use std::process::Command;

use windows_clang::*;

/// A metadata partition: one winmd namespace produced from one MIDL-generated
/// header. The namespace and traversal header are explicit here so the
/// generation pipeline has one source of partition configuration.
struct Partition {
    /// Winmd namespace, e.g. `Windows.ServiceFabric.FabricClient`.
    namespace: &'static str,
    /// The MIDL-generated header (stem, no extension) that defines this
    /// partition's types, e.g. `FabricClient`.
    header: &'static str,
}

const PARTITIONS: &[Partition] = &[
    Partition {
        namespace: "Windows.ServiceFabric.FabricTypes",
        header: "FabricTypes",
    },
    Partition {
        namespace: "Windows.ServiceFabric.FabricCommon",
        header: "FabricCommon",
    },
    Partition {
        namespace: "Windows.ServiceFabric.FabricClient",
        header: "FabricClient",
    },
    Partition {
        namespace: "Windows.ServiceFabric.FabricRuntime",
        header: "FabricRuntime",
    },
    Partition {
        namespace: "Windows.ServiceFabric.FabricTransport",
        header: "fabrictransport_",
    },
];

/// The `.idl` files to compile, resolved relative to the repository root.
/// Order matters for MIDL only in that imports must be resolvable via `/I`;
/// every file is compiled independently.
const IDLS: &[(&str, &str)] = &[
    ("idl", "FabricTypes.idl"),
    ("idl", "FabricCommon.idl"),
    ("idl", "FabricClient.idl"),
    ("idl", "FabricRuntime.idl"),
    ("internal_idl", "fabrictransport_.idl"),
];

fn main() {
    // Repository root is the parent of this crate directory.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has a parent directory")
        .to_path_buf();

    let out = repo.join("rust-metadata").join("target").join("gen");
    let headers = out.join("headers");
    let rdl_dir = out.join("rdl");
    let winmd_out = repo
        .join(".windows")
        .join("winmd")
        .join("Windows.ServiceFabric.winmd");
    std::fs::create_dir_all(&headers).unwrap();
    std::fs::create_dir_all(&rdl_dir).unwrap();
    std::fs::create_dir_all(winmd_out.parent().unwrap()).unwrap();

    // The VS Developer environment must be active so `INCLUDE` points at the
    // Windows SDK + MSVC headers/idls that MIDL and clang both consume.
    let include = std::env::var("INCLUDE")
        .expect("INCLUDE not set - run inside a VS Developer PowerShell (see run.ps1)");
    let include_dirs: Vec<String> = include
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    // 1. Provision the pinned libclang (cached after first download).
    println!("provisioning pinned libclang (network access may be required)...");
    ensure_libclang();
    assert_libclang_version();
    println!("libclang: {}", clang_version().expect("libclang version"));

    // The Win32 metadata the SF types reference (FILETIME, GUID, HRESULT, ...).
    //
    // This MUST be the flat Windows.Win32.winmd that windows-rs ships: the RDL
    // reader hardcodes the pseudo-attribute namespace to `Windows.Win32.Metadata`
    // (e.g. NativeEncodingAttribute), which only exists in that flat layout. The
    // The former package-sourced Win32 metadata puts those types in
    // `Windows.Win32.Foundation.Metadata`, so it cannot be used as the reference
    // here. Consequence: the generated SF winmd is tied to the new flat Win32
    // layout (and the matching new windows-bindgen consumer).
    let win32_winmd = write_default_win32(&out);
    println!("win32 winmd: {}", win32_winmd.display());

    // 2. IDL -> C/C++ headers via MIDL.
    let midl = find_midl();
    println!("midl: {}", midl.display());
    for (dir, idl) in IDLS {
        run_midl(&midl, &repo, dir, idl, &headers);
    }

    // 3. Each partition header -> per-namespace RDL via windows-clang.
    //
    // The partitions are processed in dependency order (Types, Common first).
    // Cross-namespace type references (e.g. FabricClient using a FabricTypes
    // struct) can only be emitted *namespace-qualified* if clang is given a
    // reference winmd that already owns those types. So each partition is
    // compiled to an intermediate winmd as we go, and every subsequent
    // partition is scraped and compiled with all previously-built winmds as
    // references. Without this the RDL reader fails with "type not found" on
    // the bare cross-namespace name.
    let mut include_args: Vec<String> = Vec::new();
    for dir in &include_dirs {
        include_args.push("-isystem".to_string());
        include_args.push(dir.clone());
    }
    let headers_arg = format!("-I{}", headers.display());
    let winmd_dir = out.join("winmd");
    std::fs::create_dir_all(&winmd_dir).unwrap();

    let mut rdl_paths: Vec<String> = Vec::new();
    let mut built_winmds: Vec<String> = Vec::new();

    // Seed RDL supplying the MIDL struct alias windows-clang drops (see the file
    // for details). It belongs to the FabricTypes namespace.
    let seed = repo
        .join("rust-metadata")
        .join("seed")
        .join("FabricTypes.rdl")
        .to_string_lossy()
        .replace('\\', "/");

    // Self-contained seed defining `MarshalingBehaviorAttribute` +
    // `MarshalingType` (under Windows.ServiceFabric.Metadata). The post-scrape
    // rewrite (below) stamps every interface with this attribute so windows-bindgen
    // projects them as thread-agile (`Send` + `Sync`). Compiled into the
    // FabricTypes partition winmd so all later partitions resolve the attribute.
    let agile_seed = repo
        .join("rust-metadata")
        .join("seed")
        .join("FabricAgile.rdl")
        .to_string_lossy()
        .replace('\\', "/");

    for p in PARTITIONS {
        let rdl_path = rdl_dir.join(format!("{}.rdl", p.header));
        let source = format!("#include <{}.h>", p.header);
        let mut clang = clang();
        clang
            .target("x86_64-pc-windows-msvc")
            .args(["-x", "c++"])
            .arg(&headers_arg)
            .args(&include_args)
            .namespace(p.namespace)
            .filter(&format!("{}.h", p.header))
            .input_text(&source)
            .reference(&win32_winmd)
            .output(&rdl_path);
        // Already-built partitions act as the cross-namespace reference so
        // clang emits qualified names for their types.
        for winmd in &built_winmds {
            clang.reference(winmd);
        }
        println!("scraping {} -> {}", p.header, rdl_path.display());
        clang
            .write()
            .unwrap_or_else(|e| panic!("clang scrape of {} failed: {e}", p.header));

        {
            let text = std::fs::read_to_string(&rdl_path)
                .unwrap_or_else(|e| panic!("read {} failed: {e}", rdl_path.display()));

            // windows-clang scrapes `const wchar_t*` typedefs (LPCWSTR, and
            // FABRIC_URI which aliases it) as raw `*const u16`. The former
            // generator mapped these to PCWSTR, which the mssf
            // string helpers (WString <-> PCWSTR) depend on. Re-point the LPCWSTR
            // alias at the Win32 PCWSTR builtin so the whole chain (LPCWSTR,
            // FABRIC_URI, and every struct field that uses them) projects as
            // `windows_core::PCWSTR` again.
            let rewritten = text.replace(
                "type LPCWSTR = *const u16;",
                "type LPCWSTR = Windows::Win32::PCWSTR;",
            );

            // Restore FABRIC_URI as a distinct newtype. The former generator
            // emitted `pub struct FABRIC_URI(pub *mut u16)`, but the new
            // windows-bindgen bare-aliases any typedef whose underlying is a
            // pointer to a *non-void* type (`aliases_pointer`), so
            // `type FABRIC_URI = LPCWSTR` collapses to a transparent alias and is
            // no longer constructible as `FABRIC_URI(..)`. A pointer to *void*
            // keeps the newtype (like `HANDLE`/`HWND`), so re-point FABRIC_URI at
            // `*mut void`: bindgen then emits `pub struct FABRIC_URI(pub *mut
            // c_void)`. It is only ever passed by value into the vtable (no
            // `Param<FABRIC_URI>` bound), so the void pointer is ABI-identical.
            let rewritten =
                rewritten.replace("type FABRIC_URI = LPCWSTR;", "type FABRIC_URI = *mut void;");

            // windows-clang scrapes the SF header's `FABRIC_AAD_ClAIMS_RETRIEVAL_METADATA`
            // types with a lowercase `l` (a typo carried from the MIDL output). The
            // previous committed baseline exposes them as `...CLAIMS...`; normalize to
            // that so consumers use the conventional spelling.
            let rewritten = rewritten.replace("FABRIC_AAD_ClAIMS", "FABRIC_AAD_CLAIMS");

            // The new windows-bindgen projects unscoped (C-style) enums as a bare
            // `pub type X = i32` alias with plain integer constants, whereas the
            // old toolchain emitted a `pub struct X(pub i32)` newtype. mssf-core
            // relies on the newtype (constructs `FABRIC_X(v)` and reads `.0`). A
            // `ScopedEnumAttribute` (RDL `#[scoped]`) makes bindgen keep the
            // newtype projection. Every `#[repr(i32)]` in the scraped RDL precedes
            // an enum, so tag them all as scoped.
            let rewritten = rewritten.replace("#[repr(i32)]", "#[repr(i32)] #[scoped]");

            // Preserve the retired generator's ^IFabric\w+$ agility scope.
            let rewritten = add_agility_attributes(&rewritten);

            std::fs::write(&rdl_path, rewritten)
                .unwrap_or_else(|e| panic!("write {} failed: {e}", rdl_path.display()));
        }

        // Compile this partition (plus its dependency winmds) into an
        // intermediate winmd that later partitions reference.
        let part_winmd = winmd_dir.join(format!("{}.winmd", p.header));
        let mut reader = windows_rdl::reader();
        reader.input(&rdl_path);
        reader.reference(&win32_winmd);
        // The alias seed lives in the FabricTypes namespace; supply it when
        // compiling that partition so its winmd (and every downstream
        // reference) carries FABRIC_STRING_PAIR.
        if p.header == "FabricTypes" {
            reader.input(&seed);
            // Agile marker types (MarshalingBehaviorAttribute + MarshalingType).
            // Compiling them into FabricTypes.winmd lets every later partition
            // resolve the `#[MarshalingBehavior(Agile)]` stamped on its interfaces.
            reader.input(&agile_seed);
        }
        for winmd in &built_winmds {
            reader.reference(winmd);
        }
        reader
            .output(&part_winmd)
            .write()
            .unwrap_or_else(|e| panic!("winmd compile of {} failed: {e}", p.header));

        rdl_paths.push(rdl_path.to_string_lossy().replace('\\', "/"));
        if p.header == "FabricTypes" {
            rdl_paths.push(seed.clone());
            rdl_paths.push(agile_seed.clone());
        }
        built_winmds.push(part_winmd.to_string_lossy().replace('\\', "/"));
    }

    // 4. Compile all RDL partitions together into the single combined winmd.
    // Every RDL now carries qualified cross-namespace names, so all five
    // namespaces resolve against each other with no external reference.
    println!(
        "compiling {} rdl partitions -> {}",
        rdl_paths.len(),
        winmd_out.display()
    );
    let mut reader = windows_rdl::reader();
    reader.inputs(&rdl_paths);
    reader.reference(&win32_winmd);
    reader
        .output(&winmd_out)
        .write()
        .unwrap_or_else(|e| panic!("winmd compile failed: {e}"));

    println!("wrote {}", winmd_out.display());
}

fn add_agility_attributes(input: &str) -> String {
    const ATTRIBUTE: &str = "#[Windows::ServiceFabric::Metadata::MarshalingBehavior(Agile)]";

    let mut output = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let trimmed = content.trim_start();
        if let Some(declaration) = trimmed.strip_prefix("interface ") {
            let name = declaration
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if name.len() > "IFabric".len()
                && name.starts_with("IFabric")
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                let indent = &content[..content.len() - trimmed.len()];
                output.push_str(indent);
                output.push_str(ATTRIBUTE);
                output.push_str(if line.ends_with("\r\n") { "\r\n" } else { "\n" });
            }
        }
        output.push_str(line);
    }
    output
}

/// Materializes the flat Win32 metadata embedded by the published
/// `windows-default` crate so windows-clang and windows-rdl can consume it as a
/// normal file reference.
fn write_default_win32(out: &Path) -> PathBuf {
    let path = out.join("Windows.Win32.winmd");
    std::fs::write(&path, windows_default::WIN32)
        .unwrap_or_else(|e| panic!("write {} failed: {e}", path.display()));
    path
}

/// Locates the newest `x64\midl.exe` under the Windows Kits 10 bin directory.
fn find_midl() -> PathBuf {
    if let Some(path) = std::env::var_os("SF_METADATA_MIDL") {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "SF_METADATA_MIDL does not point to a file: {}",
            path.display()
        );
        return path;
    }

    let base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(base)
        .expect("Windows Kits 10 bin directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("x64").join("midl.exe"))
        .filter(|p| p.is_file())
        .collect();
    candidates.sort();
    candidates
        .pop()
        .expect("no x64\\midl.exe found under Windows Kits 10 bin")
}

/// Runs MIDL to turn `<dir>/<idl>` into a header in `headers`, resolving imports
/// from both SF idl directories and the SDK (`INCLUDE`).
fn run_midl(midl: &Path, repo: &Path, dir: &str, idl: &str, headers: &Path) {
    let idl_path = repo.join(dir).join(idl);
    let status = Command::new(midl)
        .current_dir(repo)
        .args(["/nologo", "/char", "signed", "/env", "x64", "/notlb"])
        .arg("/I")
        .arg(repo.join("idl"))
        .arg("/I")
        .arg(repo.join("internal_idl"))
        .arg("/out")
        .arg(headers)
        .arg(&idl_path)
        .status()
        .unwrap_or_else(|e| panic!("failed to launch midl for {idl}: {e}"));
    assert!(status.success(), "midl failed for {idl}");
}

#[cfg(test)]
mod tests {
    use super::add_agility_attributes;

    #[test]
    fn marks_only_ifabric_interfaces_as_agile() {
        let input = concat!(
            "            interface IFabricClient : IUnknown {\n",
            "            interface IExtentLogicalLog : IUnknown {\n",
            "            interface IFabric : IUnknown {\n",
            "            interface IFabric_Test : IUnknown {\n",
        );

        let output = add_agility_attributes(input);

        assert_eq!(output.matches("MarshalingBehavior(Agile)").count(), 2);
        assert!(output.contains("MarshalingBehavior(Agile)]\n            interface IFabricClient"));
        assert!(output.contains("MarshalingBehavior(Agile)]\n            interface IFabric_Test"));
        assert!(
            !output.contains("MarshalingBehavior(Agile)]\n            interface IExtentLogicalLog")
        );
        assert!(!output.contains("MarshalingBehavior(Agile)]\n            interface IFabric :"));
    }
}
