#!/usr/bin/env python3
"""
Cross-compilation build script for WindowsForum Diagnostics.
Builds both x64 and ARM64 Windows executables from Linux/WSL.
"""

import os
import subprocess
import sys
import shutil
import multiprocessing
import json
from pathlib import Path

# Build configuration
PROJECT_DIR = Path(__file__).parent.resolve()
SRC_TAURI = PROJECT_DIR / "src-tauri"
TARGET_DIR = SRC_TAURI / "target"
OUTPUT_DIR = Path("/mnt/c/code")  # Always output to Windows drive
MSIX_DIR = PROJECT_DIR / "msix-build"

# Cross-compilation settings
XWIN_CACHE = Path.home() / ".cache" / "cargo-xwin" / "xwin"

# Windows SDK path (for MakeAppx.exe and signtool.exe)
SDK_BIN = Path("/mnt/c/Program Files (x86)/Windows Kits/10/bin/10.0.26100.0/x64")

# MSIX package configuration
PROJECT_NAME = "WindowsForum_Diagnostics"
PUBLISHER = "CN=ABDB6B3F-DF9E-447D-BC0E-4DA7BAFD14C4"

# Certificate configuration for self-signing
CERT_PATH = OUTPUT_DIR / "wfdiag-selfsign.pfx"
CERT_PASSWORD = "WFDiag2024!"

# Get CPU count for parallel builds
CPU_COUNT = multiprocessing.cpu_count()

TARGETS = {
    "x64": {
        "triple": "x86_64-pc-windows-msvc",
        "arch_dir": "x86_64",
    },
    "arm64": {
        "triple": "aarch64-pc-windows-msvc",
        "arch_dir": "aarch64",
    },
}


def get_env_for_target(target_name: str) -> dict:
    """Get environment variables for cross-compilation."""
    target = TARGETS[target_name]
    triple = target["triple"]
    arch_dir = target["arch_dir"]

    # Convert triple to environment variable format (replace - with _)
    env_triple = triple.replace("-", "_")

    env = os.environ.copy()

    # Set clang as the C compiler with proper target and include paths
    env[f"CC_{env_triple}"] = "clang"
    env[f"AR_{env_triple}"] = "llvm-lib"
    env[f"CFLAGS_{env_triple}"] = (
        f"--target={triple} "
        f"-I{XWIN_CACHE}/crt/include "
        f"-I{XWIN_CACHE}/sdk/include/ucrt "
        f"-I{XWIN_CACHE}/sdk/include/um "
        f"-I{XWIN_CACHE}/sdk/include/shared"
    )

    # Set Rust flags for linking
    env[f"CARGO_TARGET_{env_triple.upper()}_RUSTFLAGS"] = (
        f"-Lnative={XWIN_CACHE}/crt/lib/{arch_dir} "
        f"-Lnative={XWIN_CACHE}/sdk/lib/um/{arch_dir} "
        f"-Lnative={XWIN_CACHE}/sdk/lib/ucrt/{arch_dir}"
    )

    return env


def run_command(cmd: list, env: dict = None, cwd: Path = None) -> bool:
    """Run a command and return success status."""
    print(f"\n>>> Running: {' '.join(cmd)}")
    try:
        result = subprocess.run(
            cmd,
            env=env or os.environ,
            cwd=cwd or PROJECT_DIR,
            check=True,
        )
        return True
    except subprocess.CalledProcessError as e:
        print(f"Error: Command failed with exit code {e.returncode}")
        return False


def check_sccache_available() -> bool:
    """Check if sccache is available and running."""
    if shutil.which("sccache") is None:
        return False
    # Try to start sccache server if not running
    try:
        subprocess.run(["sccache", "--start-server"], capture_output=True, timeout=5)
    except (subprocess.SubprocessError, subprocess.TimeoutExpired, OSError):
        pass
    return True


def check_prerequisites() -> bool:
    """Check that required tools are available."""
    print("Checking prerequisites...")

    tools = ["cargo", "clang", "lld-link", "llvm-lib", "npm"]
    missing = []

    for tool in tools:
        if shutil.which(tool) is None:
            missing.append(tool)

    if missing:
        print(f"Error: Missing required tools: {', '.join(missing)}")
        return False

    # Check xwin cache exists
    if not XWIN_CACHE.exists():
        print(f"Error: xwin cache not found at {XWIN_CACHE}")
        print("Run 'cargo xwin build' once to download Windows SDK files")
        return False

    print("All prerequisites met.")
    return True


