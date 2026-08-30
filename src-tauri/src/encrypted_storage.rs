//! Compatibility re-export for the shared encrypted history store.
//!
//! Keeping this module name avoids changing the shipping Tauri storage API,
//! while the DPAPI v2 implementation now has one owner that native WinUI can
//! use without linking Tauri, Wry, WebView2, or Reactor.

pub use wfdiag_native_history::EncryptedStorage;
