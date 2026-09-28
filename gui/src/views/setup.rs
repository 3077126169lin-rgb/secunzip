use crate::app::SecUnzipApp;
use crate::model::AppState;
use crate::theme;
use eframe::egui;

impl SecUnzipApp {
    pub(crate) fn show_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(((ui.available_height() - 470.0) / 2.0).max(24.0));
                    crate::icons::lock_open(ui, 52.0, theme::ACCENT);
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("欢迎使用 SecUnzip")
                            .size(28.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("受控解压与内容交付 · 首次使用先完成基本设置")
                            .size(15.0)
                            .color(theme::TEXT_DIM),
                    );
                    ui.add_space(34.0);

                    theme::card(ui, theme::CARD, |ui| {
                        ui.set_min_width(420.0);
                        ui.horizontal(|ui| {
                            crate::icons::user(ui, 13.0, theme::TEXT_DIM);
                            ui.label(
                                egui::RichText::new("个人 ID")
                                    .size(13.0)
                                    .color(theme::TEXT_DIM),
                            );
                        });
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.user_id)
                                .hint_text("手机号 / 邮箱 / 任意唯一标识")
                                .desired_width(380.0),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("无需注册，仅作授权凭据")
                                .size(12.0)
                                .color(theme::TEXT_DIM),
                        );
                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            crate::icons::globe(ui, 13.0, theme::TEXT_DIM);
                            ui.label(
                                egui::RichText::new("服务器地址")
                                    .size(13.0)
                                    .color(theme::TEXT_DIM),
                            );
                        });
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.server_url)
                                .hint_text("http://服务器IP或域名:8090")
                                .desired_width(380.0),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("填能访问到的真实服务端地址，勿用 localhost")
                                .size(12.0)
                                .color(theme::TEXT_DIM),
                        );
                    });

                    ui.add_space(28.0);
                    if theme::cta(ui, "开始使用  →").clicked() {
                        if self.user_id.trim().is_empty() {
                            self.show_status("请输入个人ID", true);
                        } else if self.server_url.trim().is_empty() {
                            self.show_status("请填写服务器地址", true);
                        } else {
                            self.user_id = self.user_id.trim().to_string();
                            self.server_url = self.server_url.trim().to_string();
                            self.save_settings();
                            self.state = AppState::Main;
                            self.show_status("设置已保存", false);
                        }
                    }
                });
            });
    }
}
