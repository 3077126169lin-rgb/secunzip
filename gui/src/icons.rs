//! 矢量图标：手搓绘制，不使用任何 emoji 或图标字体。
//!
//! 用 `egui::Painter` 在 24×24 的设计画布上定义，实际绘制时按 `size` 缩放。
//! 颜色由调用方给定，因此能跟随主题。

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

/// 设计画布边长
const VB: f32 = 24.0;
/// 描边宽度（设计画布单位）
const SW: f32 = 1.8;

fn stroke(color: Color32) -> Stroke {
    Stroke::new(SW, color)
}

/// 画布坐标 → 屏幕坐标
fn p(c: Pos2, s: f32, x: f32, y: f32) -> Pos2 {
    Pos2::new(c.x + (x - VB / 2.0) * s, c.y + (y - VB / 2.0) * s)
}

/// 占用一块 size×size 的区域并绘制图标
fn draw(ui: &mut egui::Ui, size: f32, f: impl FnOnce(&egui::Painter, Pos2, f32)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    f(&painter, rect.center(), size / VB);
    resp
}

/// 可点击版本：悬停时提亮
fn draw_click(
    ui: &mut egui::Ui,
    size: f32,
    color: Color32,
    f: impl FnOnce(&egui::Painter, Pos2, f32),
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    let painter = ui.painter_at(rect);
    let col = if resp.hovered() {
        color
    } else {
        color.gamma_multiply(0.65)
    };
    f(&painter, rect.center(), size / VB);
    let _ = col;
    resp
}

fn poly(painter: &egui::Painter, pts: &[Pos2], st: Stroke) {
    painter.add(Shape::line(pts.to_vec(), st));
}

/// 文件夹轮廓（供普通与可点击两版共用）
fn folder_path(pt: &egui::Painter, c: Pos2, s: f32, st: Stroke) {
    poly(
        pt,
        &[
            p(c, s, 3.5, 19.0),
            p(c, s, 3.5, 5.5),
            p(c, s, 9.5, 5.5),
            p(c, s, 11.5, 8.0),
            p(c, s, 20.5, 8.0),
            p(c, s, 20.5, 19.0),
            p(c, s, 3.5, 19.0),
        ],
        st,
    );
}

// ===== 图标 =====

/// 闭合的锁
pub fn lock(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.rect_stroke(
            Rect::from_min_max(p(c, s, 5.5, 10.5), p(c, s, 18.5, 20.0)),
            2.0 * s,
            st,
        );
        poly(
            pt,
            &[
                p(c, s, 8.5, 10.5),
                p(c, s, 8.5, 8.0),
                p(c, s, 12.0, 5.5),
                p(c, s, 15.5, 8.0),
                p(c, s, 15.5, 10.5),
            ],
            st,
        );
        pt.circle_filled(p(c, s, 12.0, 15.0), 1.4 * s, color);
    })
}

/// 打开的锁
pub fn lock_open(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.rect_stroke(
            Rect::from_min_max(p(c, s, 5.5, 10.5), p(c, s, 18.5, 20.0)),
            2.0 * s,
            st,
        );
        poly(
            pt,
            &[
                p(c, s, 8.5, 10.5),
                p(c, s, 8.5, 8.0),
                p(c, s, 12.0, 5.5),
                p(c, s, 15.5, 8.0),
                p(c, s, 15.5, 9.0),
            ],
            st,
        );
        pt.circle_filled(p(c, s, 12.0, 15.0), 1.4 * s, color);
    })
}

/// 包裹 / 箱子
pub fn box_(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.rect_stroke(
            Rect::from_min_max(p(c, s, 4.5, 8.5), p(c, s, 19.5, 20.0)),
            1.8 * s,
            st,
        );
        poly(
            pt,
            &[p(c, s, 4.5, 8.5), p(c, s, 12.0, 4.5), p(c, s, 19.5, 8.5)],
            st,
        );
        poly(pt, &[p(c, s, 12.0, 4.5), p(c, s, 12.0, 20.0)], st);
    })
}

/// 文件
pub fn file(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        poly(
            pt,
            &[
                p(c, s, 6.0, 3.5),
                p(c, s, 14.0, 3.5),
                p(c, s, 18.5, 8.0),
                p(c, s, 18.5, 20.5),
                p(c, s, 6.0, 20.5),
                p(c, s, 6.0, 3.5),
            ],
            st,
        );
        poly(
            pt,
            &[p(c, s, 14.0, 3.5), p(c, s, 14.0, 8.0), p(c, s, 18.5, 8.0)],
            st,
        );
    })
}

/// 文件夹
pub fn folder(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| folder_path(pt, c, s, stroke(color)))
}

/// 可点击的文件夹（打开所在位置）
pub fn folder_click(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw_click(ui, size, color, |pt, c, s| {
        folder_path(pt, c, s, stroke(color))
    })
}

