use eframe::egui;
use crate::app::SecUnzipApp;
use crate::monitor::{is_loopback, local_ip};
use crate::theme;

impl SecUnzipApp {
    pub(crate) fn show_settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("settings_scroll").show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                crate::icons::gear(ui, 44.0, theme::ACCENT);
                ui.add_space(8.0);
                ui.label(egui::RichText::new("设置").size(24.0).strong().color(theme::TEXT));
                ui.add_space(24.0);
            });

            ui.scope(|ui| {
                theme::card(ui, theme::CARD, |ui| {
                    ui.label(egui::RichText::new("账户").size(15.0).strong().color(theme::TEXT));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        crate::icons::user(ui, 13.0, theme::TEXT_DIM);
                        ui.label(egui::RichText::new("个人 ID").size(13.0).color(theme::TEXT_DIM));
                    });
                    ui.add(egui::TextEdit::singleline(&mut self.user_id).desired_width(360.0));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        crate::icons::globe(ui, 13.0, theme::TEXT_DIM);
                        ui.label(egui::RichText::new("服务器地址").size(13.0).color(theme::TEXT_DIM));
                    });
                    ui.add(egui::TextEdit::singleline(&mut self.server_url).hint_text("http://服务器IP或域名:8090").desired_width(360.0));
                });
                ui.add_space(14.0);

                theme::card(ui, theme::CARD, |ui| {
                    ui.label(egui::RichText::new("文件关联").size(15.0).strong().color(theme::TEXT));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("双击 .secunzip 用本程序打开").size(12.0).color(theme::TEXT_DIM));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("注册关联").clicked() {
                            let (msg, err) = match crate::register::register_file_type() {
                                Ok(m) => (m, false),
                                Err(e) => (e, true),
                            };
                            self.show_status(&msg, err);
                        }
                        if ui.button("取消关联").clicked() {
                            let (msg, err) = match crate::register::unregister_file_type() {
                                Ok(m) => (m, false),
                                Err(e) => (e, true),
                            };
                            self.show_status(&msg, err);
                        }
                    });
                });
                ui.add_space(14.0);

                theme::card(ui, theme::CARD, |ui| {
                    ui.label(egui::RichText::new("服务端").size(15.0).strong().color(theme::TEXT));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("一键部署 / 启动本地服务端（端口 8090）").size(12.0).color(theme::TEXT_DIM));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("一键部署/启动服务端").clicked() {
                            let (msg, err) = self.start_server();
                            self.show_status(&msg, err);
                        }
                        if ui.button("测试连接").clicked() {
                            if self.server_reachable() {
                                self.show_status(&format!("服务器在线（{}）", self.server_url), false);
                            } else {
                                self.show_status(&format!("服务器未连接（{}）：请启动服务端或核对地址", self.server_url), true);
                            }
                        }
                    });
                    // 跨机部署 / 连接向导
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("跨机部署 / 连接向导").size(13.0).strong().color(theme::TEXT));
                    if is_loopback(&self.server_url) {
                        ui.label(egui::RichText::new("当前服务器地址只对本机可见，接收方无法连接").size(12.0).color(theme::GOLD));
                    }
                    match local_ip() {
                        Some(ip) => {
                            ui.label(egui::RichText::new(format!("本机局域网地址：http://{}:8090（接收方按此连接）", ip)).size(12.0).color(theme::TEXT_DIM));
                            if ui.button("把服务器地址改为局域网地址").clicked() {
                                self.server_url = format!("http://{}:8090", ip);
                                self.save_settings();
                                self.show_status(&format!("服务器地址已设为 http://{}:8090（接收方按此连接）", ip), false);
                            }
                        }
                        None => {
                            ui.label(egui::RichText::new("未能检测到局域网 IP（可能离线，跨机请手动填可访问地址）").size(12.0).color(theme::TEXT_DIM));
                        }
                    }
                    ui.label(egui::RichText::new("提示：接收方与本机需互通，且防火墙放行 8090 端口").size(11.0).color(theme::TEXT_DIM));
                });
                ui.add_space(28.0);

                ui.vertical_centered(|ui| {
                    if theme::cta(ui, "保存设置").clicked() {
                        self.user_id = self.user_id.trim().to_string();
                        self.server_url = self.server_url.trim().to_string();
                        self.save_settings();
                        self.show_status("设置已保存", false);
                    }
                });
                ui.add_space(20.0);
            });
        });
    }
}

