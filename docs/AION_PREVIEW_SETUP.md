# Optional Aion Instruct Preview deployment

WFDiag supports the pinned preview SDK described in
[AION_INTEGRATION_REVIEW.md](AION_INTEGRATION_REVIEW.md). This is a separate backend
from retail Windows AI/Phi Silica. A future retail Aion release needs its own
contract review; an OS build number does not identify the loaded model.

## Supported profile

- ARM64 Snapdragon Copilot+ PC with compatible NPU drivers.
- Registered WFDiag package identity (Store or the existing development identity protocol).
- `Microsoft.AionInstructPreview.Framework.1.0` >= `1.0.0.0`.
- `Microsoft.WindowsAppRuntime.1.8` >= `8000.836.2153.0`, supplying preview WinML.
- Qualcomm QNN execution provider 1.8, prepared through Microsoft's WinML catalog tool.

The production UI's Runtime 2 dependency remains pinned by
`reactor-baselines/manifest.json`. Preview dependencies are optional process
attachments checked before preview activation. They are not added to every Store
installation, and the manifest renderer does not need a second hard-coded pin.
Failure to attach a dependency reports its package family to the user and remains
retryable after setup. Successful dependencies remain attached until process exit.
The SDK handles QNN registration; the QNN package is a main package and must **not**
be added as a static framework dependency.

WFDiag continues to require package identity for both providers. Microsoft's
unpackaged sample demonstrates an additional deployment mode; WFDiag does not
claim that mode. x64 preview availability is rejected explicitly. Legacy Phi
continues to use its existing identity, activation and LAF path.

## Prepare a validation machine

1. Use Microsoft's [sample at the reviewed commit](https://github.com/microsoft/Aion-Instruct-Preview-Sample/tree/9d78b722323d573bafed410b250aa4ffd6e074f7)
   and signed [release v1.0.0.0](https://github.com/microsoft/Aion-Instruct-Preview-Sample/releases/tag/v1.0.0.0).
   Follow its prerequisites to install the ARM64 preview framework and Runtime 1.8.
2. From that checkout, run the official QNN acquisition tool:

   ```powershell
   dotnet run --project .\tools\AcquireQnnEp\AcquireQnnEp.csproj -c Release -p:Platform=ARM64
   ```

   This uses `ExecutionProviderCatalog`, prepares the QNN provider and verifies
   registration. It can download/install components. WFDiag never runs this setup
   tool automatically or extracts its source into an application command.
3. From this repository, run the read-only check:

   ```powershell
   .\scripts\check-aion-prerequisites.ps1
   ```

4. Run Microsoft's sample first, then the current WFDiag build under registered
   identity on the same machine. Package discovery alone does not validate the
   runtime's DLL resolution, NPU drivers or ability to generate text.

## Runtime behavior and evidence to capture

The preview has no readiness or prompt-measurement API. Discovery checks the
preview factory with its actual interface ID; it does not eagerly compile the
model. Chat/report status displays preparation before the first response. The
model-load budget is ten minutes, inside the eleven-minute preparation budget;
only then does the 180-second chat/report turn budget begin. Native inference is
bounded to 150 seconds. Cancellation or dropping the future cancels native work.

Prompts contain the retained conversation and evidence. The prompt-only preview
overload avoids shared native conversation state between chats, reports and
analysis. A context-overflow status is reported explicitly; WFDiag does not call
retail prompt-length or options APIs on the preview object.

Record package versions and results for:

- Cold preparation exceeding two minutes, warm load and cancellation during load.
- Progress deltas before completion, exact final text without duplication, and cancellation during inference.
- Separate chat sessions, reset, oversized context and recovery.
- Chat, report, per-task analysis, prioritization and fix-plan generation.
- Explicit Aion/Phi selection with both installed; a preview failure must not silently activate retail Phi.
- Model attribution (`Aion Instruct Preview` after successful preview generation).
- Missing framework/WinML/QNN, unsupported x64 and a loose executable.

Store/package/readiness gates stay pending until that evidence exists. Cross-builds
and hermetic tests do not close hardware gates.
