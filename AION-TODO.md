# Aion Instruct Integration TODO

This document tracks Aion Instruct source integration across WFDiag. The UI/provider checklist and the source fixes from the official SDK review are implemented. See [the integration review](docs/AION_INTEGRATION_REVIEW.md) for evidence and [preview setup](docs/AION_PREVIEW_SETUP.md) for deployment requirements. Live preview inference and Store/device validation remain release gates.

---

## 1. Status Overview

- [x] **Engine Layer (`wfdiag-native-phi`)**: Separate generated preview backend implemented; live preview inference remains pending.
  - [x] Native dynamic package attachment (`kernel32.dll` with `kernelbase.dll` fallback)
  - [x] Full `windows-core` 0.100 and `windows-link` 0.100 unification
  - [x] Separate preview and retail activation contracts (`AionInstructPreview.Text.LanguageModel` and `Microsoft.Windows.AI.Text.LanguageModel`); class-name switching alone is insufficient
  - [x] WinAppSDK 2.x `IncompatibleLowRankAdapter` (error 7) handling
- [x] **UI Shell (`apps/wfdiag`)**: Implemented.
  - [x] Shell wire parser recognizes `aion_instruct`
  - [x] Provider constants (`AI_PROVIDER_LABELS`, `AI_PROVIDER_IDS`, `PROVIDER_SETUP_LABELS`) include Aion Instruct
  - [x] Selector caption resolves `"Auto currently resolves to Aion Instruct (on-device)."`
  - [x] First-run onboarding proposes Aion Instruct on Copilot+ PCs
  - [x] Settings dialog suppresses model catalog dropdown for Aion Instruct
  - [x] Settings setup tab clarifies Aion Instruct requires no LAF token
- [x] **Provider & Facade Projection (`wfdiag-native-ai-provider`)**: Implemented.
  - [x] `project_provider_status` projects independent `aion_model_name` / `phi_model_name` fields for the two UI badges
- [x] **Workload Routing (`wfdiag-native-ai-report`)**: Implemented.
  - [x] `choose_report_provider` preserves `AIProvider::AionInstruct` on-device without forced local fallback

---

## 2. Implementation Checklist

### Step 1: UI Shell Constants (`apps/wfdiag/src/app/consts.rs`)

- [x] Expand `AI_PROVIDER_LABELS` to 12 items: add `"Aion Instruct (on-device)"`
- [x] Expand `AI_PROVIDER_IDS` to 12 items: add `"aion_instruct"`
- [x] Update `PROVIDER_SETUP_LABELS[0]` to `"On-device AI (Aion / Phi Silica)"`

### Step 2: Shell Routing Policy (`apps/wfdiag/src/app/policy.rs`)

- [x] Update `provider_from_wire` to include `AIProvider::AionInstruct` in `PROVIDERS: [AIProvider; 11]`
- [x] Refactor `OnboardingAction::PreferPhi` to `OnboardingAction::PreferOnDevice(&'static str)`
- [x] Add `(AIProvider::AionInstruct, true, true)` branch in `onboarding_candidates`
- [x] Update policy tests to verify Aion Instruct candidate matching and caption resolution

### Step 3: AI Screen Action Handler (`apps/wfdiag/src/screens/ai/update.rs`)

- [x] Handle `OnboardingAction::PreferOnDevice(wire)` to dispatch `AppCommand::SetProviderPreference`

### Step 4: Settings Dialog View (`apps/wfdiag/src/dialogs/settings/view.rs`)

- [x] Suppress model catalog row for both `PhiSilica` and `AionInstruct`
- [x] Update slot 0 description/hint to clarify tokenless operation for Aion Instruct

### Step 5: Provider Status Model Badges (`crates/wfdiag-native-ai-provider/src/lib.rs`)

- [x] Pass each backend’s own nonblank model name only to its provider; use `"Aion Instruct"` / `"Phi Silica"` defaults without copying one model name to both rows

### Step 6: Report Generation Auto-Routing (`crates/wfdiag-native-ai-report/src/lib.rs`)

- [x] Confirmed `choose_report_provider` already preserves Aion Instruct on-device (12k budget); protected the existing behavior with regression tests
- [x] Add unit test verifying Auto mode report provider selection with Aion Instruct

---

### Shared readiness gate and integration repairs

- [x] Generalize the facade gate to `OnDevicePreferenceGate`, using each selected model's own availability, readiness and failure message.
- [x] Reuse the facade gate in Settings and `SetProviderPreference`; include existing preference aliases and preserve package identity validation.
- [x] Replace stale onboarding handlers, add missing Aion fields to shell test fixtures, and look up selector assertions by wire ID.
- [x] Cover mixed Aion/Phi availability through real facade workers and complete a mocked Auto Aion report with Foundry also available.
- [x] Repair the Windows engine test's five-value result destructuring. Inject package identity into the retail check so the test cannot activate a runtime installed on the test host; preview probing now lives in its own backend.

---

## 3. Verification

Completed:

