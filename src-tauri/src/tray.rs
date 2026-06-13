//! System tray icon: Show/Hide, Quick Scan, Exit, and optional close-to-tray.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

/// Mirrors the `closeToTray` setting so the window-event handler never does
/// file I/O (same pattern as ai_service's in-memory preference).
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);

pub fn close_to_tray_enabled() -> bool {
    CLOSE_TO_TRAY.load(Ordering::Relaxed)
}

pub fn set_close_to_tray(enabled: bool) {
    CLOSE_TO_TRAY.store(enabled, Ordering::Relaxed);
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            show_main_window(app);
        }
    }
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_hide = MenuItem::with_id(app, "show-hide", "Show/Hide", true, None::<&str>)?;
    let quick_scan = MenuItem::with_id(app, "quick-scan", "Quick Scan", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_hide, &quick_scan, &separator, &exit])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("WindowsForum Diagnostics")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show-hide" => toggle_main_window(app),
            "quick-scan" => {
                // Surface the window so the user sees the scan start, then
                // let the frontend (App.tsx listener) drive the actual scan
                show_main_window(app);
                let _ = app.emit("tray://quick-scan", ());
            }
            // exit(0) bypasses CloseRequested, so close-to-tray cannot
            // swallow an explicit Exit from the tray menu
            "exit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}
