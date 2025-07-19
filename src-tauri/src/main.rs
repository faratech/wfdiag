#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    wfdiag_tauri::run();
}