use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Rounding, Stroke, TextureHandle, TextureOptions, Vec2};
use mouseroaming_proto::OsKind;

const APPLE_TEX_SIZE: usize = 128;

pub struct OsIconAssets {
    pub apple_logo: TextureHandle,
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceIconStyle {
    pub is_local: bool,
    pub is_active: bool,
    pub is_hovered: bool,
    pub is_dragging: bool,
    pub alpha: f32,
}

impl Default for DeviceIconStyle {
    fn default() -> Self {
        Self {
            is_local: false,
            is_active: false,
            is_hovered: false,
            is_dragging: false,
            alpha: 1.0,
        }
    }
}

impl OsIconAssets {
    pub fn load(ctx: &egui::Context) -> Self {
        let rgba = rasterize_apple_logo_rgba(APPLE_TEX_SIZE);
        let image = ColorImage::from_rgba_unmultiplied([APPLE_TEX_SIZE, APPLE_TEX_SIZE], &rgba);
        let apple_logo = ctx.load_texture("os_icon_apple_logo", image, TextureOptions::LINEAR);
        Self { apple_logo }
    }
}

fn with_alpha(color: Color32, factor: f32) -> Color32 {
    let a = ((color.a() as f32) * factor.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// Paints a crisp, OS-specific window icon (Windows 11 window, macOS Apple window, Linux, etc.).
pub fn paint_os_window_icon(
    painter: &egui::Painter,
    rect: Rect,
    os: OsKind,
    assets: &OsIconAssets,
    style: DeviceIconStyle,
) {
    let alpha = style.alpha.clamp(0.0, 1.0);

    // Soft elevation shadow
    let shadow_offset = if style.is_dragging {
        Vec2::new(0.0, 8.0)
    } else if style.is_hovered {
        Vec2::new(0.0, 5.0)
    } else {
        Vec2::new(0.0, 3.0)
    };
    let shadow_expand = if style.is_dragging { 4.0 } else { 2.0 };
    painter.rect_filled(
        rect.translate(shadow_offset).expand(shadow_expand),
        Rounding::same(11.0),
        with_alpha(Color32::from_rgba_unmultiplied(0, 0, 0, 95), alpha),
    );

    // Outer highlight glow when active, hovered, or dragging
    if style.is_active || style.is_dragging || style.is_hovered {
        let glow_color = if style.is_active {
            Color32::from_rgba_unmultiplied(52, 211, 153, 60)
        } else if style.is_dragging {
            Color32::from_rgba_unmultiplied(56, 189, 248, 80)
        } else {
            Color32::from_rgba_unmultiplied(96, 165, 250, 48)
        };
        painter.rect_filled(
            rect.expand(4.5),
            Rounding::same(12.0),
            with_alpha(glow_color, alpha),
        );
    }

    match os {
        OsKind::Windows => paint_windows11_window(painter, rect, style, alpha),
        OsKind::MacOS => paint_macos_window(painter, rect, assets, style, alpha),
        OsKind::Linux => paint_linux_window(painter, rect, style, alpha),
        OsKind::Unknown => paint_unknown_window(painter, rect, style, alpha),
    }
}

fn border_stroke(default_color: Color32, style: DeviceIconStyle, alpha: f32) -> Stroke {
    let (width, color) = if style.is_active {
        (2.2_f32, Color32::from_rgb(52, 211, 153))
    } else if style.is_dragging {
        (2.2_f32, Color32::from_rgb(56, 189, 248))
    } else if style.is_hovered {
        (1.8_f32, Color32::from_rgb(125, 211, 252))
    } else if style.is_local {
        (1.6_f32, default_color)
    } else {
        (1.3_f32, default_color)
    };
    Stroke::new(width, with_alpha(color, alpha))
}

fn paint_windows11_window(
    painter: &egui::Painter,
    rect: Rect,
    style: DeviceIconStyle,
    alpha: f32,
) {
    let rounding = Rounding::same(8.0);
    let body_fill = if style.is_local || style.is_hovered {
        Color32::from_rgb(28, 46, 72)
    } else {
        Color32::from_rgb(22, 36, 58)
    };
    let title_fill = if style.is_local || style.is_hovered {
        Color32::from_rgb(38, 62, 96)
    } else {
        Color32::from_rgb(31, 50, 78)
    };

    // Window body
    painter.rect_filled(rect, rounding, with_alpha(body_fill, alpha));

    // Title bar
    let title_h = (rect.height() * 0.24).clamp(12.0, 18.0);
    let title_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + title_h));
    painter.rect_filled(
        title_rect,
        Rounding {
            nw: 8.0,
            ne: 8.0,
            sw: 0.0,
            se: 0.0,
        },
        with_alpha(title_fill, alpha),
    );
    painter.line_segment(
        [
            Pos2::new(rect.min.x + 1.0, title_rect.max.y),
            Pos2::new(rect.max.x - 1.0, title_rect.max.y),
        ],
        Stroke::new(1.0_f32, with_alpha(Color32::from_rgb(62, 102, 148), alpha)),
    );

    // Top-right Windows 11 caption buttons: minimize (-), maximize (□), close (×)
    let ctrl_y = title_rect.center().y;
    let ctrl_color = with_alpha(Color32::from_rgb(180, 210, 240), alpha);
    let ctrl_stroke = Stroke::new(1.1_f32, ctrl_color);
    let close_cx = rect.max.x - 10.0;
    let max_cx = close_cx - 12.0;
    let min_cx = max_cx - 12.0;

    // Minimize
    painter.line_segment(
        [Pos2::new(min_cx - 3.0, ctrl_y + 1.0), Pos2::new(min_cx + 3.0, ctrl_y + 1.0)],
        ctrl_stroke,
    );
    // Maximize
    painter.rect_stroke(
        Rect::from_center_size(Pos2::new(max_cx, ctrl_y), Vec2::splat(5.5)),
        Rounding::same(1.0),
        ctrl_stroke,
    );
    // Close
    painter.line_segment(
        [Pos2::new(close_cx - 2.6, ctrl_y - 2.6), Pos2::new(close_cx + 2.6, ctrl_y + 2.6)],
        ctrl_stroke,
    );
    painter.line_segment(
        [Pos2::new(close_cx + 2.6, ctrl_y - 2.6), Pos2::new(close_cx - 2.6, ctrl_y + 2.6)],
        ctrl_stroke,
    );

    // Windows 11 4-pane logo centered in the window content area
    let content_rect = Rect::from_min_max(Pos2::new(rect.min.x, title_rect.max.y), rect.max);
    let logo_size = (content_rect.height() * 0.66).min(content_rect.width() * 0.44);
    let logo_rect = Rect::from_center_size(content_rect.center(), Vec2::splat(logo_size));
    paint_windows11_four_panes(painter, logo_rect, alpha);

    // Outer window border
    painter.rect_stroke(
        rect,
        rounding,
        border_stroke(Color32::from_rgb(82, 168, 255), style, alpha),
    );
}

fn paint_windows11_four_panes(painter: &egui::Painter, rect: Rect, alpha: f32) {
    let gap = (rect.width() * 0.09).max(1.8);
    let pane_w = (rect.width() - gap) * 0.5;
    let pane_h = (rect.height() - gap) * 0.5;
    let r = Rounding::same((rect.width() * 0.06).clamp(1.0, 2.5));

    let panes = [
        (
            Rect::from_min_size(rect.min, Vec2::new(pane_w, pane_h)),
            Color32::from_rgb(84, 210, 255), // Top-left bright cyan
        ),
        (
            Rect::from_min_size(Pos2::new(rect.min.x + pane_w + gap, rect.min.y), Vec2::new(pane_w, pane_h)),
            Color32::from_rgb(44, 181, 246), // Top-right sky blue
        ),
        (
            Rect::from_min_size(Pos2::new(rect.min.x, rect.min.y + pane_h + gap), Vec2::new(pane_w, pane_h)),
            Color32::from_rgb(18, 145, 227), // Bottom-left azure
        ),
        (
            Rect::from_min_size(
                Pos2::new(rect.min.x + pane_w + gap, rect.min.y + pane_h + gap),
                Vec2::new(pane_w, pane_h),
            ),
            Color32::from_rgb(0, 120, 212), // Bottom-right Windows blue
        ),
    ];

    for (pane_rect, color) in panes {
        painter.rect_filled(pane_rect, r, with_alpha(color, alpha));
    }
}

fn paint_macos_window(
    painter: &egui::Painter,
    rect: Rect,
    assets: &OsIconAssets,
    style: DeviceIconStyle,
    alpha: f32,
) {
    let rounding = Rounding::same(9.0);
    let body_fill = if style.is_local || style.is_hovered {
        Color32::from_rgb(46, 51, 62)
    } else {
        Color32::from_rgb(38, 42, 51)
    };
    let title_fill = if style.is_local || style.is_hovered {
        Color32::from_rgb(60, 66, 80)
    } else {
        Color32::from_rgb(51, 56, 68)
    };

    // Window body
    painter.rect_filled(rect, rounding, with_alpha(body_fill, alpha));

    // macOS title bar
    let title_h = (rect.height() * 0.25).clamp(12.0, 18.0);
    let title_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + title_h));
    painter.rect_filled(
        title_rect,
        Rounding {
            nw: 9.0,
            ne: 9.0,
            sw: 0.0,
            se: 0.0,
        },
        with_alpha(title_fill, alpha),
    );
    painter.line_segment(
        [
            Pos2::new(rect.min.x + 1.0, title_rect.max.y),
            Pos2::new(rect.max.x - 1.0, title_rect.max.y),
        ],
        Stroke::new(1.0_f32, with_alpha(Color32::from_rgb(86, 94, 112), alpha)),
    );

    // Classic macOS traffic-light buttons at top-left
    let dot_y = title_rect.center().y;
    let dot_r = (title_h * 0.20).clamp(2.4, 3.4);
    let start_x = rect.min.x + 10.0;
    let step = dot_r * 3.0;
    let lights = [
        Color32::from_rgb(255, 95, 86),  // Close (red)
        Color32::from_rgb(255, 189, 46), // Minimize (yellow)
        Color32::from_rgb(39, 201, 63),  // Fullscreen (green)
    ];
    for (idx, color) in lights.iter().enumerate() {
        painter.circle_filled(
            Pos2::new(start_x + (idx as f32) * step, dot_y),
            dot_r,
            with_alpha(*color, alpha),
        );
    }

    // Crisp anti-aliased Apple logo centered in window content area
    let content_rect = Rect::from_min_max(Pos2::new(rect.min.x, title_rect.max.y), rect.max);
    let logo_size = (content_rect.height() * 0.74).min(content_rect.width() * 0.48);
    let logo_rect = Rect::from_center_size(content_rect.center(), Vec2::splat(logo_size));
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    painter.image(
        assets.apple_logo.id(),
        logo_rect,
        uv,
        with_alpha(Color32::from_rgb(241, 245, 249), alpha),
    );

    // Outer metallic-silver border
    painter.rect_stroke(
        rect,
        rounding,
        border_stroke(Color32::from_rgb(168, 178, 196), style, alpha),
    );
}

