# Generating Service Fabric metadata with Rust

`rust-metadata` is the repository's supported generator for
`.windows/winmd/Microsoft.ServiceFabric.winmd`. It uses the published
windows-rs metadata crates and does not require a separate managed SDK or
package restore.

## Pipeline

1. `run.ps1` discovers Visual Studio and the Windows SDK, then enters an x64
   developer environment.
2. `windows-clang` provisions its pinned libclang release.
3. The Windows SDK `midl.exe` compiles the Service Fabric IDL files into
   headers.
4. `windows-clang` scrapes five namespace partitions into RDL in dependency
   order.
5. `windows-rdl` compiles the partitions and seed definitions into the single,
   self-contained `Microsoft.ServiceFabric.winmd`.

Intermediate headers, RDL, partition metadata, and the embedded flat Win32
reference remain under `rust-metadata/target/gen`. Only the final Service
Fabric metadata file is committed.

## Required transformations

The generator applies a small set of deterministic transformations after
scraping:

- Supplies the `FABRIC_STRING_PAIR` alias omitted by the header scraper.
- Defines `FILETIME` in the Service Fabric namespace so the final metadata
  remains single-rooted under `Microsoft`.
- Preserves `PCWSTR` projection for wide-string aliases and keeps `FABRIC_URI`
  as an ABI-compatible newtype.
- Normalizes the `FABRIC_AAD_CLAIMS` spelling.
- Marks C-style enums as scoped so binding generation preserves their newtype
  representation.
- Marks interfaces whose names match `IFabric\w+` with
  `MarshalingBehaviorAttribute(Agile)` so binding generation retains the
  expected thread-agility behavior. Non-matching interfaces are not marked.

The seed definitions are in `rust-metadata/seed/`.

## Generate

Prerequisites are a stable Rust toolchain, Visual Studio C++ build tools, a
Windows 10 or 11 SDK containing x64 `midl.exe`, PowerShell 7 (`pwsh`), and
CMake. The first run may download the pinned libclang component.

Use the standard CMake target:

```pwsh
cmake . -B build -T host=x64 -A x64
cmake --build build --target generate_winmd
```

Or run the generator directly:

```pwsh
pwsh -File rust-metadata/run.ps1
```

Both commands write `.windows/winmd/Microsoft.ServiceFabric.winmd`.

## Validate

```pwsh
cmake --build build --target validate_winmd
```

Validation uses `windows-metadata` to compare the regenerated artifact with the
committed Rust-generated baseline. It compares type sets, type flags,
inheritance, implemented interfaces, class layout, fields, constants, method
signatures and parameter metadata, GUIDs, and custom attributes. This avoids a
dependency on `ildasm` and ignores container details that are not part of the
typed metadata model.

The migration was also checked once against the retired baseline: the previous
artifact contained 1,263 types and the Rust artifact contains 1,280 types. All
272 real `IFabric*` interfaces retained their names, GUIDs, and ordered method
names. Three duplicate mangled artifacts that did not represent distinct APIs
were intentionally omitted:

- `IFabricClientConnectionEventHandler0000`
- `IFabricClientConnectionEventHandler0001`
- `IFabricServiceNotificationEventHandler0000`

## Troubleshooting

- **Visual Studio discovery fails**: install Visual Studio C++ build tools, or
  set `SF_METADATA_VSWHERE` / `SF_METADATA_VS_INSTALL_PATH`.
- **`midl.exe` is missing**: install a Windows 10 or 11 SDK, or set
  `SF_METADATA_WINDOWS_KITS_BIN`.
- **libclang provisioning fails**: verify network access on the first run; the
  provisioned component is cached for later runs.
- **Validation reports differences**: regenerate intentionally, inspect the
  typed difference report, and commit the updated winmd only when the metadata
  change is expected.