- Portable workspace `cargo check --locked --workspace --all-targets --exclude wfdiag`.
- Portable workspace `cargo clippy --locked --workspace --all-targets --exclude wfdiag -- -D warnings`.
- Portable workspace `cargo test --locked --workspace --exclude wfdiag`, 852 passed, including the new facade selection/report regressions.
- Readiness-script unit tests: 28 passed.
- Version synchronization and Store identity checks passed.
- Windows x64 and ARM64 workspace Clippy passed with `--locked --all-targets -- -D warnings`, both with no features and with `--features wfdiag/validation` (four configurations).
- `cargo fmt --all -- --check` and `git diff --check` passed.

The cross-builds emitted the documented `clang-cl` compiler-family detection warning; all four commands exited successfully. Shell tests were compiled, not executed.

Reproduction commands:

```bash
cargo check --locked --workspace --all-targets --exclude wfdiag
cargo clippy --locked --workspace --all-targets --exclude wfdiag -- -D warnings
cargo test --locked --workspace --exclude wfdiag

# Run for both x86_64-pc-windows-msvc and aarch64-pc-windows-msvc,
# first without features, then with --features wfdiag/validation.
PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin clippy --locked --workspace --all-targets --target aarch64-pc-windows-msvc -- -D warnings

python3 -m unittest scripts/test_check_reactor_readiness.py
python3 scripts/check-version-sync.py
python3 scripts/check-store-identity.py
cargo fmt --all -- --check
```

## 4. Pending Windows and hardware evidence

- [ ] Execute native shell tests on Windows in production and validation configurations. Cross-compilation type-checks these tests but does not execute them.
- [ ] Validate preview chat, reports, analysis, fix plans, attribution and UI on supported ARM64 Copilot+ hardware under Store identity. Validate legacy Phi separately; keep x64 Aion runtime evidence pending until a supported runtime is available.
- [ ] Complete the existing baseline, native-control, packaging and distribution readiness gates through their documented evidence protocols.

`check-reactor-readiness.py --json` still reports `ready: false` (exit 1), including baseline provenance/assets, backend parity, Aion Store validation and packaging/distribution evidence. No gate or baseline was changed to make this report pass. Mocked reports do not establish actual model activation, streaming behavior or device readiness.

## 5. Reopened by official SDK review (2026-09-28)

These source corrections address the review findings; hardware closure is tracked separately above. Evidence and pinned sources are in [AION_INTEGRATION_REVIEW.md](docs/AION_INTEGRATION_REVIEW.md).

- [x] **AION-01:** Generated separate preview bindings from pinned official WinMD; preview interfaces, async types and status values stay separate from retail. Added hash/ABI checks and a reproducible generator.
- [x] **AION-02:** Optional process dependencies attach the preview framework and Runtime 1.8 at their SDK floors. Added actionable errors, a read-only prerequisite checker and the documented Microsoft QNN setup flow; production Store pin unchanged.
- [x] **AION-03:** Separate cancellable model preparation (ten-minute native budget, eleven-minute outer budget) precedes the existing inference deadline. Chat/report lifecycle status reaches the shell. A simulated five-minute load regression passes.
- [x] **AION-04:** Explicit backend selection and distinct caches/fingerprints reach chat, report, analysis, prioritization and fix-plan generation. Removed OS/LAF/registry/environment identity guesses; independent probes and model fields support side-by-side installation.
- [x] **AION-05:** Retained the apartment guard through activation retry; repaired aligned package enumeration and size-query handling with growth retries. Removed the heuristic preview enumeration path entirely.
- [x] **AION-06:** Preview progress callbacks feed a bounded coalescing stream, preserve Unicode, reconcile final text, propagate cancellation and attribute successful preview responses. Prompt-only generation isolates conversation state across workloads and resets.
- [x] **AION-07:** Retained Store-only product policy consistently in probe, preparation and preference validation. Documented the deliberate difference from Microsoft's unpackaged examples and explicitly rejected unsupported x64 preview execution.

### Fix verification

- Portable workspace Clippy with `-D warnings` and 861 tests passed.
- Windows ARM64 native `wfdiag-native-phi` tests: 14 passed, including generated preview contract assertions. This run also found and fixed the existing Windows-only readable-error formatting failure.
- Windows x64/ARM64 workspace Clippy: production and validation configurations checked.
- Pinned metadata/ABI checks and exact binding regeneration passed.
- Version synchronization and Store identity checks passed; readiness still has the existing hardware/evidence blockers.
- Connected Windows host: ARM64, Runtime 1.8 installed, preview framework absent, QNN 1.8.30.0 installed (older than the SDK's documented 1.8.41 generation). Live preview inference was not attempted.
- PowerShell prerequisite checker syntax passed. Direct execution from the unsigned WSL share was rejected by the host's execution policy; equivalent read-only package queries supplied the inventory above. No execution policy was changed.

Mocked tests, ABI assertions and compilation do not establish actual preview model activation, NPU streaming or packaged DLL resolution. Those remain in the Windows/hardware checklist.