fn paint_linux_window(
    painter: &egui::Painter,
    rect: Rect,
    style: DeviceIconStyle,
    alpha: f32,
) {
    let rounding = Rounding::same(8.0);
    painter.rect_filled(rect, rounding, with_alpha(Color32::from_rgb(36, 30, 42), alpha));

    let title_h = (rect.height() * 0.24).clamp(12.0, 18.0);
    let title_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + title_h));
    painter.rect_filled(
        title_rect,
        Rounding {
            nw: 8.0,
            ne: 8.0,
            sw: 0.0,
            se: 0.0,
        },
        with_alpha(Color32::from_rgb(58, 44, 58), alpha),
    );
    painter.circle_filled(
        Pos2::new(rect.max.x - 10.0, title_rect.center().y),
        3.0,
        with_alpha(Color32::from_rgb(233, 84, 32), alpha),
    );

    // Terminal >_ symbol
    let content_rect = Rect::from_min_max(Pos2::new(rect.min.x, title_rect.max.y), rect.max);
    let c = content_rect.center();
    let s = content_rect.height() * 0.24;
    let prompt_stroke = Stroke::new(2.2_f32, with_alpha(Color32::from_rgb(245, 158, 11), alpha));
    painter.line_segment([Pos2::new(c.x - s * 1.1, c.y - s * 0.7), Pos2::new(c.x - s * 0.2, c.y)], prompt_stroke);
    painter.line_segment([Pos2::new(c.x - s * 1.1, c.y + s * 0.7), Pos2::new(c.x - s * 0.2, c.y)], prompt_stroke);
    painter.line_segment(
        [Pos2::new(c.x + s * 0.15, c.y + s * 0.65), Pos2::new(c.x + s * 1.25, c.y + s * 0.65)],
        Stroke::new(2.2_f32, with_alpha(Color32::from_rgb(52, 211, 153), alpha)),
    );

    painter.rect_stroke(
        rect,
        rounding,
        border_stroke(Color32::from_rgb(233, 128, 72), style, alpha),
    );
}

