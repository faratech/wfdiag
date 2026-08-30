# Design QA — native Reactor

## Authority and method

The visual authority is the Microsoft Store-signed WFDiag 2.5.8 package installed on this host,
not the earlier 2.5.4 development captures. The 18 immutable source states, their exact logical
viewports, source version, and SHA-256 hashes are recorded in
`reactor-baselines/manifest.json` and stored under
`reactor-baselines/captures/store-2.5.8`.

`reactor-spike` is a manually composed native WinUI 3 shell through Windows Reactor at the pinned
`windows-rs` revision. It does not host WebView2, HTML, CSS, JavaScript, the React bundle, or a
web/native UI bridge.

The ARM64 self-contained build is launched on Windows through WSL interop and captured with
`scripts/capture-window.ps1`. That helper enters a per-monitor-v2 DPI context, sizes the DWM-visible
frame to the requested logical viewport, captures the native window, and density-normalizes only
when necessary. Every review puts the Store image on the left and the native image on the right in
one combined input; screenshots viewed independently are not treated as QA.

## Current paired evidence

The final ARM64 candidate has matching-viewport native captures and source-left/native-right pairs
for all 18 Store 2.5.8 states under `captures-2.5.8/final`. The complete contact sheet is
`captures-2.5.8/final/all-18-comparisons-contact.png`. The primary-state paths retained below are:

| State | Viewport | Native capture | Combined review |
| --- | --- | --- | --- |
| Diagnostics empty | 1440×1000 | `captures-2.5.8/diagnostics-empty-desktop-dark-reactor-current.png` | `captures-2.5.8/diagnostics-empty-desktop-dark-comparison-current.png` |
| Diagnostics live system (native ARM64) | 1440×1000 | `captures-2.5.8/live-system-validation/diagnostics-live-system-20260830-114311.png` | `captures-2.5.8/live-system-validation/diagnostics-empty-desktop-dark-live-system-store-left-reactor-right.png` |
| Diagnostics populated | 1440×900 | `captures-2.5.8/diagnostics-populated-desktop-dark-reactor-current.png` | `captures-2.5.8/diagnostics-populated-desktop-dark-comparison-current.png` |
| Live Monitor populated | 1440×900 | `captures-2.5.8/monitor-populated-desktop-dark-reactor-current.png` | `captures-2.5.8/monitor-populated-desktop-dark-comparison-current.png` |
| Processes populated | 1440×900 | `captures-2.5.8/live-process-validation/deterministic-processes-populated-1440x900.png` | `captures-2.5.8/live-process-validation/processes-populated-store-left-reactor-right.png` |
| AI empty | 1440×1000 | `captures-2.5.8/ai-empty-desktop-dark-reactor-current.png` | `captures-2.5.8/ai-empty-desktop-dark-comparison-current.png` |
| Issues empty | 1440×1000 | `captures-2.5.8/issues-empty-desktop-dark-reactor-current.png` | `captures-2.5.8/issues-empty-desktop-dark-comparison-current.png` |
| Issues populated | 1440×900 | `captures-2.5.8/issues-populated-desktop-dark-reactor-current.png` | `captures-2.5.8/issues-populated-desktop-dark-comparison-current.png` |
| History comparison | 1440×900 | `captures-2.5.8/history-comparison-desktop-dark-reactor-current.png` | `captures-2.5.8/history-comparison-desktop-dark-comparison-current.png` |
| Settings top | 1440×900 | `captures-2.5.8/settings-top-desktop-dark-reactor-current.png` | `captures-2.5.8/settings-top-desktop-dark-comparison-current.png` |

The exact Store wallpaper treatment uses CSS blur, saturation, scaling, dimming, and per-element
backdrop blur. Reactor does not expose the same per-element backdrop surface at the pinned
revision. The native shell therefore embeds deterministic derivatives of the protected light and
OLED wallpaper assets behind native tint and borders. The current dark derivative was aligned by
cross-correlation; in control-free regions its normalized correlation with the Store capture is
about 0.996 and its mean absolute RGB error is below one channel value.

## What now matches closely

- The 230 px information architecture, title area, six destinations, Tools section, Settings,
  About, Collapse, machine card, floating panel, status footer, content gutters, and dark hierarchy.
- The empty Diagnostics hero geometry, real Font Awesome stethoscope asset, two-line leading,
  primary CTA color, and current 2.5.8 footer content.
- Populated Diagnostics statistics and all 17 task labels/durations, plus the visible raw Computer
  System key/value output from the Store reference.
- Processes table density and columns, Issues card hierarchy, History comparison structure, AI
  workspace structure, and the Settings top modal are represented with native controls.
- Native title/navigation/buttons/inputs/settings, pane collapse, theme switching, process
  filtering, monitor pause/resume/refresh, keyboard refresh, and accessibility names exist without
  a browser host.
- `NativeMonitorRuntime` now delivers live CPU, memory, storage, network, GPU, and NPU samples into
  Reactor through `wfdiag-ui-core`; the Tauri emitter is not used by the native shell.
- The live Processes page now passes ARM64 expanded-desktop load, pause/resume, refresh, filtering,
  PID sorting, selection/details, scroll and page-2 testing. Its native virtualized rows align with
  the fixed header in the paired 2.5.8 comparison, with no WebView modules or WER reports.
- A fixture-free ARM64 Diagnostics run now matches the installed Store 2.5.8 machine card visibly
  at the same 1440×1000 / 150% viewport. UI Automation additionally proves the non-visible native
  architecture contract, exact 2.5.8 footer, local XAML load, graceful close, and no WebView/WER
  evidence. The combined review retains the Store frame on the left and this live frame on the
  right.

## Remaining parity work

- Complete acceptance review and any final pixel corrections for the newly paired empty,
  compact-AI, conversation, issue-handoff, and Settings-bottom states.
- Complete same-state evidence for light/system/high-contrast themes, reduced motion, keyboard
  focus, and 100%/150%/200% DPI on native x64 and ARM64 Windows.
- Complete compact/collapsed and x64 runtime coverage for Processes. Replace the remaining
  fixture-only issue and AI paths and finish History, Settings, export, remediation, scan
  persistence, tray, notification, clipboard, packaged-update validation, and single-instance
  backend/desktop-service adapters.
- Validate UI Automation roles, names, keyboard traversal, text scaling, high contrast, and screen
  reader output on the candidate runtime.
- Finish clean-machine framework-dependent Store/MSIX and self-contained direct-installer matrices.
- Resolve the pinned Reactor revision's upstream window lifecycle and accelerator gaps, or carry a
  reviewed native extension until an official usable release supplies them.

Native WinUI and the CSS implementation do not use the same font rasterizer, focus visuals, input
chrome, or backdrop pipeline. Small raster-level differences can remain even after all measured
layout, content, color, and behavior requirements match. They must be recorded explicitly; missing
information, changed behavior, incorrect state, or lost brand hierarchy are not acceptable as
native-fidelity exceptions.

**Current result: native direction implemented and current-version parity work active; production
cutover remains blocked.**

Do not replace the shipping Tauri UI until every manifest state has a same-version combined review,
the backend and desktop-service adapters are complete, and the runtime/packaging/device gates pass.
WebView2 is not part of the selected UI architecture.
