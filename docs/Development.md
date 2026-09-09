# Development

## Prerequisites

- Rust stable toolchain
- Visual Studio with the C++ build tools workload
- A Windows 10 or 11 SDK containing the x64 `midl.exe`
- PowerShell 7 (`pwsh`) and CMake

The generator provisions its pinned libclang release on first use, so the first
run requires network access.

## Generate and validate metadata

Run the repository's CMake targets:

```pwsh
cmake . -B build -T host=x64 -A x64
cmake --build build --target generate_winmd
cmake --build build --target validate_winmd
```

The generation target enters the Visual Studio developer environment and writes
`.windows/winmd/Microsoft.ServiceFabric.winmd`. The validation target compares
the regenerated metadata semantically with the committed baseline.

For direct generator use:

```pwsh
pwsh -File rust-metadata/run.ps1
```