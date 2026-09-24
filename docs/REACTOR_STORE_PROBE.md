# Reactor Store/MSIX probe and the native Store package

`scripts/build-reactor-msix-probe.py` started as an isolated, non-publishing
package probe for the Windows Reactor cutover. It never signs, installs,
registers, uploads, or publishes a package. Since the 2026-09-01 cutover
decision it is also the single source of truth for the **shipped** native
package: `.github/workflows/build-and-publish-store.yml` builds `wfdiag.exe`
itself and then calls the probe's subcommands so the Store bundle and the
probe can never disagree about the manifest or the payload:

```text
build-reactor-msix-probe.py stage --target x64 --executable <wfdiag.exe> \
    --output msix-build
build-reactor-msix-probe.py pack --target x64 --layout msix-build/layout-x64 \
    --package msix-build/bundle/WindowsForum_Diagnostics_<ver>_x64.msix
build-reactor-msix-probe.py bundle --packages-dir msix-build/bundle \
    --bundle msix-build/WindowsForum_Diagnostics_<ver>.msixbundle
build-reactor-msix-probe.py validate-msix --target x64 <package.msix>
```

Signing still happens in the separate signing workflows; `validate-msix`
deliberately rejects a signed package because it inspects unsigned CI
The probe builds the default, framework-dependent Reactor target for x64 and
ARM64. Each clean package layout contains:

- `wfdiag.exe`
- the canonical Store manifest and its four referenced image assets

No app-local DLL is allowed, including the obsolete bootstrap shim. In particular, the
probe rejects app-local `Microsoft.WindowsAppRuntime.dll`, WinUI/XAML runtime DLLs, and
Windows AI DLLs.
The executable PE machine is checked independently for x64 and ARM64.

### Bootstrap correction (2.5.9)

The official crates.io `windows-reactor` 0.100.0 source uses
`src/native/winui/bootstrap.rs` to call the OS package-dependency APIs directly.
The old git setup helper's embedded bootstrap DLL and per-architecture hashes
are not the deployment contract of that published crate. Do not restore the
old DLL to make packaging pass: both layouts and MSIX archives now reject it.
The startup diagnostics and portable ZIP instructions follow the same contract.

This correction does not remove the shared-runtime manifest dependency or close
any clean-machine, packaged-startup, hardware, or Store certification gate.
The owner subsequently approved tagging and Store submission after confirming
the application works, explicitly proceeding despite the outstanding evidence.
That release approval does not mark the validation gates passed.

The manifest is derived from `AppxManifest.xml`, preserving the production
Store identity/publisher/version, both `TargetDeviceFamily` declarations, all
visual assets, `Windows.FullTrustApplication`, and all network,
`runFullTrust`, and `systemAIModels` capabilities. The transform sets the
architecture, points at the Reactor executable, and requires the Store
runtime dependency to be exactly (the pin is single-sourced from
`reactor-baselines/manifest.json` → `reactor_pin`, which `AppxManifest.xml`
already declares since the cutover decision):

```xml
<PackageDependency Name="Microsoft.WindowsAppRuntime.2"
                   MinVersion="2.4.0.0"
                   Publisher="CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US" />
```

The official pinned Reactor 0.100.0 initializes this framework directly through
Windows `TryCreatePackageDependency` / `AddPackageDependency` APIs, not a
bootstrap DLL. The framework package must be present on any
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
PE architecture, archive inspection, and rejection of a
dual runtime or app-local AI DLL.
