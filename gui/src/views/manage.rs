use eframe::egui;
use crate::api;
use crate::app::{short_id, SecUnzipApp};
use crate::theme;

impl SecUnzipApp {
    pub(crate) fn show_manage(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            crate::icons::wrench(ui, 18.0, theme::ACCENT);
            ui.label(egui::RichText::new("管理此文件").size(18.0).strong().color(theme::TEXT));
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("应用ID {} · 你是此文件的管理员", short_id(&self.app_id))).size(13.0).color(theme::TEXT_DIM));
        ui.add_space(16.0);

        theme::card(ui, theme::CARD, |ui| {
            ui.label(egui::RichText::new("授权用户").size(15.0).strong().color(theme::TEXT));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("用户ID").size(13.0).color(theme::TEXT_DIM));
                ui.add(egui::TextEdit::singleline(&mut self.grant_user).desired_width(200.0));
                ui.label(egui::RichText::new("有效期").size(13.0).color(theme::TEXT_DIM));
                ui.add(egui::TextEdit::singleline(&mut self.grant_expires).desired_width(140.0).hint_text("7d 或 20261231"));
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_sized([120.0, 40.0], egui::Button::new(egui::RichText::new("授权").size(14.0).color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                    self.do_grant();
                }
                if ui.add_sized([120.0, 40.0], egui::Button::new(egui::RichText::new("吊销").size(14.0)).fill(theme::INPUT_BG)).clicked() {
                    self.do_revoke();
                }
            });
        });

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("待审批申请").size(15.0).strong().color(theme::TEXT));
            if ui.small_button("刷新").clicked() {
                self.refresh_requests();
            }
        });
        ui.add_space(8.0);

        if self.pending_requests.is_empty() {
            theme::card(ui, theme::CARD, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    crate::icons::empty(ui, 30.0, theme::TEXT_DIM);
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("暂无待审批申请").size(14.0).color(theme::TEXT_DIM));
                    ui.add_space(20.0);
                });
            });
        } else {
            let mut action: Option<(String, bool)> = None;
            egui::ScrollArea::vertical().id_salt("req_list").show(ui, |ui| {
                for req in &self.pending_requests {
                    theme::card(ui, theme::CARD, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}", req.user_id)).size(15.0).strong().color(theme::TEXT));
                            if let Some(days) = req.need_days {
                                theme::badge(ui, &format!("需要{}天", days), theme::ACCENT);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("拒绝").clicked() {
                                    action = Some((req.user_id.clone(), false));
                                }
                                if ui.add_sized([80.0, 30.0], egui::Button::new(egui::RichText::new("通过").size(13.0).color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                                    action = Some((req.user_id.clone(), true));
                                }
                            });
                        });
                        if let Some(msg) = &req.message {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("说明: {}", msg)).size(13.0).color(theme::TEXT_DIM));
                        }
                        ui.label(egui::RichText::new(format!("时间: {}", req.created_at)).size(12.0).color(theme::TEXT_DIM));
                    });
                    ui.add_space(8.0);
                }
            });

            if let Some((user_id, approved)) = action {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    if approved {
                        api::approve_request(&self.server_url, &self.app_id, &self.secret, &user_id, None).await
                    } else {
                        api::deny_request(&self.server_url, &self.app_id, &self.secret, &user_id).await
                    }
                });
                match result {
                    Ok(msg) => { self.show_status(&format!("{}", msg), false); self.refresh_requests(); }
                    Err(e) => self.show_status(&format!("{}", e), true),
                }
            }
        }
    }

    pub(crate) fn do_grant(&mut self) {
        if self.grant_user.is_empty() {
            self.show_status("请输入用户ID", true);
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let exp = if self.grant_expires.is_empty() { None } else { Some(self.grant_expires.clone()) };
            api::grant_user(&self.server_url, &self.app_id, &self.secret, &self.grant_user, exp).await
        });
        match result {
            Ok(msg) => { self.show_status(&format!("{}", msg), false); self.grant_user.clear(); }
            Err(e) => self.show_status(&format!("{}", e), true),
        }
    }

    pub(crate) fn do_revoke(&mut self) {
        if self.grant_user.is_empty() {
            self.show_status("请输入用户ID", true);
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            api::revoke_user(&self.server_url, &self.app_id, &self.secret, &self.grant_user).await
        });
        match result {
            Ok(msg) => { self.show_status(&format!("{}", msg), false); self.grant_user.clear(); }
            Err(e) => self.show_status(&format!("{}", e), true),
        }
    }

}

