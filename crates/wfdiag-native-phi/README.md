# wfdiag-native-phi

UI-framework-neutral owner of WFDiag's Windows AI / Phi Silica runtime.

The crate keeps the shipping 2.5.8 behavior in one process-wide implementation:

- registered-package-identity gate before any WinRT or DLL work;
- Windows build and Limited Access Feature checks;
- standard WinRT activation with the existing direct-DLL fallback;
- one serialized, invalidatable `LanguageModel` cache shared by status and generation;
- exact prompt-fit measurement and generation response handling; and
- the reviewed `windows-bindgen` projection used by both Tauri and Reactor.

`WindowsPhiStatusSource` implements the provider-management crate's
`PhiStatusSource` boundary. A native shell can pass it directly to
`ProviderProbeBundle::shipping_networks`. Blocking COM/WinRT status work runs
through `spawn_blocking`; an unpackaged executable returns the established
Store-required status before runtime initialization.

Tauri command attributes and native UI code intentionally remain outside this
crate. Regenerate `src/windows_ai_bindings.rs` only through:

```text
python3 scripts/build-cross.py generate-bindings
```

Do not hand-edit the generated projection.