fn paint_unknown_window(
    painter: &egui::Painter,
    rect: Rect,
    style: DeviceIconStyle,
    alpha: f32,
) {
    let rounding = Rounding::same(8.0);
    painter.rect_filled(rect, rounding, with_alpha(Color32::from_rgb(34, 41, 54), alpha));
    let title_h = (rect.height() * 0.24).clamp(12.0, 18.0);
    let title_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + title_h));
    painter.rect_filled(
        title_rect,
        Rounding {
            nw: 8.0,
            ne: 8.0,
            sw: 0.0,
            se: 0.0,
        },
        with_alpha(Color32::from_rgb(48, 58, 74), alpha),
    );
    let content_rect = Rect::from_min_max(Pos2::new(rect.min.x, title_rect.max.y), rect.max);
    let monitor = Rect::from_center_size(content_rect.center(), Vec2::new(24.0, 16.0));
    painter.rect_stroke(
        monitor,
        Rounding::same(2.0),
        Stroke::new(1.6_f32, with_alpha(Color32::from_rgb(160, 174, 192), alpha)),
    );
    painter.rect_stroke(
        rect,
        rounding,
        border_stroke(Color32::from_rgb(120, 134, 156), style, alpha),
    );
}

/// Paints a compact inline OS badge icon (e.g. 18x18) for lists and labels.
pub fn paint_mini_os_icon(
    painter: &egui::Painter,
    rect: Rect,
    os: OsKind,
    assets: &OsIconAssets,
) {
    match os {
        OsKind::Windows => {
            paint_windows11_four_panes(painter, rect.shrink(1.5), 1.0);
        }
        OsKind::MacOS => {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(
                assets.apple_logo.id(),
                rect.shrink(0.5),
                uv,
                Color32::from_rgb(235, 240, 248),
            );
        }
        OsKind::Linux => {
            let c = rect.center();
            let s = rect.width() * 0.25;
            let st = Stroke::new(1.6_f32, Color32::from_rgb(245, 158, 11));
            painter.line_segment([Pos2::new(c.x - s, c.y - s * 0.7), Pos2::new(c.x - 0.5, c.y)], st);
            painter.line_segment([Pos2::new(c.x - s, c.y + s * 0.7), Pos2::new(c.x - 0.5, c.y)], st);
            painter.line_segment(
                [Pos2::new(c.x + 1.0, c.y + s * 0.65), Pos2::new(c.x + s * 1.2, c.y + s * 0.65)],
                Stroke::new(1.6_f32, Color32::from_rgb(52, 211, 153)),
            );
        }
        OsKind::Unknown => {
            painter.rect_stroke(
                rect.shrink(2.0),
                Rounding::same(2.0),
                Stroke::new(1.5_f32, Color32::from_rgb(160, 174, 192)),
            );
        }
    }
}

