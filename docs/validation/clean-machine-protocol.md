# Clean-machine validation protocol (windows-reactor cutover)

Manual protocol for the gates that cannot be closed from the dev box or CI.
Every step lists the evidence to attach to
`reactor-baselines/manifest.json` before the corresponding gate can move to
`passed`. Work through it on a **clean machine snapshot** (x64 and ARM64),
never the dev box.

## Prerequisites (per machine)

- Windows 11 with the Store app **uninstalled** (`Get-AppxPackage
  *WindowsForumDiagnostics*` returns nothing).
- `scripts/check-external-gates.py --skip-network` run first: the packaging
  pre-flight must report `ready`.
- Signed artifacts from the release pipeline: Store MSIX bundle, MSI, NSIS,
  portable directory — recorded with SHA-256 in the run notes.

## 1. Store MSIX (gate: cutover.store_packaging_validation)

| # | Step | Acceptance |
|---|------|------------|
| 1.1 | Install the bundle (`Add-AppxPackage`) | Install succeeds; no dep-prompts for the Windows App Runtime |
| 1.2 | Identity readback | `Get-AppxPackage` Name/Publisher/PFN match `AppxManifest.xml` exactly |
| 1.3 | Capability readback | Manifest capabilities (incl. `systemAIModels`) unchanged |
| 1.4 | Launch + `test-reactor-live-system.ps1` | All checks pass, zero WER events |
| 1.5 | Flow suites | `test-reactor-chat.ps1`, `test-reactor-report.ps1`, `test-reactor-remediation.ps1` pass |
| 1.6 | Upgrade | Installing a newer build preserves scans + settings |
| 1.7 | Uninstall | Clean removal; `%LOCALAPPDATA%\WFDiag` credentials retained per policy |

## 2. Direct distribution (gate: cutover.direct_distribution_validation)

| # | Step | Acceptance |
|---|------|------------|
| 2.1 | MSI install / repair / uninstall | Clean install; `msiexec /fv` repair succeeds |
| 2.2 | NSIS install / uninstall | Same acceptance as 2.1 |
| 2.3 | Portable directory | Launches from any writable directory; no writes outside its roots |
| 2.4 | Framework-dependent vs self-contained | Record which Windows App Runtime strategy each artifact uses; only ONE strategy may ship |
| 2.5 | Update check | GitHub release check fires; Store installs stay silent |

## 3. On-device AI (gate: cutover.aion_store_validation)

On a real Copilot+ device (40+ TOPS), with the Store-signed build:

- Phi/Aion readiness reflects the documented Store-required behavior.
- Chat with the on-device provider completes without the LAF `Unavailable`
  defect (`docs/` Phi Silica notes apply).
- Record `Get-AppxPackage Microsoft.WindowsAppRuntime*` output: the runtime
  strategy must match the Store MSIX decision.

## 4. Runtime alignment decision (gate: runtime.alignment)

Fill in after section 3 on both architectures:

- Chosen strategy: **1.8 framework / 2.x framework / self-contained** (circle one).
- Evidence: the section-3 capture set + installed-framework listings from
  every test machine.
- If the Store manifest `PackageDependency` changes, update
  `AppxManifest.xml`, `reactor-baselines/manifest.json` (`reactor_pin`), and
  re-run `scripts/check-reactor-readiness.py` in the SAME reviewed change.

## 5. Sign-off

| Gate | Machine | Date | Result | Evidence paths |
|------|---------|------|--------|----------------|
| Store MSIX | | | | |
| MSI/NSIS/portable | | | | |
| Aion/Phi on device | | | | |
| Runtime decision | | | | |
