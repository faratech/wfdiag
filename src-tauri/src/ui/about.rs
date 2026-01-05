use eframe::egui::{self, RichText};
use crate::WfDiagApp;

pub fn show(app: &mut WfDiagApp, ctx: &egui::Context) {
    let mut open = app.show_about;
    let mut close_clicked = false;

    egui::Window::new("About WFDiag")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .min_width(350.0)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);

                ui.label(RichText::new("🔧").size(48.0));

                ui.add_space(8.0);

                ui.label(RichText::new("WFDiag").size(24.0).strong());
                ui.label(RichText::new("Windows System Diagnostics").size(13.0).weak());

                ui.add_space(8.0);

                ui.label(RichText::new("Version 2.1.8").size(12.0));

                ui.add_space(16.0);

                ui.separator();

                ui.add_space(8.0);

                if let Some(ref info) = app.system_info {
                    ui.label(RichText::new("System Information").size(12.0).strong());
                    ui.add_space(4.0);
                    ui.label(RichText::new(&info.os_name).size(11.0));
                    ui.label(RichText::new(&info.os_version).size(11.0).weak());
                    ui.label(RichText::new(&info.computer_name).size(11.0).weak());
                }

                ui.add_space(16.0);

                ui.label(RichText::new("Built with Rust + egui").size(11.0).weak());
                ui.hyperlink_to("WindowsForum.com", "https://windowsforum.com");

                ui.add_space(16.0);

                if ui.button("Close").clicked() {
                    close_clicked = true;
                }

                ui.add_space(8.0);
            });
        });

    app.show_about = open && !close_clicked;
}
