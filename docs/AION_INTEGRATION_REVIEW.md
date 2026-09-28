# Aion Instruct sample integration review

Reviewed 2026-09-28 against WFDiag `5cc6cbf`.

**Conclusion at reviewed commit `5cc6cbf`: UI and provider routing were integrated, but native Aion Preview inference was not fully integrated.** The released preview has a different WinRT contract from the retail Windows AI contract currently used by WFDiag. Passing portable tests and Windows cross-compilation does not validate that boundary.

## Sources and method

- Microsoft sample repository, commit [`9d78b722323d573bafed410b250aa4ffd6e074f7`](https://github.com/microsoft/Aion-Instruct-Preview-Sample/tree/9d78b722323d573bafed410b250aa4ffd6e074f7): packaged WinUI, unpackaged WPF and console examples, client, manifests, bootstrap and QNN preparation tool.
- Official [release `v1.0.0.0`](https://github.com/microsoft/Aion-Instruct-Preview-Sample/releases/tag/v1.0.0.0), SDK asset `AionInstructPreview.Text.Framework.1.0.0.nupkg`. SHA-256: `208e739716ba35e87c8a7b0c10013fee63080c8cfb383793a706d0a5a73e7a7f`.
- Inspected the SDK's `ref/native/AionInstructPreview.Text.idl`, build targets and `lib/uap10.0/AionInstructPreview.Text.winmd`. Independently decoded the WinMD interface GUID attributes, method tables and enum constants with a metadata reader.
- Compared those contracts with the native runtime, generated bindings, provider composition, shipping resolvers, chat transport, analysis/fix-plan callers and package renderer. No model installation or Windows inference was performed.

The sample README describes similarity to Windows App SDK APIs. The **released metadata is more restrictive** and must be the source for generated bindings. The following findings are based on that metadata, not an assumption of binary compatibility.

## Findings

### AION-01 — High: preview activation uses the wrong interface and method layout

`crates/wfdiag-native-phi/src/runtime.rs:956` requests the retail `ILanguageModelStatics` interface from `AionInstructPreview.Text.LanguageModel`. The SDK declares a different interface:

| Contract | Preview SDK | Current WFDiag binding |
| --- | --- | --- |
| Statics IID | `f37c8314-9118-5036-8f18-4a071bf9103d` | `8f18f9af-6095-553b-8d9d-6bcc98026546` |
| Statics methods after IInspectable | `CreateAsync` | `GetReadyState`, `EnsureReadyAsync`, `CreateAsync` |
| Default model IID | `01216f3c-4cee-5f00-aedc-c705ed94c10e` | `6331c629-8c86-5bfe-8c4e-9ca5573cc14b` |
| Generation methods | On preview `ILanguageModel` | Queried through retail `ILanguageModel2` |
| Response status value 2 | `Error` | `BlockedByPolicy` |

The preview metadata has **no** `GetReadyState`, `EnsureReadyAsync`, `GetUsablePromptLength` or `LanguageModelOptions`. WFDiag invokes these retail APIs before or during generation (`runtime.rs:1002`, `:1180`, `:1158`, `:2232`). Its direct-DLL fallback also targets the retail class and DLL.

Consequences: the current code cannot consume the released preview through its declared contract. Normal activation rejects the incorrect interface request; fallback can reach the retail runtime instead. Merely substituting the preview IID would still leave invalid method layouts, object types and result interpretation.

Required correction: generate separate preview bindings from the pinned official WinMD; implement a typed preview backend and preserve the separate retail backend. Map status values explicitly. Do not hand-edit the existing generated file or reinterpret preview objects as retail objects. Source: the [released SDK](https://github.com/microsoft/Aion-Instruct-Preview-Sample/releases/tag/v1.0.0.0).

### AION-02 — High: packaged preview prerequisites are incomplete

The [sample manifest](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/Package.appxmanifest) declares both the preview framework and Windows App Runtime 1.8 in addition to the UI's Runtime 2 dependency. The SDK's targets explain that the preview framework cannot itself declare these framework dependencies and that Runtime 1.8 supplies its WinML stack.

WFDiag's `AppxManifest.xml:30` declares only Runtime 2. Dynamic attachment in `runtime.rs:824` attempts only the Aion framework. The legacy bootstrapper does not guarantee Runtime 1.8 on packaged builds. Furthermore, `scripts/build-reactor-msix-probe.py:275` removes every Windows App Runtime dependency and adds only the pinned Runtime 2 dependency, so adding 1.8 to the source manifest alone would not survive packaging.

The official [QNN acquisition tool](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/tools/AcquireQnnEp/Program.cs) also prepares a usable NPU execution provider. WFDiag has no equivalent preview prerequisite check or setup explanation. This is a clean-machine integration gap; an already provisioned developer machine can conceal it.

Required correction: define and validate the preview deployment profile, graph dependencies and actionable prerequisite diagnostics. Preserve the single-sourced Reactor pin and shipping release policy; review preview deployment separately before changing Store manifests. Do not add the QNN main package as a static framework dependency.

### AION-03 — High: initial model loading exceeds our deadlines

The [sample client](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/AionInstructClient.cs) keeps a loading state while `CreateAsync` performs the initial NPU compilation, documented as approximately four to five minutes. WFDiag cancels creation after two minutes (`runtime.rs:1901`); chat has an outer 180-second turn budget (`crates/wfdiag-native-ai-chat/src/engine.rs:17`).

A first-run preview load taking the documented duration cannot complete under those budgets. Required correction: give model preparation its own cancellable lifecycle and loading UI before inference begins. Keep generation deadlines nested correctly; changing only the inner timeout leaves the outer timeout broken.

### AION-04 — High: reported model identity is inferred rather than tied to activation

`runtime.rs:1455` reports Aion when LAF unlock fails, or based on a Windows build threshold, registry strings or an environment override. Those facts do not prove which model was activated. The existing `docs/PHI_SILICA.md` investigation already records retail Phi `GetReadyState` succeeding while LAF was unavailable.

This can label a retail fallback as Aion and assign it Aion's larger context budget. Both provider IDs also use the same `phi_provider` adapter (`crates/wfdiag-app/src/ports/ai.rs:491`); the requested engine is not passed to native generation. The new badges correctly project their input, but that input can be wrong.

Required correction: carry the successfully selected backend through probing, generation, caching and result attribution. Distinguish proven preview identity from unknown retail model identity. Explicit selection must not silently activate another backend. Any test override must obey the existing validation-feature policy.

### AION-05 — Medium: preview detection has two independent lifetime/API errors

In `runtime.rs:940`, `let _ = enter_winrt_apartment()` immediately drops the guard before retrying activation. `WinRtApartment::drop` calls `RoUninitialize`. Keep the guard alive for the operation, preferably by initializing at the outer probe boundary.

In `runtime.rs:1501`, the package enumeration rejects every nonzero result from the initial zero-buffer call. Windows documents `ERROR_INSUFFICIENT_BUFFER` as the normal result carrying the required allocation size. The same error exists in `framework_package_dirs` around line 630. It prevents these fallback enumeration paths from using the returned size. See [GetCurrentPackageInfo](https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getcurrentpackageinfo).

Required correction: repair both buffer-size checks and apartment ownership, with injected native-call regression tests. These repairs do not replace the separate preview binding work.

### AION-06 — Medium: Aion is advertised as streaming but sends only the final response

The [sample client](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/AionInstructClient.cs) forwards progress deltas during generation. WFDiag's `PhiChatProvider::stream` waits for the entire response and then sends one string (`crates/wfdiag-native-phi/src/lib.rs:103`). The native polling helper never installs a progress callback, while `capabilities(AionInstruct)` advertises streaming.

Required correction: forward preview progress into the existing bounded stream, handle cancellation and reconcile the final result without duplicating text. Populate actual model/backend attribution. The shared adapter currently reports an empty `actual_models` list.

### AION-07 — Scope mismatch: preview supports unpackaged apps, WFDiag remains Store-only

Microsoft provides a working [unpackaged console example](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/unpackaged-console/Program.cs). WFDiag's lower runtime attempts this path, but `validate_provider_preference` rejects explicit Aion without package identity (`crates/wfdiag-native-ai-provider/src/lib.rs:188`).

Store-only is the repository's current product policy. Supporting the example's unpackaged mode requires a deliberate policy decision and matching upper-layer validation. If Store-only remains the decision, describe dynamic preview support accordingly and do not claim unpackaged parity. Phi's existing identity requirement remains separate.

## What is already connected

The Aion wire ID, selector, onboarding, setup-pane mapping, per-provider readiness gate, model badge projection, local-first routing and report selection are wired. Chat/report and analysis/fix-plan callers reach the native adapter. Existing portable tests exercise those seams through mocks.

The sample retains a native conversation context; WFDiag instead flattens its retained conversation into each prompt. That is a valid design option because the preview supports a prompt-only overload. Native context caching is optional and would require session isolation and reset handling. The preview's missing prompt-length API still requires a different context-limit strategy using its own response status.

The current preview is ARM64/Snapdragon-only according to Microsoft's [sample prerequisites](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/README.md#prerequisites). An x64 build passing does not establish a supported x64 preview runtime. Microsoft's [transparency notes](https://github.com/microsoft/Aion-Instruct-Preview-Sample/blob/9d78b722323d573bafed410b250aa4ffd6e074f7/TRANSPARENCY_NOTES.md) describe this release as a prototyping preview; future retail behavior must be verified against its released contract.

## Closure criteria

1. Pin the SDK provenance and generate separate preview bindings; add metadata contract checks for interfaces, methods and response enums.
2. Introduce explicit backend selection and truthful availability/model attribution across every native workload.
3. Fix graph enumeration/apartment ownership and verify packaged preview prerequisites on a clean supported machine.
4. Separate cancellable model loading from bounded inference; test a simulated load exceeding two minutes.
5. Test streaming, cancellation, final-text reconciliation, context overflow and chat reset at the adapter boundary.
6. On an ARM64 Copilot+ PC, run the official sample and WFDiag against the same installed SDK, including cold load, warm load, chat, report, analysis and fix-plan generation. Record actual backend identity and package versions. Keep x64 runtime validation pending until a supported runtime is available.

The earlier 852 portable tests and four Windows Clippy configurations remain useful evidence for the implemented UI/facade work. They do not close these native integration findings. The initial review changed no runtime code. The subsequent fixes are recorded below.

## Subsequent source fixes

The implementation now uses `aion_bindings.rs`, generated from the pinned SDK
WinMD, and a separate `aion.rs` backend. Preview activation never requests retail
interfaces. Explicit backend selection reaches all native workloads, and independent
probe/model fields support simultaneous preview and retail availability.

`ondevice.rs` separates cancellable preparation from inference. Chat/report emit
lifecycle status rather than fabricated tool activity. Preview progress is bounded,
coalesced under backpressure and reconciled against final text. The retail API's
actual model identity remains unknown instead of being inferred from OS/LAF state.

Deployment uses optional process dependencies with the SDK's preview/Runtime 1.8
floors; it preserves the production Runtime 2 pin. See [preview setup](AION_PREVIEW_SETUP.md)
for QNN preparation, the read-only checker and the retained Store-only policy.
The apartment retry now owns its guard, package enumeration accepts the normal
size-query result with aligned storage, and the heuristic preview enumeration is removed.

See [AION-TODO.md](../AION-TODO.md#fix-verification) for verification and remaining
hardware evidence. All source findings have corrections; live preview validation
is still open because the connected ARM64 host lacks the preview framework and has
an older QNN package. No readiness gate has been weakened.