def build_frontend() -> bool:
    """Build the frontend (TypeScript/React) using npm."""
    print(f"\n{'='*60}")
    print("Building Frontend")
    print(f"{'='*60}")

    # Run npm run build
    cmd = ["npm", "run", "build"]
    return run_command(cmd, cwd=PROJECT_DIR)


def generate_windows_ai_bindings() -> bool:
    """Generate Rust bindings for Windows AI APIs using windows-bindgen."""
    print(f"\n{'='*60}")
    print("Generating Windows AI Bindings")
    print(f"{'='*60}")

    winmd_dir = SRC_TAURI / ".windows" / "winmd"
    bindings_file = SRC_TAURI / "src" / "windows_ai_bindings.rs"

    # Check if winmd files exist
    ai_text_winmd = winmd_dir / "Microsoft.Windows.AI.Text.winmd"
    if not ai_text_winmd.exists():
        print(f"Note: Windows AI winmd files not found at {winmd_dir}")
        print("Skipping Windows AI bindings generation")
        return True  # Not a failure, just skip

    # Create a temporary Rust project to run bindgen
    bindgen_dir = Path("/tmp/wfdiag_bindgen")
    bindgen_dir.mkdir(parents=True, exist_ok=True)

    # Also need Windows.winmd for Foundation types
    # Find system Windows.winmd
    windows_winmd = Path("/mnt/c/Program Files (x86)/Windows Kits/10/UnionMetadata/10.0.26100.0/Windows.winmd")
    if not windows_winmd.exists():
        # Try to find any Windows.winmd
        import glob
        winmd_matches = glob.glob("/mnt/c/Program Files (x86)/Windows Kits/10/UnionMetadata/*/Windows.winmd")
        if winmd_matches:
            windows_winmd = Path(winmd_matches[0])
        else:
            print("Warning: Windows.winmd not found, bindings may be incomplete")
            windows_winmd = None

    # Write Cargo.toml
    cargo_toml = bindgen_dir / "Cargo.toml"
    cargo_toml.write_text('''[package]
name = "bindgen_runner"
version = "0.1.0"
edition = "2021"

[dependencies]
windows-bindgen = "0.65"
''')

    # Build the list of input winmd files - include all AI-related winmd files
    import glob as glob_module
    input_files = list(glob_module.glob(str(winmd_dir / "Microsoft.Windows.AI*.winmd")))
    if windows_winmd:
        input_files.append(str(windows_winmd))

    # Join with ", " for the Rust array
    input_list = ",\n        ".join(f'"{f}"' for f in input_files)

    # Write main.rs - filter for Text namespace AND base AI namespace (for AIFeatureReadyState)
    (bindgen_dir / "src").mkdir(exist_ok=True)
    main_rs = bindgen_dir / "src" / "main.rs"
    main_rs.write_text(f'''fn main() {{
    let _ = windows_bindgen::bindgen([
        "--in",
        {input_list},
        "--out",
        "{bindings_file}",
        "--filter",
        "Microsoft.Windows.AI.Text",
        "--filter",
        "Microsoft.Windows.AI.AIFeatureReadyState",
        "--filter",
        "Microsoft.Windows.AI.AIFeatureReadyResult",
        "--filter",
        "Microsoft.Windows.AI.AIFeatureReadyResultState",
        "--flat",
    ]);

    println!("Bindings generated to {bindings_file}");
}}
''')

    # Run cargo
    cmd = ["cargo", "run"]
    result = run_command(cmd, cwd=bindgen_dir)

    if result and bindings_file.exists():
        print(f"Successfully generated Windows AI bindings: {bindings_file}")
        return True
    else:
        print("Warning: Windows AI bindings generation may have failed")
        return True  # Don't fail the build


def ensure_targets() -> bool:
    """Ensure Rust targets are installed."""
    print("\nEnsuring Rust targets are installed...")

    for name, target in TARGETS.items():
        cmd = ["rustup", "target", "add", target["triple"]]
        if not run_command(cmd):
            print(f"Failed to add target {target['triple']}")
            return False

    return True


def get_build_output_path(target_name: str, release: bool = True) -> Path:
    """Get the path to the built executable in the target directory."""
    target = TARGETS[target_name]
    triple = target["triple"]
    profile = "release" if release else "debug"
    return TARGET_DIR / triple / profile / "wfdiag-tauri.exe"


