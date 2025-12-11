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
from pathlib import Path

# Build configuration
PROJECT_DIR = Path(__file__).parent.resolve()
SRC_TAURI = PROJECT_DIR / "src-tauri"
TARGET_DIR = SRC_TAURI / "target"
OUTPUT_DIR = Path("/mnt/c/code")  # Always output to Windows drive

# Cross-compilation settings
XWIN_CACHE = Path.home() / ".cache" / "cargo-xwin" / "xwin"

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


def build_target(target_name: str, release: bool = True, jobs: int = None) -> bool:
    """Build for a specific target.

    Args:
        target_name: The target architecture name (x64, arm64)
        release: Whether to build in release mode
        jobs: Number of parallel jobs (defaults to CPU count)
    """
    target = TARGETS[target_name]
    triple = target["triple"]

    # Use all CPU cores by default
    num_jobs = jobs if jobs is not None else CPU_COUNT

    print(f"\n{'='*60}")
    print(f"Building for {target_name} ({triple})")
    print(f"Using {num_jobs} parallel jobs")
    print(f"{'='*60}")

    env = get_env_for_target(target_name)

    # Set CARGO_BUILD_JOBS to use all available cores
    env["CARGO_BUILD_JOBS"] = str(num_jobs)

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
    except:
        pass
    return 0


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Cross-compile WindowsForum Diagnostics for Windows"
    )
    parser.add_argument(
        "action",
        choices=["check", "build", "build-all"],
        help="Action to perform"
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

    args = parser.parse_args()
    release = not args.debug
    jobs = args.jobs
    skip_frontend = args.skip_frontend

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
        if not build_target(args.target, release, jobs):
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

        # Build both targets
        if not ensure_targets():
            sys.exit(1)

        results = {}
        for target_name in TARGETS:
            results[target_name] = build_target(target_name, release, jobs)

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

    print("\nDone!")


if __name__ == "__main__":
    main()
