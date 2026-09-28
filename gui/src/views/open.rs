use crate::api;
use crate::app::{human_size, short_id, SecUnzipApp};
use crate::model::{Action, Mode, OpenTab, PackedEntry};
use crate::theme;
use eframe::egui;
use secunzip::runtime::{mount_vfs_to_drive, pick_free_drive, unmount as unmount_drive, VirtualFS};
use std::path::PathBuf;
use std::sync::Arc;

impl SecUnzipApp {
    pub(crate) fn show_open(&mut self, ui: &mut egui::Ui) {
        let has_file = self.current_file.is_some() || self.blackbox.is_some();

        ui.horizontal(|ui| {
            if has_file {
                if self.blackbox.is_some() {
                    crate::icons::box_(ui, 16.0, theme::ACCENT);
                    ui.label(
                        egui::RichText::new("黑盒内容")
                            .size(15.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                } else {
                    let name = self
                        .current_file
                        .as_ref()
                        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "—".into());
                    crate::icons::file(ui, 16.0, theme::TEXT_DIM);
                    ui.label(
                        egui::RichText::new(name)
                            .size(15.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                }
                ui.add_space(18.0);
            }
            if ui
                .selectable_label(self.open_tab == OpenTab::Browse, "浏览")
                .clicked()
            {
                self.open_tab = OpenTab::Browse;
            }
            if has_file
                && self.is_admin
                && ui
                    .selectable_label(self.open_tab == OpenTab::Manage, "管理此文件")
                    .clicked()
            {
                self.open_tab = OpenTab::Manage;
            }
            if ui
                .selectable_label(self.open_tab == OpenTab::Mine, "我的文件")
                .clicked()
            {
                self.open_tab = OpenTab::Mine;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if has_file {
                    if ui.button("换文件").clicked() {
                        self.current_file = None;
                        self.blackbox = None;
                        self.is_admin = false;
                        self.secret.clear();
                        self.app_id.clear();
                        self.reset_password_inputs();
                        self.vfs = None;
                        if let Some(h) = self.mount_handle.take() {
                            unmount_drive(h);
                        }
                    }
                    if !self.is_admin && ui.button("导入密钥").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("Secret", &["secret"])
                            .pick_file()
                        {
                            if let Ok(s) = std::fs::read_to_string(&p) {
                                self.secret = s.trim().to_string();
                                self.is_admin = !self.secret.is_empty();
                                let (msg, err) = if self.is_admin {
                                    ("已导入密钥（你是此文件管理员）", false)
                                } else {
                                    ("密钥文件为空", true)
                                };
                                self.show_status(msg, err);
                            }
                        }
                    }
                }
            });
        });
        ui.add_space(12.0);

        match self.open_tab {
            OpenTab::Browse => {
                if has_file {
                    self.show_browse(ui)
                } else {
                    self.show_open_picker(ui)
                }
            }
            OpenTab::Manage => self.show_manage(ui),
            OpenTab::Mine => self.show_mine(ui),
        }
    }

    pub(crate) fn show_open_picker(&mut self, ui: &mut egui::Ui) {
        // 拖放打开
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().find(|p| {
            p.extension()
                .map(|e| e.eq_ignore_ascii_case("secunzip"))
                .unwrap_or(false)
        }) {
            self.current_file = Some(path);
            self.load_file_info();
            return;
        }

        ui.vertical_centered(|ui| {
            ui.add_space(((ui.available_height() - 440.0) / 2.0).max(24.0));
            crate::icons::lock_open(ui, 56.0, theme::ACCENT);
            ui.add_space(12.0);
            ui.label(egui::RichText::new("打开加密内容").size(26.0).strong().color(theme::TEXT));
            ui.add_space(8.0);
            ui.label(egui::RichText::new("选择 .secunzip 文件解密浏览，或打包新的加密内容").size(15.0).color(theme::TEXT_DIM));
            ui.add_space(34.0);

            theme::card(ui, theme::CARD, |ui| {
                ui.set_min_width(480.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    crate::icons::upload(ui, 42.0, theme::TEXT_DIM);
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("把 .secunzip 拖到这里").size(17.0).color(theme::TEXT));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("或").size(13.0).color(theme::TEXT_DIM));
                    ui.add_space(12.0);
                    if ui.add_sized([260.0, 46.0], egui::Button::new(egui::RichText::new("浏览文件…").size(15.0).color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("SecUnzip", &["secunzip"]).pick_file() {
                            self.current_file = Some(path);
                            self.load_file_info();
                        }
                    }
                    ui.add_space(20.0);
                });
            });

            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("还没有加密内容？").size(14.0).color(theme::TEXT_DIM));
                if ui.link(egui::RichText::new("打包新文件  →").size(14.0).color(theme::ACCENT)).clicked() {
                    self.mode = Mode::Pack;
                }
                if !self.packed_list.is_empty() {
                    ui.add_space(18.0);
                    if ui.link(egui::RichText::new("查看我打包的文件  →").size(14.0).color(theme::ACCENT)).clicked() {
                        self.open_tab = OpenTab::Mine;
                    }
                }
            });
            ui.add_space(18.0);
            egui::Frame::none().fill(theme::INPUT_BG).rounding(egui::Rounding::same(10.0))
                .inner_margin(egui::Margin::same(16.0)).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        crate::icons::info(ui, 13.0, theme::TEXT_DIM);
                        ui.label(egui::RichText::new("使用流程").size(13.0).strong().color(theme::TEXT_DIM));
                    });
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("打包生成文件 → 发给对方 → 对方用个人 ID「申请临时权限」→ 你在「管理此文件」里审批 → 对方即可解密浏览").size(12.0).color(theme::TEXT_DIM));
                });
            });
        });
    }

    pub(crate) fn show_mine(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("我打包的文件")
                    .size(20.0)
                    .strong()
                    .color(theme::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::badge(ui, &format!("{} 个", self.packed_list.len()), theme::ACCENT);
            });
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("你创建的加密文件，点击可打开、授权与管理")
                .size(12.0)
                .color(theme::TEXT_DIM),
        );
        ui.add_space(12.0);

        if self.packed_list.is_empty() {
            theme::card(ui, theme::CARD, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new("还没有打包记录")
                            .size(15.0)
                            .color(theme::TEXT_DIM),
                    );
                    ui.add_space(8.0);
                    if ui.button("去打包第一个加密文件").clicked() {
                        self.mode = Mode::Pack;
                    }
                    ui.add_space(24.0);
                });
            });
            return;
        }

        let list = self.packed_list.clone();
        let mut open: Option<PathBuf> = None;
        let mut remove: Option<PathBuf> = None;
        let mut reveal: Option<PathBuf> = None;
        theme::card(ui, theme::CARD, |ui| {
            for (i, e) in list.iter().enumerate() {
                ui.horizontal(|ui| {
                    crate::icons::file(ui, 18.0, theme::TEXT_DIM);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(&e.name)
                                .size(14.0)
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · {} · ID {}",
                                e.created_at,
                                human_size(e.size as usize),
                                short_id(&e.app_id)
                            ))
                            .size(11.0)
                            .color(theme::TEXT_DIM),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::icons::cross_click(ui, 14.0, theme::ERR)
                            .on_hover_text("从列表移除")
                            .clicked()
                        {
                            remove = Some(e.path.clone());
                        }
                        if crate::icons::folder_click(ui, 14.0, theme::TEXT_DIM)
                            .on_hover_text("打开所在文件夹")
                            .clicked()
                        {
                            reveal = Some(e.path.clone());
                        }
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("打开").size(12.0))
                                    .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            open = Some(e.path.clone());
                        }
                    });
                });
                if i + 1 < list.len() {
                    ui.separator();
                }
            }
        });
        if let Some(p) = remove {
            PackedEntry::remove(&p);
            self.packed_list = PackedEntry::load_all();
        }
        if let Some(p) = reveal {
            let _ = std::process::Command::new("explorer")
                .arg(format!("/select,{}", p.display()))
                .spawn();
        }
        if let Some(p) = open {
            self.current_file = Some(p);
            self.load_file_info();
            self.open_tab = OpenTab::Browse;
        }

        ui.add_space(16.0);
        if ui.button("打包新文件").clicked() {
            self.mode = Mode::Pack;
        }
    }

    pub(crate) fn show_browse(&mut self, ui: &mut egui::Ui) {
        if self.vfs.is_none() {
            theme::card(ui, theme::CARD, |ui| {
                ui.horizontal(|ui| {
                    crate::icons::lock(ui, 24.0, theme::ACCENT);
                    ui.label(
                        egui::RichText::new("解锁并浏览内容")
                            .size(18.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::badge(
                            ui,
                            &format!("ID {}", short_id(&self.app_id)),
                            theme::TEXT_DIM,
                        );
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("内容在内存中解密，只读、不落盘，关闭即销毁")
                        .size(13.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add_space(18.0);

                ui.horizontal(|ui| {
                    if theme::cta(ui, "解密并打开（内存）").clicked() {
                        self.do_open();
                    }
                    ui.add_space(16.0);
                    if theme::secondary(ui, "申请临时权限").clicked() {
                        self.do_request_access();
                    }
                });

                // 口令输入框只在服务端因口令拒绝（缺失/错误）或申请时需要时出现，
                // 管理员凭 secret 路径不显示、也不需要口令。
                if self.show_password_input {
                    ui.add_space(14.0);
                    egui::Frame::none()
                        .fill(theme::INPUT_BG)
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                crate::icons::lock(ui, 13.0, theme::TEXT_DIM);
                                ui.label(
                                    egui::RichText::new("取密钥口令")
                                        .size(13.0)
                                        .strong()
                                        .color(theme::TEXT_DIM),
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    "普通用户取密钥需校验口令（明文发往服务端校验）；管理员凭密钥文件，无需口令",
                                )
                                .size(12.0)
                                .color(theme::TEXT_DIM),
                            );
                            if !self.password_hint.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(self.password_hint.clone())
                                        .size(12.0)
                                        .color(theme::ERR),
                                );
                            } else if self.remembered_source.is_some() {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new("已自动填入本机记住的口令")
                                        .size(12.0)
                                        .color(theme::OK),
                                );
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("口令")
                                        .size(13.0)
                                        .color(theme::TEXT_DIM),
                                );
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut self.open_password)
                                        .password(true)
                                        .desired_width(220.0)
                                        .hint_text("申请或授权时设定的口令"),
                                );
                                let entered = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if ui.button("用此口令重试").clicked() || entered {
                                    self.do_open();
                                }
                            });
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    &mut self.remember_password,
                                    egui::RichText::new("记住口令")
                                        .size(13.0)
                                        .color(theme::TEXT_DIM),
                                );
                                ui.label(
                                    egui::RichText::new(
                                        "仅在本机 config.json 明文保存，取密钥成功后才会写入；可在设置页清除",
                                    )
                                    .size(12.0)
                                    .color(theme::TEXT_DIM),
                                );
                            });
                        });
                }

                ui.add_space(14.0);
                if ui
                    .link(
                        egui::RichText::new("申请选项…")
                            .size(13.0)
                            .color(theme::TEXT_DIM),
                    )
                    .clicked()
                {
                    self.show_request_opts = !self.show_request_opts;
                }
                if self.show_request_opts {
                    ui.add_space(8.0);
                    egui::Frame::none()
                        .fill(theme::INPUT_BG)
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("天数")
                                        .size(13.0)
                                        .color(theme::TEXT_DIM),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.request_days)
                                        .desired_width(60.0),
                                );
                                ui.label(
                                    egui::RichText::new("说明")
                                        .size(13.0)
                                        .color(theme::TEXT_DIM),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.request_message)
                                        .desired_width(240.0)
                                        .hint_text("选填"),
                                );
                            });
                        });
                }
            });
            return;
        }

        let (entries, file_count, total_size) = {
            let Some(vfs) = &self.vfs else { return };
            (
                vfs.list_dir(&self.cur_dir),
                vfs.file_count(),
                vfs.total_size(),
            )
        };

        ui.horizontal(|ui| {
            theme::badge(ui, &format!("内存挂载 · {} 个文件 · {}", file_count, human_size(total_size)), theme::OK);
            theme::badge(ui, "只读 · 关闭后自动销毁", theme::TEXT_DIM);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("卸载").clicked() {
                    if let Some(h) = self.mount_handle.take() { unmount_drive(h); }
                    self.vfs = None;
                    self.preview_path = None;
                }
                if self.mount_handle.is_some() {
                    if ui.button("卸载资源管理器盘").clicked() {
                        if let Some(h) = self.mount_handle.take() {
                            let mapped = h.is_mapped();
                            unmount_drive(h);
                            let msg = if mapped {
                                "已卸载资源管理器挂载"
                            } else {
                                "已停止 WebDAV 服务（此前未建立盘符映射）"
                            };
                            self.show_status(msg, false);
                        }
                    }
                } else if ui.button("挂载到资源管理器").clicked() {
                    if let Some(vfs) = self.vfs.clone() {
                        // 盘符不写死：按系统实际占用情况选一个空闲盘符，
                        // 没有空闲盘符时只启动 WebDAV 服务并如实报错。
                        let drive = pick_free_drive();
                        match mount_vfs_to_drive(vfs, drive.as_deref()) {
                            Ok(h) => {
                                let letter = h.drive().to_string();
                                let url = h.url();
                                self.mount_handle = Some(h);
                                self.show_status(&format!("已挂载到 {}（WebDAV {}）。若资源管理器未出现，可手动「映射网络驱动器」填 {}", letter, url, url), false);
                            }
                            Err(e) => {
                                // 服务可能仍在运行，保留句柄以便「卸载」能停掉它；
                                // 失败时 unmount 不会删除任何盘符映射。
                                let msg = e.to_string();
                                if let Some(h) = e.into_handle() {
                                    self.mount_handle = Some(h);
                                }
                                self.show_status(&msg, true);
                            }
                        }
                    }
                }
            });
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("上级").clicked() {
                if let Some(pos) = self.cur_dir.rfind('/') {
                    self.cur_dir.truncate(pos);
                } else {
                    self.cur_dir.clear();
                }
                self.preview_path = None;
            }
            ui.label(
                egui::RichText::new(format!("/{}", self.cur_dir))
                    .size(13.0)
                    .color(theme::TEXT_DIM),
            );
        });
        ui.add_space(8.0);

        ui.columns(2, |cols| {
            theme::card(&mut cols[0], theme::CARD, |ui| {
                ui.label(
                    egui::RichText::new("文件")
                        .size(15.0)
                        .strong()
                        .color(theme::TEXT),
                );
                ui.add_space(8.0);
                let mut action: Option<(String, bool)> = None;
                egui::ScrollArea::vertical()
                    .id_salt("file_list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &entries {
                            let (icon, name) = if entry.is_dir {
                                ("", format!("{}/", entry.name))
                            } else {
                                ("", entry.name.clone())
                            };
                            let resp = egui::Frame::none()
                                .fill(theme::INPUT_BG)
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(egui::Margin::symmetric(12.0, 9.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(icon).size(15.0));
                                        ui.label(
                                            egui::RichText::new(&name)
                                                .size(14.0)
                                                .color(theme::TEXT),
                                        );
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());
                            if resp.clicked() {
                                action = Some((entry.path.clone(), entry.is_dir));
                            }
                            ui.add_space(4.0);
                        }
                    });
                if let Some((path, is_dir)) = action {
                    self.pending_action = Some(if is_dir {
                        Action::EnterDir(path)
                    } else {
                        Action::Preview(path)
                    });
                }
            });

            theme::card(&mut cols[1], theme::CARD, |ui| {
                ui.label(
                    egui::RichText::new("预览")
                        .size(15.0)
                        .strong()
                        .color(theme::TEXT),
                );
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("preview")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match &self.preview_path {
                        Some(p) => {
                            ui.label(
                                egui::RichText::new(p.to_string())
                                    .size(13.0)
                                    .color(theme::TEXT_DIM),
                            );
                            ui.add_space(8.0);
                            if VirtualFS::is_text_file(p) {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.preview_text.as_str())
                                        .desired_width(f32::INFINITY)
                                        .font(egui::TextStyle::Monospace),
                                );
                            } else if VirtualFS::is_image_file(p) {
                                ui.horizontal(|ui| {
                                    crate::icons::image(ui, 14.0, theme::TEXT);
                                    ui.label(
                                        egui::RichText::new("图片文件（内存中）")
                                            .size(14.0)
                                            .color(theme::TEXT),
                                    );
                                });
                            } else {
                                ui.label(
                                    egui::RichText::new("二进制文件")
                                        .size(14.0)
                                        .color(theme::TEXT_DIM),
                                );
                            }
                        }
                        None => {
                            ui.add_space(30.0);
                            ui.vertical_centered(|ui| {
                                crate::icons::file(ui, 32.0, theme::TEXT_DIM);
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new("点击左侧文件查看")
                                        .size(14.0)
                                        .color(theme::TEXT_DIM),
                                );
                            });
                        }
                    });
            });
        });
    }

    pub(crate) fn do_open(&mut self) {
        // 非管理员走 user_id 授权需个人 ID；管理员凭 secret，无需
        let admin = self.is_admin && !self.secret.is_empty();
        if !admin && !self.ensure_user() {
            return;
        }
        self.show_status("正在连接服务器…", false);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            if admin {
                api::request_key_admin(&self.server_url, &self.app_id, &self.secret).await
            } else {
                // 口令留空时不发送该字段，服务端会明确回「口令缺失」
                let pwd = if self.open_password.is_empty() {
                    None
                } else {
                    Some(self.open_password.as_str())
                };
                api::request_key(&self.server_url, &self.app_id, &self.user_id, pwd).await
            }
        });
        match result {
            Ok(key) => {
                let mount = self.open_loader().map(|l| l.mount_vfs_with_key(&key));
                match mount {
                    Some(Ok(vfs)) => {
                        // 只有在勾选「记住口令」且本次取密钥成功后才写入配置；
                        // 失败（含口令错误）绝不写入。
                        if !admin && self.remember_password && !self.open_password.is_empty() {
                            self.remember_current_password();
                        }
                        self.vfs = Some(Arc::new(vfs));
                        self.cur_dir.clear();
                        self.preview_path = None;
                        self.show_password_input = false;
                        self.password_hint.clear();
                        self.show_status("已挂载（内存只读，未落盘）", false);
                    }
                    Some(Err(e)) => self.show_status(&format!("解密失败: {}", e), true),
                    None => self.show_status("文件加载失败", true),
                }
            }
            Err(e) => {
                // 口令被拒不是死路：就地展开口令输入框，让用户改正后重试，
                // 并把服务端 message 原样展示（错误口令 / 缺少口令 / 旧授权无口令）
                if !admin && crate::app::is_password_refusal(&e) {
                    let mut hint = e.clone();
                    // 服务端判「口令错误」：说明记住的口令已失效（例如管理员改过口令）。
                    // 立刻删掉本地记忆并清空输入框，避免一直拿错口令重试。
                    if crate::app::is_wrong_password(&e) && !self.open_password.is_empty() {
                        let forgotten = self.forget_rejected_password();
                        self.open_password.clear();
                        if forgotten {
                            hint = format!(
                                "{} —— 已清除本机记住的口令，请向管理员确认当前口令后重新输入",
                                e
                            );
                        }
                    }
                    self.show_password_input = true;
                    self.password_hint = hint.clone();
                    self.show_status(&hint, true);
                    return;
                }
                let msg = if e.contains("网络错误") {
                    format!(
                        "无法连接服务器（{}）：请确认服务端已启动、地址正确",
                        self.server_url
                    )
                } else if e.contains("授权") || e.contains("权限") || e.contains("未") {
                    format!("{} —— 可点「申请临时权限」向管理员申请访问", e)
                } else {
                    e.to_string()
                };
                self.show_status(&msg, true);
            }
        }
    }

    pub(crate) fn do_request_access(&mut self) {
        if !self.ensure_user() {
            return;
        }
        // need_days 必须是正整数（服务端拒绝 0 与负数），先在本地拦住并说明
        let days: i32 = match self.request_days.trim().parse::<i32>() {
            Ok(d) if d > 0 => d,
            _ => {
                self.show_status("申请天数必须是正整数（例如 3）", true);
                return;
            }
        };
        if self.open_password.is_empty() {
            self.show_password_input = true;
            self.show_status(
                "请先在下方「取密钥口令」里设定口令（申请时自行设定，之后取密钥要用它），再点「申请临时权限」",
                true,
            );
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let msg = if self.request_message.is_empty() {
                None
            } else {
                Some(self.request_message.clone())
            };
            api::request_access(
                &self.server_url,
                &self.app_id,
                &self.user_id,
                Some(days),
                msg,
                &self.open_password,
            )
            .await
        });
        match result {
            Ok(msg) => self.show_status(
                &format!("{} —— 等管理员审批通过后，再点「解密并打开」", msg),
                false,
            ),
            Err(e) => self.show_status(&e.to_string(), true),
        }
    }
}