def get_final_output_path(target_name: str) -> Path:
    """Get the final output path on Windows drive."""
    return OUTPUT_DIR / f"wfdiag-{target_name}.exe"


def copy_to_output(target_name: str, release: bool = True) -> Path | None:
    """Copy the built executable to the output directory."""
    build_path = get_build_output_path(target_name, release)
    final_path = get_final_output_path(target_name)

    if not build_path.exists():
        print(f"Error: Built executable not found at {build_path}")
        return None

    # Ensure output directory exists
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # Copy the file
    print(f"\n>>> Copying {build_path} -> {final_path}")
    shutil.copy2(build_path, final_path)

    return final_path


def build_target(target_name: str, release: bool = True, jobs: int = None, use_sccache: bool = False) -> bool:
    """Build for a specific target.

    Args:
        target_name: The target architecture name (x64, arm64)
        release: Whether to build in release mode
        jobs: Number of parallel jobs (defaults to CPU count)
        use_sccache: Whether to use sccache for compilation caching
    """
    target = TARGETS[target_name]
    triple = target["triple"]

    # Use all CPU cores by default
    num_jobs = jobs if jobs is not None else CPU_COUNT

    print(f"\n{'='*60}")
    print(f"Building for {target_name} ({triple})")
    print(f"Using {num_jobs} parallel jobs")
    if use_sccache:
        print("Using sccache for compilation caching")
    print(f"{'='*60}")

    env = get_env_for_target(target_name)

    # Set CARGO_BUILD_JOBS to use all available cores
    env["CARGO_BUILD_JOBS"] = str(num_jobs)

    # Enable sccache if requested
    if use_sccache:
        env["RUSTC_WRAPPER"] = "sccache"
        # Set sccache to use more cache slots for parallel builds
        env["SCCACHE_IDLE_TIMEOUT"] = "0"  # Don't timeout

    # Set RUSTFLAGS for parallel code generation
    existing_rustflags = env.get("RUSTFLAGS", "")
    # Note: codegen-units is set to 1 in Cargo.toml for release builds (for LTO)
    # so we don't override it here for release, only for debug builds
    if not release:
        env["RUSTFLAGS"] = f"{existing_rustflags} -C codegen-units={num_jobs}"

    cmd = ["cargo", "build", "--target", triple, "-j", str(num_jobs)]
    if release:
        cmd.append("--release")

    return run_command(cmd, env=env, cwd=SRC_TAURI)


def check_target(target_name: str) -> bool:
    """Check (compile without linking) for a specific target."""
    target = TARGETS[target_name]
    triple = target["triple"]

    print(f"\n{'='*60}")
    print(f"Checking {target_name} ({triple})")
    print(f"{'='*60}")

    env = get_env_for_target(target_name)

    cmd = ["cargo", "check", "--target", triple]

    return run_command(cmd, env=env, cwd=SRC_TAURI)


def get_system_memory_gb() -> float:
    """Get total system memory in GB."""
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    # Convert from kB to GB
                    kb = int(line.split()[1])
                    return kb / (1024 * 1024)
    except (OSError, IOError, ValueError, IndexError):
        pass
    return 0


def get_version() -> str:
    """Get version from tauri.conf.json."""
    conf_path = SRC_TAURI / "tauri.conf.json"
    with open(conf_path) as f:
        conf = json.load(f)
    return conf.get("version", "0.0.0")


def wslpath(path: Path) -> str:
    """Convert Linux path to Windows path using wslpath."""
    result = subprocess.run(
        ["wslpath", "-w", str(path)],
        capture_output=True,
        text=True,
        check=True
    )
    return result.stdout.strip()


