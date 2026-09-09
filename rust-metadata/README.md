# rust-metadata

Rust-based generator for `.windows/winmd/Microsoft.ServiceFabric.winmd`, using
the published `windows-clang`, `windows-rdl`, `windows-metadata`, and
`windows-default` crates at version `0.100.0`.

## Pipeline

1. Provision the pinned libclang release.
2. Compile Service Fabric IDL files into headers with the Windows SDK
   `midl.exe`.
3. Scrape the five namespace partitions into RDL in dependency order.
4. Apply the Service Fabric alias, type-projection, enum, and agility
   transformations in `src/main.rs` and `seed/`.
5. Compile the combined RDL directly to
   `.windows/winmd/Microsoft.ServiceFabric.winmd`.

Intermediates are written under `target/gen/`. The embedded
`Windows.Win32.winmd` there is a generation-only reference and is not
distributed with the final artifact.

## Run

Requires a stable Rust toolchain, Visual Studio C++ build tools, a Windows 10
or 11 SDK, and PowerShell 7 (`pwsh`). The repository's standard CMake workflow
also requires CMake:

```pwsh
pwsh -File rust-metadata/run.ps1
```

The script discovers Visual Studio and x64 `midl.exe`, enters the developer
environment, and runs the locked release build.

## Validate

From the repository root:

```pwsh
pwsh -File scripts/check_winmd.ps1
```

The validator reads both winmd files with `windows-metadata` and compares their
typed structures. `cargo test --manifest-path rust-metadata/Cargo.toml --locked`
runs the validator and generator tests.

See [the detailed generation guide](../docs/RustWinmdGeneration.md) for design
decisions, transformations, and troubleshooting.
