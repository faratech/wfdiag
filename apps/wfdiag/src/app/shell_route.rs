//! Routing the chrome's own messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::shell_msg::ShellMsg;
use crate::app::state::Page;
use wfdiag_native_diagnostics::ScanKind;
use windows_reactor::*;

impl WfdiagShell {
    /// One chrome message: navigation, the pane, the theme, refresh.
    pub(crate) fn route_shell(&mut self, message: ShellMsg, context: &ComponentContext<Self>) {
        match message {
            ShellMsg::Navigate(Some(tag)) => {
                if let Some(page) = Page::from_tag(&tag) {
                    self.navigate_to_page(page, context);
                } else if tag == "quick-scan" {
                    self.transition_to_page(Page::Diagnostics);
                    self.begin_diagnostic_scan(ScanKind::Quick);
                } else {
                    match tag.as_str() {
                        "export" => self.request_export_to_file(),
                        "share" => self.request_share_to_windowsforum(),
                        "email" => self.request_email_report(),
                        _ => (),
                    }
                }
            }
            ShellMsg::Navigate(None) => {}
            ShellMsg::TogglePane => self.toggle_navigation_rail(),
            ShellMsg::ToggleTheme => {
                self.handle_palette_command("toggle-theme".to_string(), context);
            }
            ShellMsg::Refresh => self.refresh_current_page(context),
            ShellMsg::RestartAsAdmin => self.request_admin_relaunch(),
        }
    }
}
