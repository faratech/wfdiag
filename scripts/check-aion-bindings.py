#!/usr/bin/env python3
"""Verify pinned preview metadata and the generated contract's essential ABI."""
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PHI = ROOT / "crates/wfdiag-native-phi"
EXPECTED = "edafb7edb0ff7e93ef20bcef1fd8a5fb17048e6724ccebe4858e14f0cafc5fde"
actual = hashlib.sha256((PHI / "metadata/AionInstructPreview.Text.winmd").read_bytes()).hexdigest()
if actual != EXPECTED:
    raise SystemExit("Aion metadata differs from the reviewed SDK contract")
source = (PHI / "src/aion_bindings.rs").read_text()
for required in (
    "0xf37c8314_9118_5036_8f18_4a071bf9103d",
    "0x01216f3c_4cee_5f00_aedc_c705ed94c10e",
    "pub const Error: Self = Self(2)",
    "pub const PromptLargerThanContext: Self = Self(3)",
):
    if required not in source:
        raise SystemExit(f"Missing preview ABI contract: {required}")
for forbidden in ("GetReadyState", "EnsureReadyAsync", "GetUsablePromptLength", "LanguageModelOptions"):
    if forbidden in source:
        raise SystemExit(f"Retail API leaked into preview bindings: {forbidden}")
print("Aion metadata hash and preview ABI checks passed")
