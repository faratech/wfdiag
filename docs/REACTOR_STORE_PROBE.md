# Reactor Store/MSIX probe

`scripts/build-reactor-msix-probe.py` is an isolated, non-publishing package
probe for the Windows Reactor cutover. It does not replace or mutate the Tauri
Store build. It never signs, installs, registers, uploads, or publishes a
package.

The probe builds the default, framework-dependent Reactor target for x64 and
ARM64. Each clean package layout contains:

- `wfdiag-reactor-spike.exe`
- Reactor's architecture-matched
  `Microsoft.WindowsAppRuntime.Bootstrap.dll`
- the canonical Store manifest and its four referenced image assets

No other DLL is allowed. In particular, the probe does not copy anything from
`src-tauri/resources/ai-sdk`; it rejects app-local
`Microsoft.WindowsAppRuntime.dll`, WinUI/XAML runtime DLLs, and Windows AI DLLs.
The bootstrap SHA-256 and PE machine are pinned independently for x64 and ARM64,
which prevents an older 1.8 bootstrap or a cross-architecture bootstrap from
entering the package.

The manifest is derived from `AppxManifest.xml`, preserving the production
Store identity/publisher/version, both `TargetDeviceFamily` declarations, all
visual assets, `Windows.FullTrustApplication`, and all network,
`runFullTrust`, and `systemAIModels` capabilities. The probe-only transform sets
the architecture, points at the Reactor executable, and replaces the Store
runtime dependency with exactly:

```xml
<PackageDependency Name="Microsoft.WindowsAppRuntime.2"
                   MinVersion="2.4.0.0"
                   Publisher="CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US" />
```

This matches the Windows App Runtime 2.4 bootstrap and metadata hard-coded by
the pinned Reactor revision. The framework package must be present on any
machine used for a future packaged runtime test; it is intentionally not
carried app-local in this probe.

## Build

From WSL with the Windows SDK installed:

```bash
python3 scripts/build-reactor-msix-probe.py
```

The default output is
`/mnt/c/code/wfdiag-reactor-store-probe`. Override it with `--output`, but a WSL
path must be under `/mnt/<drive>` so Windows `MakeAppx.exe` can access it. On
native Windows, the default is `artifacts/reactor-store-probe`.

The command emits two unsigned `.msix` files, one unsigned `.msixbundle`, a
machine-readable `probe-report.json`, and `NON-PUBLISHING-PROBE.txt`. The report
records artifact hashes and explicitly records that signing, installation, and
publication did not occur.

These artifacts preserve the real Store identity and are inspection evidence,
not release candidates. Do not install or submit them. Packaged runtime, Phi or
Aion, Store ingestion, signing, and clean-machine startup remain separate
validation gates.

## Verify the probe logic

```bash
python3 -m unittest scripts/test_build_reactor_msix_probe.py -v
```

The tests pin manifest preservation/runtime alignment, exact payload inventory,
PE architecture and bootstrap identity, archive inspection, and rejection of a
dual runtime or app-local AI DLL.
