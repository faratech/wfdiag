# Aion preview contract

`AionInstructPreview.Text.winmd` is the unmodified contract from Microsoft's
[SDK release v1.0.0.0](https://github.com/microsoft/Aion-Instruct-Preview-Sample/releases/tag/v1.0.0.0),
asset `AionInstructPreview.Text.Framework.1.0.0.nupkg`, path
`lib/uap10.0/AionInstructPreview.Text.winmd`.

Copyright (c) 2026 Microsoft Corporation (SDK package metadata).
This is interface metadata only; no model or runtime binaries are redistributed.

- SDK archive SHA-256: `208e739716ba35e87c8a7b0c10013fee63080c8cfb383793a706d0a5a73e7a7f`
- WinMD SHA-256: `edafb7edb0ff7e93ef20bcef1fd8a5fb17048e6724ccebe4858e14f0cafc5fde`
- Generator: `windows-bindgen = 0.100.0`, pinned independently under `scripts/aion-bindings`.

Regenerate from the repository root:

```sh
python3 scripts/check-aion-bindings.py
cargo run --locked --manifest-path scripts/aion-bindings/Cargo.toml
```

Ordinary product builds use the checked-in generated file and need no downloads.
The generator never edits `windows_ai_bindings.rs` (the separate retail contract).
To verify reproducibility without editing the checked-in bindings:

```sh
cargo run --locked --manifest-path scripts/aion-bindings/Cargo.toml -- --check
```
