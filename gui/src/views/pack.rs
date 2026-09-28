use crate::app::SecUnzipApp;
use crate::model::Mode;
use crate::theme;
use eframe::egui;

impl SecUnzipApp {
    pub(crate) fn show_pack(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("pack_scroll")
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    crate::icons::box_(ui, 44.0, theme::ACCENT);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("打包加密内容")
                            .size(24.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "把文件/文件夹加密打包并注册到服务端，你将成为该内容的管理员",
                        )
                        .size(14.0)
                        .color(theme::TEXT_DIM),
                    );
                    ui.add_space(22.0);
                });

                ui.scope(|ui| {
                    theme::card(ui, theme::CARD, |ui| {
                        theme::section_title(ui, "1", "选择源文件 / 文件夹");
                        ui.add_space(12.0);
                        if self.pack_sources.is_empty() {
                            egui::Frame::none()
                                .fill(theme::INPUT_BG)
                                .rounding(egui::Rounding::same(10.0))
                                .inner_margin(egui::Margin::same(22.0))
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        crate::icons::empty(ui, 26.0, theme::TEXT_DIM);
                                        ui.add_space(6.0);
                                        ui.label(
                                            egui::RichText::new("还没有添加任何文件")
                                                .size(14.0)
                                                .color(theme::TEXT_DIM),
                                        );
                                    });
                                });
                        } else {
                            let mut remove_idx: Option<usize> = None;
                            for (i, src) in self.pack_sources.iter().enumerate() {
                                egui::Frame::none()
                                    .fill(theme::INPUT_BG)
                                    .rounding(egui::Rounding::same(8.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 9.0))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if src.is_dir() {
                                                crate::icons::folder(ui, 15.0, theme::TEXT_DIM);
                                            } else {
                                                crate::icons::file(ui, 15.0, theme::TEXT_DIM);
                                            }
                                            ui.label(
                                                egui::RichText::new(src.display().to_string())
                                                    .size(13.0)
                                                    .color(theme::TEXT),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if crate::icons::cross_click(
                                                        ui,
                                                        13.0,
                                                        theme::ERR,
                                                    )
                                                    .clicked()
                                                    {
                                                        remove_idx = Some(i);
                                                    }
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(4.0);
                            }
                            if let Some(i) = remove_idx {
                                self.pack_sources.remove(i);
                            }
                        }
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if ui.button("添加文件").clicked() {
                                if let Some(paths) = rfd::FileDialog::new().pick_files() {
                                    self.pack_sources.extend(paths);
                                }
                            }
                            if ui.button("添加文件夹").clicked() {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.pack_sources.push(dir);
                                }
                            }
                        });
                    });
                    ui.add_space(14.0);

                    theme::card(ui, theme::CARD, |ui| {
                        theme::section_title(ui, "2", "输出位置");
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if self.pack_output.as_os_str().is_empty() {
                                ui.label(
                                    egui::RichText::new("（未选择输出文件）")
                                        .size(14.0)
                                        .color(theme::TEXT_DIM),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new(self.pack_output.display().to_string())
                                        .size(14.0)
                                        .color(theme::TEXT),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("选择输出文件").clicked() {
                                        let def_name = self
                                            .pack_sources
                                            .first()
                                            .map(|s| {
                                                let stem = s
                                                    .file_stem()
                                                    .map(|x| x.to_string_lossy().to_string())
                                                    .unwrap_or_else(|| "package".into());
                                                format!("{}.secunzip", stem)
                                            })
                                            .unwrap_or_else(|| "package.secunzip".into());
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("SecUnzip", &["secunzip"])
                                            .set_file_name(&def_name)
                                            .save_file()
                                        {
                                            self.pack_output = path;
                                        }
                                    }
                                },
                            );
                        });
                        ui.add_space(10.0);
                        ui.checkbox(
                            &mut self.pack_blackbox,
                            "输出黑盒 EXE（双击黑盒运行，联网取钥）",
                        );
                    });
                    ui.add_space(14.0);

                    theme::card(ui, theme::CARD, |ui| {
                        theme::section_title(ui, "3", "密钥与选项");
                        ui.add_space(12.0);
                        ui.checkbox(&mut self.pack_allow_temp, "允许他人申请临时权限");
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("密钥生成流程（全空 = 随机密钥）")
                                .size(13.0)
                                .color(theme::TEXT_DIM),
                        );
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(theme::INPUT_BG)
                            .rounding(egui::Rounding::same(10.0))
                            .inner_margin(egui::Margin::same(14.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("口令")
                                            .size(13.0)
                                            .color(theme::TEXT_DIM),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.pack_key)
                                            .desired_width(180.0)
                                            .password(true),
                                    );
                                    ui.checkbox(&mut self.pack_flow_machine, "机器码");
                                    ui.checkbox(&mut self.pack_flow_user, "用户");
                                    ui.checkbox(&mut self.pack_flow_date, "日期");
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("派生")
                                            .size(13.0)
                                            .color(theme::TEXT_DIM),
                                    );
                                    let mut h = self.pack_flow_hash;
                                    let sel = match h {
                                        1 => "SHA512",
                                        2 => "Blake3",
                                        3 => "不哈希",
                                        _ => "SHA256",
                                    };
                                    egui::ComboBox::from_id_salt("khash")
                                        .selected_text(sel)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut h, 0, "SHA256");
                                            ui.selectable_value(&mut h, 1, "SHA512");
                                            ui.selectable_value(&mut h, 2, "Blake3");
                                            ui.selectable_value(&mut h, 3, "不哈希");
                                        });
                                    self.pack_flow_hash = h;
                                    ui.checkbox(&mut self.pack_flow_b64, "Base64");
                                });
                            });
                    });
                    ui.add_space(24.0);

                    ui.vertical_centered(|ui| {
                        if theme::cta(ui, "开始打包").clicked() {
                            self.do_pack();
                        }
                        ui.add_space(10.0);
                        if ui
                            .link(
                                egui::RichText::new("← 返回打开")
                                    .size(13.0)
                                    .color(theme::TEXT_DIM),
                            )
                            .clicked()
                        {
                            self.mode = Mode::Open;
                        }
                    });
                    ui.add_space(20.0);
                });
            });
    }
}
