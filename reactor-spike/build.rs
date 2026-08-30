use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const WEBVIEW2_PROJECTION: &str = "Microsoft.Web.WebView2.Core.dll";
const APP_VERSION_SOURCE: &str = "../version.json";

// The pinned setup crate copies these PE images from the Windows App Runtime
// MSIX. Its copy helpers deliberately ignore missing inputs, so checking only
// the two DLLs that failed during the original mixed-architecture incident can
// still let a partially staged direct-installer payload through.
const REQUIRED_RUNTIME_DLLS: &[&str] = &[
    "CoreMessagingXP.dll",
    "dcompi.dll",
    "dwmcorei.dll",
    "DwmSceneI.dll",
    "DWriteCore.dll",
    "marshal.dll",
    "Microsoft.DirectManipulation.dll",
    "Microsoft.Graphics.Imaging.dll",
    "Microsoft.InputStateManager.dll",
    "Microsoft.Internal.FrameworkUdk.dll",
    "Microsoft.UI.Composition.OSSupport.dll",
    "Microsoft.UI.dll",
    "Microsoft.UI.Input.dll",
    "Microsoft.UI.Windowing.Core.dll",
    "Microsoft.UI.Windowing.dll",
    "Microsoft.UI.Xaml.Controls.dll",
    "Microsoft.UI.Xaml.Internal.dll",
    "Microsoft.UI.Xaml.Phone.dll",
    "Microsoft.ui.xaml.dll",
    "Microsoft.ui.xaml.resources.19h1.dll",
    "Microsoft.ui.xaml.resources.common.dll",
    "Microsoft.Windows.ApplicationModel.Resources.dll",
    "Microsoft.WindowsAppRuntime.dll",
    "MRM.dll",
    "SessionHandleIPCProxyStub.dll",
    "WinUIEdit.dll",
    "wuceffectsi.dll",
];

fn main() {
    configure_app_version();

    if std::env::var_os("CARGO_FEATURE_SELF_CONTAINED").is_some() {
        // Direct-installer validation: stage the Windows App Runtime beside the
        // executable and embed Reactor's self-contained application manifest.
        require_native_windows_packaging_host();
        scope_setup_cache_to_target_arch();
        windows_reactor_setup::as_self_contained();
        verify_staged_runtime_architecture();
        remove_unused_webview_projection();
    } else {
        // Default: keep the prototype small and use the shared Windows App
        // Runtime, staging only its bootstrap DLL beside the executable.
        windows_reactor_setup::as_framework_dependent();
        if cfg!(windows) {
            // Keep deployment-mode target directories separate, but also clean
            // up an obsolete projection left by an older WFDiag build if a
            // developer accidentally reuses one.
            remove_unused_webview_projection();
        }
    }
}

fn require_native_windows_packaging_host() {
    let host = std::env::var("HOST").expect("Cargo must provide the build host triple");
    assert!(
        host.contains("-windows-"),
        "the `self-contained` Reactor feature is a packaging build and must run with native Windows Cargo; use `cargo xwin check`/`clippy` without this feature for Linux-side validation (host was {host})"
    );
}

fn configure_app_version() {
    let version_path = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR must be set for a Cargo build script"),
    )
    .join(APP_VERSION_SOURCE);
    println!("cargo:rerun-if-changed={}", version_path.display());

    let contents = std::fs::read_to_string(&version_path).unwrap_or_else(|error| {
        panic!(
            "failed to read canonical application version {}: {error}",
            version_path.display()
        )
    });
    let document: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|error| {
        panic!(
            "failed to parse canonical application version {}: {error}",
            version_path.display()
        )
    });
    let version = document
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| {
            panic!(
                "canonical application version {} has no string `version` field",
                version_path.display()
            )
        });
    assert!(
        valid_app_version(version),
        "canonical application version must use X.Y.Z numeric form, got {version:?} in {}",
        version_path.display()
    );

    // Keep the native UI tied to the repository's release source of truth.
    // The Reactor crate's own 0.0.1 version describes the migration prototype,
    // not the shipping WFDiag product shown to users.
    println!("cargo:rustc-env=WFDIAG_APP_VERSION={version}");
}

