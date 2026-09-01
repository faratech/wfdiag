//! UI-thread WinUI focus helpers missing from Reactor's current public surface.
//!
//! The narrow generated projection lives in `winui_focus_bindings`; all app
//! policy stays here so this module can disappear once Reactor exposes focus
//! observation and arbitrary focus restoration directly.

use std::cell::RefCell;

use windows_core::{IInspectable, Interface};

use crate::winui_focus_bindings::{
    ComboBox, FocusManager, FocusState, NumberBox, PasswordBox, TextBox, UIElement,
};

thread_local! {
    static PRE_PALETTE_FOCUS: RefCell<Option<IInspectable>> = const { RefCell::new(None) };
}

/// Query the real focused XAML object on the component/UI thread.
///
/// WinUI controls are windowless, so `GetFocus`/HWND class checks cannot
/// identify a focused TextBox, PasswordBox, or ComboBox.
#[must_use]
pub fn editable_control_focused() -> bool {
    focused_element().is_some_and(|focused| {
        focused.cast::<TextBox>().is_ok()
            || focused.cast::<PasswordBox>().is_ok()
            || focused.cast::<ComboBox>().is_ok()
            || focused.cast::<NumberBox>().is_ok()
    })
}

/// Capture the exact element focused immediately before the palette opens.
pub fn capture_pre_palette_focus() {
    PRE_PALETTE_FOCUS.with(|slot| {
        let mut slot = slot.borrow_mut();
        // A rapid close/reopen can occur before the asynchronous ContentDialog
        // hide has restored focus. Keep the original non-popup target instead
        // of replacing it with the retiring palette TextBox.
        if slot.is_none() {
            *slot = focused_element();
        }
    });
}

/// Restore the captured element if it is still in a live XAML tree.
///
/// Returns false when there was no element, it was removed by the selected
/// command, or WinUI rejected focus. The caller can then use a stable fallback.
#[must_use]
pub fn restore_pre_palette_focus() -> bool {
    PRE_PALETTE_FOCUS.with(|slot| {
        let Some(element) = slot.borrow_mut().take() else {
            return false;
        };
        element
            .cast::<UIElement>()
            .ok()
            .and_then(|element| element.Focus(FocusState::Programmatic).ok())
            .unwrap_or(false)
    })
}

fn focused_element() -> Option<IInspectable> {
    FocusManager::GetFocusedElement().ok()
}
