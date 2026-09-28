use std::sync::atomic::Ordering;
use eframe::egui;
use crate::app::SecUnzipApp;
use crate::model::Mode;
use crate::theme;

impl SecUnzipApp {
    pub(crate) fn show_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").frame(
            egui::Frame::none().fill(theme::CARD).inner_margin(egui::Margin::symmetric(18.0, 12.0)),
        ).show(ctx, |ui| {
            ui.horizontal(|ui| {
                crate::icons::lock(ui, 22.0, theme::ACCENT);
                ui.label(egui::RichText::new("SecUnzip").size(19.0).strong().color(theme::TEXT));
                ui.add_space(28.0);

                self.seg(ui, Mode::Open, "打开");
                self.seg(ui, Mode::Pack, "打包");
                self.seg(ui, Mode::Settings, "设置");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 服务器实时状态点（后台轮询，每 4s 刷新）
                    let (label, color) = match self.monitor.server_up.load(Ordering::Relaxed) {
                        1 => ("服务器在线", theme::OK),
                        2 => ("服务器离线", theme::ERR),
                        _ => ("检测中", theme::TEXT_DIM),
                    };
                    crate::icons::dot(ui, 10.0, color);
                    theme::badge(ui, label, color);
                    if self.is_admin {
                        theme::badge(ui, "管理员", theme::GOLD);
                    }
                    egui::Frame::none()
                        .fill(theme::INPUT_BG)
                        .rounding(egui::Rounding::same(18.0))
                        .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(self.user_id.to_string()).size(13.0).color(theme::TEXT_DIM));
                        });
                });
            });
        });
    }

    pub(crate) fn seg(&mut self, ui: &mut egui::Ui, mode: Mode, label: &str) {
        let active = self.mode == mode;
        let fill = if active { theme::ACCENT } else { egui::Color32::TRANSPARENT };
        let text = if active { egui::Color32::WHITE } else { theme::TEXT_DIM };
        let resp = egui::Frame::none()
            .fill(fill)
            .rounding(egui::Rounding::same(9.0))
            .inner_margin(egui::Margin::symmetric(16.0, 8.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(label).size(14.0).color(text));
            })
            .response
            .interact(egui::Sense::click());
        if resp.clicked() {
            self.mode = mode;
        }
    }

    pub(crate) fn show_status_bar(&mut self, ctx: &egui::Context) {
        if self.status_message.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("status").frame(
            egui::Frame::none().fill(theme::CARD).inner_margin(egui::Margin::symmetric(18.0, 10.0)),
        ).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let color = if self.status_is_error { theme::ERR } else { theme::OK };
                if self.status_is_error {
                    crate::icons::cross(ui, 15.0, color);
                } else {
                    crate::icons::check(ui, 15.0, color);
                }
                ui.label(egui::RichText::new(&self.status_message).size(14.0).color(color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::cross_click(ui, 14.0, theme::TEXT_DIM).clicked() {
                        self.status_message.clear();
                    }
                });
            });
        });
    }

}

