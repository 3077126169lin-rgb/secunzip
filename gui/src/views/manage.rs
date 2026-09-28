use crate::api;
use crate::app::{short_id, SecUnzipApp};
use crate::theme;
use eframe::egui;

impl SecUnzipApp {
    pub(crate) fn show_manage(&mut self, ui: &mut egui::Ui) {
        // 首次进入管理页时，用本机记住的管理口令预填「授权口令」（与命令行共用
        // config.json 的 admin_password），省去重复输入。只填一次：之后用户手动
        // 清空该框不会被下一帧再填回来。审批口令刻意不预填，避免默认覆盖掉
        // 申请人在申请时自己设定的口令（那一栏留空表示沿用）。
        if !self.admin_prefill_done {
            self.admin_prefill_done = true;
            if let Some(pwd) = self.admin_password.clone() {
                if self.grant_password.is_empty() {
                    self.grant_password = pwd;
                }
            }
        }

        ui.horizontal(|ui| {
            crate::icons::wrench(ui, 18.0, theme::ACCENT);
            ui.label(
                egui::RichText::new("管理此文件")
                    .size(18.0)
                    .strong()
                    .color(theme::TEXT),
            );
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!(
                "应用ID {} · 你是此文件的管理员",
                short_id(&self.app_id)
            ))
            .size(13.0)
            .color(theme::TEXT_DIM),
        );
        ui.add_space(16.0);

        theme::card(ui, theme::CARD, |ui| {
            ui.label(
                egui::RichText::new("授权用户")
                    .size(15.0)
                    .strong()
                    .color(theme::TEXT),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("用户ID")
                        .size(13.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add(egui::TextEdit::singleline(&mut self.grant_user).desired_width(200.0));
                ui.label(
                    egui::RichText::new("有效期")
                        .size(13.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.grant_expires)
                        .desired_width(140.0)
                        .hint_text("7d 或 20261231"),
                );
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("口令")
                        .size(13.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.grant_password)
                        .password(true)
                        .desired_width(200.0)
                        .hint_text("必填"),
                );
                ui.label(
                    egui::RichText::new("对方取密钥时需输入此口令，请另行告知")
                        .size(12.0)
                        .color(theme::TEXT_DIM),
                );
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        [120.0, 40.0],
                        egui::Button::new(
                            egui::RichText::new("授权")
                                .size(14.0)
                                .color(egui::Color32::WHITE),
                        )
                        .fill(theme::ACCENT),
                    )
                    .clicked()
                {
                    self.do_grant();
                }
                if ui
                    .add_sized(
                        [120.0, 40.0],
                        egui::Button::new(egui::RichText::new("吊销").size(14.0))
                            .fill(theme::INPUT_BG),
                    )
                    .clicked()
                {
                    self.do_revoke();
                }
            });
        });

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("待审批申请")
                    .size(15.0)
                    .strong()
                    .color(theme::TEXT),
            );
            if ui.small_button("刷新").clicked() {
                self.refresh_requests();
            }
        });
        ui.add_space(8.0);
        // 审批口令：留空则请求里不带该字段，服务端沿用申请人自己设定的口令
        egui::Frame::none()
            .fill(theme::INPUT_BG)
            .rounding(egui::Rounding::same(10.0))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("审批口令")
                            .size(13.0)
                            .color(theme::TEXT_DIM),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.approve_password)
                            .password(true)
                            .desired_width(200.0)
                            .hint_text("选填"),
                    );
                    ui.label(
                        egui::RichText::new("留空表示沿用申请时设定的口令")
                            .size(12.0)
                            .color(theme::TEXT_DIM),
                    );
                });
            });
        ui.add_space(10.0);

        if self.pending_requests.is_empty() {
            theme::card(ui, theme::CARD, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    crate::icons::empty(ui, 30.0, theme::TEXT_DIM);
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("暂无待审批申请")
                            .size(14.0)
                            .color(theme::TEXT_DIM),
                    );
                    ui.add_space(20.0);
                });
            });
        } else {
            let mut action: Option<(String, bool)> = None;
            egui::ScrollArea::vertical()
                .id_salt("req_list")
                .show(ui, |ui| {
                    for req in &self.pending_requests {
                        theme::card(ui, theme::CARD, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(req.user_id.to_string())
                                        .size(15.0)
                                        .strong()
                                        .color(theme::TEXT),
                                );
                                if let Some(days) = req.need_days {
                                    theme::badge(ui, &format!("需要{}天", days), theme::ACCENT);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("拒绝").clicked() {
                                            action = Some((req.user_id.clone(), false));
                                        }
                                        if ui
                                            .add_sized(
                                                [80.0, 30.0],
                                                egui::Button::new(
                                                    egui::RichText::new("通过")
                                                        .size(13.0)
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .fill(theme::ACCENT),
                                            )
                                            .clicked()
                                        {
                                            action = Some((req.user_id.clone(), true));
                                        }
                                    },
                                );
                            });
                            if let Some(msg) = &req.message {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(format!("说明: {}", msg))
                                        .size(13.0)
                                        .color(theme::TEXT_DIM),
                                );
                            }
                            ui.label(
                                egui::RichText::new(format!("时间: {}", req.created_at))
                                    .size(12.0)
                                    .color(theme::TEXT_DIM),
                            );
                        });
                        ui.add_space(8.0);
                    }
                });

            if let Some((user_id, approved)) = action {
                let rt = tokio::runtime::Runtime::new().unwrap();
                // 留空 = 不覆盖，沿用申请人设定的口令（字段整体省略）
                let approve_pwd = if self.approve_password.is_empty() {
                    None
                } else {
                    Some(self.approve_password.clone())
                };
                let result = rt.block_on(async {
                    if approved {
                        api::approve_request(
                            &self.server_url,
                            &self.app_id,
                            &self.secret,
                            &user_id,
                            None,
                            approve_pwd.as_deref(),
                        )
                        .await
                    } else {
                        api::deny_request(&self.server_url, &self.app_id, &self.secret, &user_id)
                            .await
                    }
                });
                match result {
                    Ok(msg) => {
                        self.show_status(&msg.to_string(), false);
                        // 审批成功且确实下发过口令：记下管理口令，下次不必重输
                        if let Some(pwd) = approve_pwd {
                            self.remember_admin_password(&pwd);
                        }
                        self.refresh_requests();
                    }
                    Err(e) => self.show_status(&e.to_string(), true),
                }
            }
        }
    }

    pub(crate) fn do_grant(&mut self) {
        if self.grant_user.is_empty() {
            self.show_status("请输入用户ID", true);
            return;
        }
        if self.grant_password.is_empty() {
            self.show_status("请输入口令（对方取密钥时需要它）", true);
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let exp = if self.grant_expires.is_empty() {
                None
            } else {
                Some(self.grant_expires.clone())
            };
            api::grant_user(
                &self.server_url,
                &self.app_id,
                &self.secret,
                &self.grant_user,
                exp,
                &self.grant_password,
            )
            .await
        });
        match result {
            Ok(msg) => {
                self.show_status(&msg.to_string(), false);
                self.grant_user.clear();
                // 授权成功：记下管理口令（与命令行同一字段），继续给别的用户授权时不必重输。
                // 口令框保留在界面上（掩码显示），也已在 config.json 里。
                let pwd = self.grant_password.clone();
                self.remember_admin_password(&pwd);
            }
            Err(e) => self.show_status(&e.to_string(), true),
        }
    }

    pub(crate) fn do_revoke(&mut self) {
        if self.grant_user.is_empty() {
            self.show_status("请输入用户ID", true);
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            api::revoke_user(
                &self.server_url,
                &self.app_id,
                &self.secret,
                &self.grant_user,
            )
            .await
        });
        match result {
            Ok(msg) => {
                self.show_status(&msg.to_string(), false);
                self.grant_user.clear();
            }
            Err(e) => self.show_status(&e.to_string(), true),
        }
    }
}