/// 扳手（管理）
pub fn wrench(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.circle_stroke(p(c, s, 16.5, 7.5), 3.6 * s, st);
        poly(
            pt,
            &[
                p(c, s, 14.0, 10.0),
                p(c, s, 5.5, 18.5),
                p(c, s, 4.5, 20.0),
                p(c, s, 6.0, 19.0),
            ],
            st,
        );
    })
}

/// 齿轮（设置）
pub fn gear(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.circle_stroke(p(c, s, 12.0, 12.0), 3.4 * s, st);
        for i in 0..8 {
            let a = std::f32::consts::TAU * (i as f32) / 8.0;
            let (sn, cs) = a.sin_cos();
            pt.line_segment(
                [
                    p(c, s, 12.0 + cs * 6.5, 12.0 + sn * 6.5),
                    p(c, s, 12.0 + cs * 9.2, 12.0 + sn * 9.2),
                ],
                st,
            );
        }
    })
}

/// 叉（关闭 / 移除）
pub fn cross(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.line_segment([p(c, s, 6.5, 6.5), p(c, s, 17.5, 17.5)], st);
        pt.line_segment([p(c, s, 17.5, 6.5), p(c, s, 6.5, 17.5)], st);
    })
}

/// 可点击的叉（关闭 / 移除）
pub fn cross_click(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw_click(ui, size, color, |pt, c, s| {
        let st = stroke(color);
        pt.line_segment([p(c, s, 6.5, 6.5), p(c, s, 17.5, 17.5)], st);
        pt.line_segment([p(c, s, 17.5, 6.5), p(c, s, 6.5, 17.5)], st);
    })
}

/// 对勾
pub fn check(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        poly(
            pt,
            &[p(c, s, 5.0, 12.5), p(c, s, 10.0, 17.5), p(c, s, 19.0, 6.5)],
            st,
        );
    })
}

/// 地球（网络）
pub fn globe(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.circle_stroke(p(c, s, 12.0, 12.0), 8.0 * s, st);
        pt.line_segment([p(c, s, 4.0, 12.0), p(c, s, 20.0, 12.0)], st);
        poly(
            pt,
            &[p(c, s, 7.0, 6.5), p(c, s, 12.0, 5.5), p(c, s, 17.0, 6.5)],
            st,
        );
        poly(
            pt,
            &[p(c, s, 7.0, 17.5), p(c, s, 12.0, 18.5), p(c, s, 17.0, 17.5)],
            st,
        );
    })
}

/// 用户
pub fn user(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.circle_stroke(p(c, s, 12.0, 8.5), 3.8 * s, st);
        poly(
            pt,
            &[
                p(c, s, 4.5, 20.0),
                p(c, s, 6.5, 15.5),
                p(c, s, 17.5, 15.5),
                p(c, s, 19.5, 20.0),
            ],
            st,
        );
    })
}

/// 图片
pub fn image(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.rect_stroke(
            Rect::from_min_max(p(c, s, 3.5, 5.5), p(c, s, 20.5, 18.5)),
            2.0 * s,
            st,
        );
        pt.circle_filled(p(c, s, 9.0, 10.0), 1.5 * s, color);
        poly(
            pt,
            &[
                p(c, s, 4.5, 17.5),
                p(c, s, 10.0, 12.5),
                p(c, s, 14.0, 15.5),
                p(c, s, 17.0, 13.0),
                p(c, s, 20.0, 16.0),
            ],
            st,
        );
    })
}

/// 提示（灯泡）
pub fn info(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.circle_stroke(p(c, s, 12.0, 9.5), 5.5 * s, st);
        poly(
            pt,
            &[
                p(c, s, 9.5, 15.0),
                p(c, s, 9.5, 18.0),
                p(c, s, 14.5, 18.0),
                p(c, s, 14.5, 15.0),
            ],
            st,
        );
        pt.line_segment([p(c, s, 12.0, 18.0), p(c, s, 12.0, 20.5)], st);
    })
}

/// 上传
pub fn upload(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        pt.line_segment([p(c, s, 12.0, 19.0), p(c, s, 12.0, 5.0)], st);
        poly(
            pt,
            &[p(c, s, 7.0, 10.0), p(c, s, 12.0, 5.0), p(c, s, 17.0, 10.0)],
            st,
        );
        poly(pt, &[p(c, s, 5.0, 21.0), p(c, s, 19.0, 21.0)], st);
    })
}

/// 空盒子（空状态）
pub fn empty(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        let st = stroke(color);
        poly(
            pt,
            &[
                p(c, s, 3.5, 12.0),
                p(c, s, 8.0, 7.5),
                p(c, s, 16.0, 7.5),
                p(c, s, 20.5, 12.0),
                p(c, s, 20.5, 19.5),
                p(c, s, 3.5, 19.5),
                p(c, s, 3.5, 12.0),
            ],
            st,
        );
        pt.line_segment([p(c, s, 3.5, 12.0), p(c, s, 20.5, 12.0)], st);
    })
}

/// 实心圆点（状态灯）
pub fn dot(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    draw(ui, size, |pt, c, s| {
        pt.circle_filled(c, 4.5 * s, color);
    })
}