fn valid_app_version(version: &str) -> bool {
    let mut parts = version.split('.');
    (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none()
}

fn scope_setup_cache_to_target_arch() {
    let arch = target_arch();
    let (variable, base) = if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        ("LOCALAPPDATA", PathBuf::from(path))
    } else if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        ("XDG_CACHE_HOME", PathBuf::from(path))
    } else if let Some(path) = std::env::var_os("HOME") {
        ("XDG_CACHE_HOME", PathBuf::from(path).join(".cache"))
    } else {
        panic!("could not determine an architecture-scoped Reactor setup cache directory");
    };
    let scoped = base.join("wfdiag-reactor-setup").join(arch);

    // SAFETY: Cargo build scripts execute this code before Reactor setup starts
    // any work, and this build script does not create threads. The change is
    // process-local and prevents Reactor's shared `.msix_extract` directory
    // from reusing runtime files extracted for a different target architecture.
    unsafe {
        std::env::set_var(variable, scoped);
    }
}

fn verify_staged_runtime_architecture() {
    let target_dir = staged_target_dir();
    let expected = expected_pe_machine();

    for name in REQUIRED_RUNTIME_DLLS {
        let path =
            find_case_insensitive(&target_dir, name).unwrap_or_else(|| target_dir.join(name));
        let actual = pe_machine(&path);
        assert_eq!(
            actual,
            expected,
            "Reactor staged {} for PE machine 0x{actual:04X}, but target {} requires 0x{expected:04X}; remove stale setup output and rebuild",
            path.display(),
            target_arch(),
        );
    }
}

fn remove_unused_webview_projection() {
    let target_dir = staged_target_dir();
    while let Some(path) = find_case_insensitive(&target_dir, WEBVIEW2_PROJECTION) {
        std::fs::remove_file(&path).unwrap_or_else(|error| {
            panic!(
                "failed to remove unused WebView2 projection {}: {error}",
                path.display()
            )
        });
    }
    assert!(
        find_case_insensitive(&target_dir, WEBVIEW2_PROJECTION).is_none(),
        "unused WebView2 projection is still staged in {}",
        target_dir.display()
    );
}

fn find_case_insensitive(base: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).unwrap_or_else(|error| {
        panic!(
            "failed to inspect staged runtime {}: {error}",
            base.display()
        )
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to inspect an entry in staged runtime {}: {error}",
                base.display()
            )
        });
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Some(entry.path());
        }
    }
    None
}

fn staged_target_dir() -> PathBuf {
    let out_dir = PathBuf::from(
        std::env::var_os("OUT_DIR").expect("OUT_DIR must be set for a Cargo build script"),
    );
    let profile = std::env::var_os("PROFILE").expect("PROFILE must be set by Cargo");
    out_dir
        .ancestors()
        .find(|path| path.file_name() == Some(OsStr::new(&profile)))
        .map_or_else(
            || {
                panic!(
                    "could not resolve the Cargo target directory from {}",
                    out_dir.display()
                )
            },
            Path::to_path_buf,
        )
}

fn target_arch() -> String {
    std::env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH must be set by Cargo")
}

fn expected_pe_machine() -> u16 {
    match target_arch().as_str() {
        "x86" => 0x014c,
        "x86_64" => 0x8664,
        "aarch64" => 0xaa64,
        arch => panic!("unsupported Windows PE target architecture: {arch}"),
    }
}

fn pe_machine(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        panic!("failed to read staged runtime {}: {error}", path.display())
    });
    assert_eq!(
        bytes.get(..2),
        Some(b"MZ".as_slice()),
        "staged runtime is not a PE image: {}",
        path.display()
    );
    let pe_offset = u32::from_le_bytes(
        bytes
            .get(0x3c..0x40)
            .and_then(|value| value.try_into().ok())
            .unwrap_or_else(|| panic!("truncated DOS header in {}", path.display())),
    ) as usize;
    let signature_end = pe_offset
        .checked_add(4)
        .unwrap_or_else(|| panic!("invalid PE offset in {}", path.display()));
    assert_eq!(
        bytes.get(pe_offset..signature_end),
        Some(b"PE\0\0".as_slice()),
        "invalid PE header in {}",
        path.display()
    );
    let machine_start = signature_end;
    let machine_end = machine_start
        .checked_add(2)
        .unwrap_or_else(|| panic!("invalid PE machine offset in {}", path.display()));
    u16::from_le_bytes(
        bytes
            .get(machine_start..machine_end)
            .and_then(|value| value.try_into().ok())
            .unwrap_or_else(|| panic!("truncated PE header in {}", path.display())),
    )
}