def create_appx_manifest(target_name: str, version: str) -> str:
    """Generate AppxManifest.xml content for a target."""
    arch = target_name
    return f'''<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
         xmlns:systemai="http://schemas.microsoft.com/appx/manifest/systemai/windows10"
         IgnorableNamespaces="uap rescap systemai">
  <Identity Name="32827MikeFara.WindowsForumDiagnostics"
            Publisher="{PUBLISHER}"
            Version="{version}.0"
            ProcessorArchitecture="{arch}" />
  <Properties>
    <DisplayName>WindowsForum Diagnostics</DisplayName>
    <PublisherDisplayName>Mike Fara</PublisherDisplayName>
    <Logo>Logo.png</Logo>
  </Properties>
  <Dependencies>
    <!-- Both Universal and Desktop are required for Windows AI APIs on build 26200+ -->
    <TargetDeviceFamily Name="Windows.Universal" MinVersion="10.0.26100.0" MaxVersionTested="10.0.26226.0" />
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26226.0" />
    <!-- Windows App SDK 1.8 framework package dependency for Phi Silica/AI features -->
    <PackageDependency Name="Microsoft.WindowsAppRuntime.1.8"
                       MinVersion="8000.675.1142.0"
                       Publisher="CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Applications>
    <Application Id="App" Executable="{PROJECT_NAME}.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="WindowsForum Diagnostics"
                          Description="System diagnostics tool for Windows"
                          BackgroundColor="transparent"
                          Square150x150Logo="Square150x150Logo.png"
                          Square44x44Logo="Square44x44Logo.png">
        <uap:DefaultTile Wide310x150Logo="Wide310x150Logo.png" />
      </uap:VisualElements>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
    <systemai:Capability Name="systemAIModels"/>
  </Capabilities>
</Package>
'''


def build_msix(version: str) -> bool:
    """Build MSIX packages and bundle for both architectures."""
    print(f"\n{'='*60}")
    print("Building MSIX Packages")
    print(f"{'='*60}")

    # Check Windows SDK exists
    makeappx = SDK_BIN / "MakeAppx.exe"
    if not makeappx.exists():
        print(f"Error: MakeAppx.exe not found at {makeappx}")
        print("Please install Windows SDK 10.0.26100.0 or update SDK_BIN path")
        return False

    # Use Windows-accessible temp directory for MSIX building
    # MakeAppx.exe has permission issues with WSL paths
    win_msix_dir = OUTPUT_DIR / "msix-build"

    # Clean and create MSIX directories
    if win_msix_dir.exists():
        shutil.rmtree(win_msix_dir)

    x64_dir = win_msix_dir / "x64"
    arm64_dir = win_msix_dir / "arm64"
    bundle_dir = win_msix_dir / "bundle"

    x64_dir.mkdir(parents=True)
    arm64_dir.mkdir(parents=True)
    bundle_dir.mkdir(parents=True)

    # Icons directory
    icons_dir = SRC_TAURI / "icons"

    for target_name in ["x64", "arm64"]:
        target = TARGETS[target_name]
        triple = target["triple"]
        target_dir = x64_dir if target_name == "x64" else arm64_dir

        print(f"\nPreparing {target_name} package...")

        # Copy executable
        exe_src = TARGET_DIR / triple / "release" / "wfdiag-tauri.exe"
        exe_dst = target_dir / f"{PROJECT_NAME}.exe"
        if not exe_src.exists():
            print(f"Error: Built executable not found at {exe_src}")
            print("Run 'build-all' first to build the executables")
            return False
        shutil.copy2(exe_src, exe_dst)

        # Copy dist folder
        dist_src = PROJECT_DIR / "dist"
        if dist_src.exists():
            shutil.copytree(dist_src, target_dir / "dist")

        # Copy icons
        shutil.copy2(icons_dir / "icon.png", target_dir / "Logo.png")
        shutil.copy2(icons_dir / "192x192.png", target_dir / "Square150x150Logo.png")
        shutil.copy2(icons_dir / "44x44.png", target_dir / "Square44x44Logo.png")
        shutil.copy2(icons_dir / "Wide310x150Logo.png", target_dir / "Wide310x150Logo.png")

        # Copy Windows App SDK AI DLLs if available
        ai_sdk_dir = SRC_TAURI / "resources" / "ai-sdk" / target_name
        if ai_sdk_dir.exists():
            print(f"  Copying Windows App SDK AI DLLs for {target_name}...")
            for dll in ai_sdk_dir.glob("*.dll"):
                shutil.copy2(dll, target_dir / dll.name)
        else:
            print(f"  Note: No AI SDK DLLs found for {target_name} at {ai_sdk_dir}")

        # Create AppxManifest.xml
        manifest_content = create_appx_manifest(target_name, version)
        manifest_path = target_dir / "AppxManifest.xml"
        manifest_path.write_text(manifest_content, encoding="utf-8")

    # Build and copy PhiSilicaHelper for each target
    helper_src = SRC_TAURI / "resources" / "phi-silica-helper"
    if (helper_src / "PhiSilicaHelper.csproj").exists():
        print("\nBuilding PhiSilicaHelper for both architectures...")
        for target_name in ["x64", "arm64"]:
            target_dir = x64_dir if target_name == "x64" else arm64_dir
            rid = "win-x64" if target_name == "x64" else "win-arm64"

            # Build using powershell.exe to call dotnet on Windows
            helper_win_path = wslpath(helper_src)
            publish_dir = helper_src / "bin" / "Release" / "net8.0-windows10.0.22621.0" / rid / "publish"

            print(f"\n  Building PhiSilicaHelper for {target_name}...")
            ps_cmd = f'cd "{helper_win_path}"; dotnet publish -c Release -r {rid} --self-contained false -p:PublishSingleFile=false'
            cmd = ["powershell.exe", "-Command", ps_cmd]
            if run_command(cmd):
                # Copy published files
                if publish_dir.exists():
                    helper_target_dir = target_dir / "phi-silica-helper"
                    helper_target_dir.mkdir(exist_ok=True)
                    for f in publish_dir.iterdir():
                        if f.is_file():
                            shutil.copy2(f, helper_target_dir / f.name)
                    print(f"    Copied helper to {target_name} package")
                else:
                    print(f"    Warning: Helper publish dir not found at {publish_dir}")
            else:
                print(f"    Warning: Failed to build PhiSilicaHelper for {target_name}")
    else:
        print("\nNote: PhiSilicaHelper not found, skipping helper build")

    # Create MSIX packages using MakeAppx.exe
    print("\nCreating MSIX packages...")

    for target_name in ["x64", "arm64"]:
        target_dir = x64_dir if target_name == "x64" else arm64_dir
        msix_path = bundle_dir / f"{PROJECT_NAME}_{version}_{target_name}.msix"

        cmd = [
            str(makeappx),
            "pack",
            "/d", wslpath(target_dir),
            "/p", wslpath(msix_path),
            "/o"
        ]
        print(f"\n>>> Creating {target_name} MSIX...")
        if not run_command(cmd):
            print(f"Failed to create {target_name} MSIX")
            return False

    # Create MSIX bundle
    print("\nCreating MSIX bundle...")
    bundle_path = win_msix_dir / f"{PROJECT_NAME}_{version}.msixbundle"
    cmd = [
        str(makeappx),
        "bundle",
        "/d", wslpath(bundle_dir),
        "/p", wslpath(bundle_path),
        "/o"
    ]
    if not run_command(cmd):
        print("Failed to create MSIX bundle")
        return False

    print(f"\n{'='*60}")
    print("MSIX Build Complete")
    print(f"{'='*60}")
    print(f"  x64 MSIX:    {bundle_dir / f'{PROJECT_NAME}_{version}_x64.msix'}")
    print(f"  ARM64 MSIX:  {bundle_dir / f'{PROJECT_NAME}_{version}_arm64.msix'}")
    print(f"  Bundle:      {bundle_path}")

    return True


