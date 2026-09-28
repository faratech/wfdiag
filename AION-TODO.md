# Aion Instruct Integration TODO

This document tracks Aion Instruct source integration across WFDiag. The implementation checklist is complete; real Windows UI execution and Copilot+ device validation remain separate release gates.

---

## 1. Status Overview

- [x] **Engine Layer (`wfdiag-native-phi`)**: Implemented in the preceding engine commits; live-device validation remains pending.
  - [x] Native dynamic package attachment (`kernel32.dll` with `kernelbase.dll` fallback)
  - [x] Full `windows-core` 0.100 and `windows-link` 0.100 unification
  - [x] Dual activation pathways (`AionInstructPreview.Text.LanguageModel` and `Microsoft.Windows.AI.Text.LanguageModel`)
  - [x] WinAppSDK 2.x `IncompatibleLowRankAdapter` (error 7) handling
- [x] **UI Shell (`apps/wfdiag`)**: Implemented.
  - [x] Shell wire parser recognizes `aion_instruct`
  - [x] Provider constants (`AI_PROVIDER_LABELS`, `AI_PROVIDER_IDS`, `PROVIDER_SETUP_LABELS`) include Aion Instruct
  - [x] Selector caption resolves `"Auto currently resolves to Aion Instruct (on-device)."`
  - [x] First-run onboarding proposes Aion Instruct on Copilot+ PCs
  - [x] Settings dialog suppresses model catalog dropdown for Aion Instruct
  - [x] Settings setup tab clarifies Aion Instruct requires no LAF token
- [x] **Provider & Facade Projection (`wfdiag-native-ai-provider`)**: Implemented.
  - [x] `project_provider_status` passes `ondevice_model_name` for Aion Instruct and Phi Silica UI badges
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

- [x] Pass nonblank `probes.ondevice_model_name` only to the matching detected provider; use `"Aion Instruct"` / `"Phi Silica"` defaults without copying one model name to both rows

### Step 6: Report Generation Auto-Routing (`crates/wfdiag-native-ai-report/src/lib.rs`)

- [x] Confirmed `choose_report_provider` already preserves Aion Instruct on-device (12k budget); protected the existing behavior with regression tests
- [x] Add unit test verifying Auto mode report provider selection with Aion Instruct

---

### Shared readiness gate and integration repairs

- [x] Generalize the facade gate to `OnDevicePreferenceGate`, using each selected model's own availability, readiness and failure message.
- [x] Reuse the facade gate in Settings and `SetProviderPreference`; include existing preference aliases and preserve package identity validation.
- [x] Replace stale onboarding handlers, add missing Aion fields to shell test fixtures, and look up selector assertions by wire ID.
- [x] Cover mixed Aion/Phi availability through real facade workers and complete a mocked Auto Aion report with Foundry also available.
- [x] Repair the Windows engine test's five-value result destructuring. Inject package identity and preview availability into its existing check so the test cannot activate a runtime installed on the test host; production probe ordering is preserved.

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
- [ ] Validate Aion chat and report generation, model attribution, selector/onboarding behavior and legacy Phi behavior on real Copilot+ x64 and ARM64 devices under the Store identity.
- [ ] Complete the existing baseline, native-control, packaging and distribution readiness gates through their documented evidence protocols.

`check-reactor-readiness.py --json` still reports `ready: false` (exit 1), including baseline provenance/assets, backend parity, Aion Store validation and packaging/distribution evidence. No gate or baseline was changed to make this report pass. Mocked reports do not establish actual model activation, streaming behavior or device readiness.