fn eval_cubic(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let uu = u * u;
    let tt = t * t;
    let uuu = uu * u;
    let ttt = tt * t;
    let x = uuu * p0.0 + 3.0 * uu * t * p1.0 + 3.0 * u * tt * p2.0 + ttt * p3.0;
    let y = uuu * p0.1 + 3.0 * uu * t * p1.1 + 3.0 * u * tt * p2.1 + ttt * p3.1;
    (x, y)
}

fn sample_closed_beziers(segments: &[((f32, f32), (f32, f32), (f32, f32), (f32, f32))]) -> Vec<(f32, f32)> {
    const STEPS: usize = 20;
    let mut pts = Vec::with_capacity(segments.len() * STEPS);
    for &(p0, p1, p2, p3) in segments {
        for s in 0..STEPS {
            let t = s as f32 / STEPS as f32;
            pts.push(eval_cubic(p0, p1, p2, p3, t));
        }
    }
    pts
}

fn point_in_polygon(px: f32, py: f32, poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn apple_polygons() -> (Vec<(f32, f32)>, Vec<(f32, f32)>) {
    // Classic bitten Apple silhouette in normalized [0..100] x [0..100] space
    let body_segments = [
        // Top-center indent to right upper shoulder
        ((50.0, 32.5), (56.5, 32.5), (61.5, 27.5), (69.5, 28.0)),
        ((69.5, 28.0), (75.5, 28.4), (80.5, 31.5), (83.5, 36.5)),
        // Signature circular bite on the right side
        ((83.5, 36.5), (75.5, 41.0), (73.5, 48.5), (74.2, 54.5)),
        ((74.2, 54.5), (75.0, 61.5), (80.0, 65.5), (86.0, 67.5)),
        // Lower-right belly down to right foot
        ((86.0, 67.5), (83.0, 76.5), (76.5, 88.0), (69.0, 92.0)),
        ((69.0, 92.0), (64.5, 94.3), (60.5, 91.8), (55.8, 90.0)),
        // Bottom-center gentle dip to left foot
        ((55.8, 90.0), (52.5, 88.8), (47.5, 88.8), (44.2, 90.0)),
        ((44.2, 90.0), (39.5, 91.8), (35.5, 94.3), (31.0, 92.0)),
        // Left belly up to left shoulder
        ((31.0, 92.0), (22.5, 87.5), (14.0, 71.5), (14.0, 53.5)),
        ((14.0, 53.5), (14.0, 37.5), (22.5, 28.0), (31.5, 28.0)),
        // Top-left lobe back to top-center indent
        ((31.5, 28.0), (39.0, 28.0), (43.5, 32.5), (50.0, 32.5)),
    ];

    let leaf_segments = [
        ((50.5, 25.5), (59.5, 25.0), (66.5, 17.5), (65.5, 8.0)),
        ((65.5, 8.0), (56.5, 8.5), (49.5, 16.5), (50.5, 25.5)),
    ];

    (
        sample_closed_beziers(&body_segments),
        sample_closed_beziers(&leaf_segments),
    )
}

/// Rasterizes a smooth 4x4 MSAA white silhouette RGBA texture for the Apple logo.
pub fn rasterize_apple_logo_rgba(size: usize) -> Vec<u8> {
    let (body, leaf) = apple_polygons();
    let mut rgba = vec![0u8; size * size * 4];
    let scale = 100.0 / size as f32;

    for y in 0..size {
        for x in 0..size {
            let mut hits = 0u32;
            for sy in 0..4 {
                let py = (y as f32 + (sy as f32 + 0.5) * 0.25) * scale;
                for sx in 0..4 {
                    let px = (x as f32 + (sx as f32 + 0.5) * 0.25) * scale;
                    if point_in_polygon(px, py, &body) || point_in_polygon(px, py, &leaf) {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let idx = (y * size + x) * 4;
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = ((hits * 255 + 8) / 16) as u8;
            }
        }
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_logo_silhouette_has_body_leaf_and_bite() {
        let size = 100usize;
        let rgba = rasterize_apple_logo_rgba(size);
        let alpha_at = |x: usize, y: usize| rgba[(y * size + x) * 4 + 3];

        // Center of apple body must be fully opaque
        assert_eq!(alpha_at(50, 58), 255, "center of apple should be opaque");
        // Leaf above stem notch must be opaque
        assert_eq!(alpha_at(58, 17), 255, "apple leaf should be opaque");
        // Right-side bite cutout must be transparent
        assert_eq!(alpha_at(82, 52), 0, "bite area should be transparent");
        // Gap between leaf and body top dip must be transparent
        assert_eq!(alpha_at(50, 29), 0, "stem gap should be transparent");
        // Corners must be transparent
        assert_eq!(alpha_at(5, 5), 0);
        assert_eq!(alpha_at(95, 95), 0);
    }
}