def sign_msix(version: str) -> bool:
    """Sign the MSIX bundle using signtool and self-signed certificate.

    This function calls a PowerShell script on Windows to:
    1. Create self-signed certificate if it doesn't exist
    2. Install certificate to Trusted Root (requires admin on first run)
    3. Sign the MSIX bundle
    """
    print(f"\n{'='*60}")
    print("Signing MSIX Bundle")
    print(f"{'='*60}")

    # Check signtool exists
    signtool = SDK_BIN / "signtool.exe"
    if not signtool.exists():
        print(f"Error: signtool.exe not found at {signtool}")
        return False

    # Bundle path
    win_msix_dir = OUTPUT_DIR / "msix-build"
    bundle_path = win_msix_dir / f"{PROJECT_NAME}_{version}.msixbundle"

    if not bundle_path.exists():
        print(f"Error: Bundle not found at {bundle_path}")
        return False

    # Check if certificate exists
    if not CERT_PATH.exists():
        print(f"\nCertificate not found at {CERT_PATH}")
        print("Creating self-signed certificate via PowerShell...")

        # Create certificate using PowerShell
        ps_script = f'''
$ErrorActionPreference = "Stop"
$Publisher = "{PUBLISHER}"
$CertPath = "{wslpath(CERT_PATH)}"
$CertPassword = "{CERT_PASSWORD}"

# Create the certificate
$cert = New-SelfSignedCertificate `
    -Type Custom `
    -Subject $Publisher `
    -KeyUsage DigitalSignature `
    -FriendlyName "WindowsForum Diagnostics Self-Sign" `
    -CertStoreLocation "Cert:\\CurrentUser\\My" `
    -TextExtension @("2.5.29.37={{text}}1.3.6.1.5.5.7.3.3", "2.5.29.19={{text}}") `
    -NotAfter (Get-Date).AddYears(5)

Write-Host "Certificate created: $($cert.Thumbprint)"

# Export to PFX
$securePassword = ConvertTo-SecureString -String $CertPassword -Force -AsPlainText
Export-PfxCertificate -Cert $cert -FilePath $CertPath -Password $securePassword | Out-Null
Write-Host "Exported to: $CertPath"

# Try to install to trusted root (may fail without admin)
try {{
    $cerPath = [System.IO.Path]::ChangeExtension($CertPath, ".cer")
    Export-Certificate -Cert "Cert:\\CurrentUser\\My\\$($cert.Thumbprint)" -FilePath $cerPath | Out-Null
    Import-Certificate -FilePath $cerPath -CertStoreLocation "Cert:\\LocalMachine\\Root" | Out-Null
    Remove-Item $cerPath -ErrorAction SilentlyContinue
    Write-Host "Certificate added to Trusted Root"
}} catch {{
    Write-Host "Note: Run as admin to add certificate to Trusted Root"
}}
'''

        cmd = ["powershell.exe", "-Command", ps_script]
        if not run_command(cmd):
            print("Failed to create certificate")
            print("\nTo create manually, run in Windows PowerShell as Admin:")
            print(f'  powershell.exe -File C:\\code\\sign-msix.ps1')
            return False
    else:
        print(f"Using existing certificate: {CERT_PATH}")

    # Sign the bundle
    print(f"\nSigning bundle with signtool...")
    cmd = [
        str(signtool),
        "sign",
        "/fd", "SHA256",
        "/a",
        "/f", wslpath(CERT_PATH),
        "/p", CERT_PASSWORD,
        wslpath(bundle_path)
    ]

    if not run_command(cmd):
        print("Failed to sign bundle")
        print("\nIf you see 'Access denied', ensure certificate is installed to Trusted Root.")
        print("Run in Windows PowerShell as Admin:")
        print(f'  powershell.exe -File C:\\code\\sign-msix.ps1')
        return False

    # Verify signature
    print(f"\nVerifying signature...")
    cmd = [
        str(signtool),
        "verify",
        "/pa",
        wslpath(bundle_path)
    ]
    run_command(cmd)  # Don't fail on verify - self-signed may not fully verify

    print(f"\n{'='*60}")
    print("Signing Complete")
    print(f"{'='*60}")
    print(f"  Signed bundle: {bundle_path}")
    print("")
    print("To install the app, run in Windows:")
    print(f'  Add-AppxPackage -Path "{wslpath(bundle_path)}"')

    return True


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Cross-compile WindowsForum Diagnostics for Windows"
    )
    parser.add_argument(
        "action",
        choices=["check", "build", "build-all", "build-msix"],
        help="Action to perform: check, build, build-all, or build-msix"
    )
    parser.add_argument(
        "--target",
        choices=["x64", "arm64"],
        default="x64",
        help="Target architecture (default: x64)"
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="Build debug version instead of release"
    )
    parser.add_argument(
        "-j", "--jobs",
        type=int,
        default=None,
        help=f"Number of parallel jobs (default: {CPU_COUNT} - all CPUs)"
    )
    parser.add_argument(
        "--skip-frontend",
        action="store_true",
        help="Skip frontend build (use existing dist/)"
    )
    parser.add_argument(
        "--build-msix",
        action="store_true",
        help="Also build MSIX bundle after build-all"
    )
    parser.add_argument(
        "--sign",
        action="store_true",
        help="Sign the MSIX bundle with self-signed certificate"
    )
    parser.add_argument(
        "--no-sccache",
        action="store_true",
        help="Disable sccache (enabled by default if available)"
    )

    args = parser.parse_args()
    release = not args.debug
    jobs = args.jobs
    skip_frontend = args.skip_frontend

    # Determine sccache usage - enabled by default if available
    if args.no_sccache:
        use_sccache = False
    else:
        use_sccache = check_sccache_available()

    # Display system info
    mem_gb = get_system_memory_gb()
    print(f"\n{'='*60}")
    print("System Information")
    print(f"{'='*60}")
    print(f"  CPU cores: {CPU_COUNT}")
    if mem_gb > 0:
        print(f"  Total RAM: {mem_gb:.1f} GB")
    print(f"  Parallel jobs: {jobs if jobs else CPU_COUNT}")
    print(f"  Build mode: {'Debug' if args.debug else 'Release'}")
    print(f"  sccache: {'enabled' if use_sccache else 'disabled'}")

    # Check prerequisites
    if not check_prerequisites():
        sys.exit(1)

    if args.action == "check":
        # Just check compilation
        if not ensure_targets():
            sys.exit(1)
        if not check_target(args.target):
            sys.exit(1)
        print(f"\n✓ {args.target} check passed!")

    elif args.action == "build":
        # Build frontend first (unless skipped)
        if not skip_frontend:
            if not build_frontend():
                print("Frontend build failed!")
                sys.exit(1)
        else:
            print("\nSkipping frontend build (--skip-frontend)")

        # Build single target
        if not ensure_targets():
            sys.exit(1)
        if not build_target(args.target, release, jobs, use_sccache):
            sys.exit(1)

        # Copy to output directory
        output = copy_to_output(args.target, release)
        if output:
            print(f"\n✓ Build successful!")
            print(f"  Output: {output}")
        else:
            build_path = get_build_output_path(args.target, release)
            print(f"\n✗ Build completed but output not found at {build_path}")
            sys.exit(1)

    elif args.action == "build-all":
        # Build frontend first (only once for all targets, unless skipped)
        if not skip_frontend:
            if not build_frontend():
                print("Frontend build failed!")
                sys.exit(1)
        else:
            print("\nSkipping frontend build (--skip-frontend)")

        # Generate Windows AI bindings (optional - won't fail build if it fails)
        generate_windows_ai_bindings()

        # Build both targets
        if not ensure_targets():
            sys.exit(1)

        results = {}
        for target_name in TARGETS:
            results[target_name] = build_target(target_name, release, jobs, use_sccache)

        print(f"\n{'='*60}")
        print("Build Summary")
        print(f"{'='*60}")

        all_success = True
        for target_name, success in results.items():
            if success:
                output = copy_to_output(target_name, release)
                if output:
                    print(f"  ✓ {target_name}: {output}")
                else:
                    print(f"  ✗ {target_name}: FAILED to copy")
                    all_success = False
            else:
                print(f"  ✗ {target_name}: FAILED to build")
                all_success = False

        if not all_success:
            sys.exit(1)

        # Build MSIX if requested
        if args.build_msix:
            version = get_version()
            if not build_msix(version):
                print("MSIX build failed!")
                sys.exit(1)

            # Sign if requested
            if args.sign:
                if not sign_msix(version):
                    print("MSIX signing failed!")
                    sys.exit(1)

            # Copy bundle to output directory
            win_msix_dir = OUTPUT_DIR / "msix-build"
            bundle_name = f"{PROJECT_NAME}_{version}.msixbundle"
            bundle_src = win_msix_dir / bundle_name
            bundle_dst = OUTPUT_DIR / bundle_name
            if bundle_src.exists():
                shutil.copy2(bundle_src, bundle_dst)
                print(f"\n✓ Bundle copied to: {bundle_dst}")

    elif args.action == "build-msix":
        # Build MSIX packages only (requires binaries to already exist)
        version = get_version()
        print(f"\nBuilding MSIX bundle v{version}...")

        if not build_msix(version):
            print("MSIX build failed!")
            sys.exit(1)

        # Sign if requested
        if args.sign:
            if not sign_msix(version):
                print("MSIX signing failed!")
                sys.exit(1)

        # Copy bundle to output directory
        win_msix_dir = OUTPUT_DIR / "msix-build"
        bundle_name = f"{PROJECT_NAME}_{version}.msixbundle"
        bundle_src = win_msix_dir / bundle_name
        bundle_dst = OUTPUT_DIR / bundle_name
        if bundle_src.exists():
            OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
            shutil.copy2(bundle_src, bundle_dst)
            print(f"\n✓ Bundle copied to: {bundle_dst}")

    print("\nDone!")


if __name__ == "__main__":
    main()
