//! 设计系统：配色、主题、通用组件。


use eframe::egui;

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x3b, 0x82, 0xf6);
pub const BG: egui::Color32 = egui::Color32::from_rgb(0x16, 0x19, 0x1e);
pub const CARD: egui::Color32 = egui::Color32::from_rgb(0x20, 0x25, 0x2c);
pub const CARD_HOVER: egui::Color32 = egui::Color32::from_rgb(0x26, 0x2c, 0x35);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0x2e, 0x34, 0x3e);
pub const INPUT_BG: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x30, 0x3a);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe9, 0xed, 0xf2);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x99, 0xa2, 0xb0);
pub const OK: egui::Color32 = egui::Color32::from_rgb(0x34, 0xd3, 0x99);
pub const ERR: egui::Color32 = egui::Color32::from_rgb(0xf8, 0x71, 0x71);
pub const GOLD: egui::Color32 = egui::Color32::from_rgb(0xf5, 0xc5, 0x42);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = INPUT_BG;
    v.faint_bg_color = CARD;
    v.selection.bg_fill = ACCENT;
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    let round = egui::Rounding::same(9.0);
    v.widgets.noninteractive.rounding = round;
    v.widgets.inactive.rounding = round;
    v.widgets.hovered.rounding = round;
    v.widgets.active.rounding = round;
    v.window_rounding = egui::Rounding::same(14.0);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0f32, TEXT_DIM);
    v.widgets.noninteractive.bg_fill = INPUT_BG;
    v.widgets.inactive.bg_fill = INPUT_BG;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0f32, TEXT);
    v.widgets.hovered.bg_fill = CARD_HOVER;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0f32, TEXT);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0f32, egui::Color32::WHITE);

    let mut s = (*ctx.style()).clone();
    s.visuals = v;
    s.spacing.item_spacing = egui::vec2(10.0, 8.0);
    s.spacing.button_padding = egui::vec2(16.0, 9.0);
    s.spacing.interact_size.y = 34.0;
    ctx.set_style(s);
}

/// 圆角卡片容器
pub fn card<R>(ui: &mut egui::Ui, fill: egui::Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::none()
        .fill(fill)
        .rounding(egui::Rounding::same(14.0))
        .inner_margin(egui::Margin::same(22.0))
        .stroke(egui::Stroke::new(1.0f32, BORDER))
        .show(ui, add)
}

/// 大号主操作按钮（CTA）
pub fn cta(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let w = ui.available_width().min(360.0);
    ui.add_sized(
        [w, 50.0],
        egui::Button::new(egui::RichText::new(text).size(16.0).strong().color(egui::Color32::WHITE))
            .fill(ACCENT),
    )
}

/// 次要按钮（描边）
pub fn secondary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width().min(220.0), 40.0],
        egui::Button::new(egui::RichText::new(text).size(14.0)).fill(INPUT_BG),
    )
}

/// 小标签徽章
pub fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.18))
        .rounding(egui::Rounding::same(20.0))
        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.0).color(color));
        });
}

/// 分区标题（序号 + 标题）
pub fn section_title(ui: &mut egui::Ui, no: &str, title: &str) {
    ui.horizontal(|ui| {
        egui::Frame::none()
            .fill(ACCENT)
            .rounding(egui::Rounding::same(12.0))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(no).size(12.0).strong().color(egui::Color32::WHITE));
            });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(title).size(16.0).strong().color(TEXT));
    });
}

/// 居中列：限制最大宽度并在窗口内水平居中（内容块整体居中，卡内文字仍左对齐）
pub fn centered(ui: &mut egui::Ui, max_w: f32, add: impl FnOnce(&mut egui::Ui)) {
    let total = ui.available_width();
    let h = ui.available_height();
    let w = total.min(max_w);
    let pad = ((total - w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(egui::vec2(w, h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_width(w);
            ui.set_max_width(w);
            add(ui);
        });
    });
}
