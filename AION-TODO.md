# Aion Instruct Integration TODO

This document tracks the tasks required to achieve 100% native integration of Aion Instruct across the entire WFDiag application stack.

---

## 1. Status Overview

- [x] **Engine Layer (`wfdiag-native-phi`)**: 100% complete.
  - [x] Native dynamic package attachment (`kernel32.dll` with `kernelbase.dll` fallback)
  - [x] Full `windows-core` 0.100 and `windows-link` 0.100 unification
  - [x] Dual activation pathways (`AionInstructPreview.Text.LanguageModel` and `Microsoft.Windows.AI.Text.LanguageModel`)
  - [x] WinAppSDK 2.x `IncompatibleLowRankAdapter` (error 7) handling
- [ ] **UI Shell (`apps/wfdiag`)**: In progress.
  - [ ] Shell wire parser recognizes `aion_instruct`
  - [ ] Provider constants (`AI_PROVIDER_LABELS`, `AI_PROVIDER_IDS`, `PROVIDER_SETUP_LABELS`) include Aion Instruct
  - [ ] Selector caption resolves `"Auto currently resolves to Aion Instruct (on-device)."`
  - [ ] First-run onboarding proposes Aion Instruct on Copilot+ PCs
  - [ ] Settings dialog suppresses model catalog dropdown for Aion Instruct
  - [ ] Settings setup tab clarifies Aion Instruct requires no LAF token
- [ ] **Provider & Facade Projection (`wfdiag-native-ai-provider`)**: In progress.
  - [ ] `project_provider_status` passes `ondevice_model_name` for Aion Instruct and Phi Silica UI badges
- [ ] **Workload Routing (`wfdiag-native-ai-report`)**: In progress.
  - [ ] `choose_report_provider` preserves `AIProvider::AionInstruct` on-device without forced local fallback

---

## 2. Implementation Checklist

### Step 1: UI Shell Constants (`apps/wfdiag/src/app/consts.rs`)
- [ ] Expand `AI_PROVIDER_LABELS` to 12 items: add `"Aion Instruct (on-device)"`
- [ ] Expand `AI_PROVIDER_IDS` to 12 items: add `"aion_instruct"`
- [ ] Update `PROVIDER_SETUP_LABELS[0]` to `"On-device AI (Aion / Phi Silica)"`

### Step 2: Shell Routing Policy (`apps/wfdiag/src/app/policy.rs`)
- [ ] Update `provider_from_wire` to include `AIProvider::AionInstruct` in `PROVIDERS: [AIProvider; 11]`
- [ ] Refactor `OnboardingAction::PreferPhi` to `OnboardingAction::PreferOnDevice(&'static str)`
- [ ] Add `(AIProvider::AionInstruct, true, true)` branch in `onboarding_candidates`
- [ ] Update policy tests to verify Aion Instruct candidate matching and caption resolution

### Step 3: AI Screen Action Handler (`apps/wfdiag/src/screens/ai/update.rs`)
- [ ] Handle `OnboardingAction::PreferOnDevice(wire)` to dispatch `AppCommand::SetProviderPreference`

### Step 4: Settings Dialog View (`apps/wfdiag/src/dialogs/settings/view.rs`)
- [ ] Suppress model catalog row for both `PhiSilica` and `AionInstruct`
- [ ] Update slot 0 description/hint to clarify tokenless operation for Aion Instruct

### Step 5: Provider Status Model Badges (`crates/wfdiag-native-ai-provider/src/lib.rs`)
- [ ] Pass `probes.ondevice_model_name` (defaulting to `"Aion Instruct"` / `"Phi Silica"`) into `provider_info`

### Step 6: Report Generation Auto-Routing (`crates/wfdiag-native-ai-report/src/lib.rs`)
- [ ] Ensure `choose_report_provider` allows Aion Instruct to generate reports on-device (12k budget)
- [ ] Add unit test verifying Auto mode report provider selection with Aion Instruct

---

## 3. Verification Commands

```bash
# Portable engine tests
cargo check --workspace --all-targets --exclude wfdiag
cargo test --workspace --exclude wfdiag

# Windows shell cross-checks
PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin check --workspace --target x86_64-pc-windows-msvc
PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin clippy --workspace --target aarch64-pc-windows-msvc -- -D warnings

# Scripts and formatting
python3 -m unittest scripts/test_check_reactor_readiness.py
python3 scripts/check-version-sync.py
python3 scripts/check-store-identity.py
cargo fmt --all -- --check
```
