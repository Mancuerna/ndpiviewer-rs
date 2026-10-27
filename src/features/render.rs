//! Scale and annotation overlays; source coordinates stay unchanged.
use super::{circle, polygon, Annotation, Tool};
use crate::{calibration, Slide, Viewer};
use eframe::egui;

pub(crate) struct Overlay {
    pub center: egui::Vec2,
    pub zoom: f32,
    pub angle: f32,
    pub rotation: u8,
    pub mirrored: [bool; 2],
    pub calibration: Option<calibration::Calibration>,
    pub show_scale: bool,
    pub show_labels: bool,
    pub annotation_width: f32,
    pub annotation_text_size: f32,
}

impl Viewer {
    fn overlay_style(&self) -> Overlay {
        Overlay {
            center: self.center,
            zoom: self.zoom,
            angle: self.features.angle,
            rotation: self.features.rotation,
            mirrored: self.features.mirrored,
            calibration: self.slide.as_ref().and_then(|s| s.calibration),
            show_scale: self.features.show_scale
                && self.shown_plane.is_some()
                && (!self.chrome.clean_view || self.preferences.clean_show_scale),
            show_labels: self.features.show_labels,
            annotation_width: self.preferences.annotation_width,
            annotation_text_size: self.preferences.annotation_text_size,
        }
    }
    pub(crate) fn paint_scale_bar(&self, ui: &egui::Ui, rect: egui::Rect, slide: &Slide) {
        self.overlay_style().paint_scale_bar(ui, rect, slide);
    }
    pub(crate) fn paint_annotation(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        a: &Annotation,
        color: egui::Color32,
        labels: &mut Vec<egui::Rect>,
    ) {
        self.overlay_style()
            .paint_annotation(ui, rect, a, color, labels);
    }
}

impl Overlay {
    fn screen_point(&self, p: egui::Vec2, rect: egui::Rect) -> egui::Pos2 {
        rect.center()
            + crate::orientation::turn(
                crate::orientation::forward(
                    (p - self.center) * self.zoom,
                    self.rotation,
                    self.mirrored,
                ),
                self.angle,
            )
    }
    pub(crate) fn paint_scale_bar(&self, ui: &egui::Ui, rect: egui::Rect, slide: &Slide) {
        if !self.show_scale || rect.width() < 140.0 || rect.height() < 80.0 {
            return;
        }
        let calibration = slide.calibration.map(|c| {
            let v = crate::orientation::inverse(
                crate::orientation::turn(egui::Vec2::X, -self.angle),
                self.rotation,
                self.mirrored,
            );
            let spacing = (v.x as f64 * c.x).hypot(v.y as f64 * c.y);
            calibration::Calibration {
                x: spacing,
                y: spacing,
            }
        });
        let Some(bar) = calibration::scale_bar(
            calibration,
            self.zoom,
            0,
            160.0_f32.min(rect.width() - 64.0),
        ) else {
            return;
        };
        // Paint inside the exported canvas after annotations so the scale stays
        // readable on bright/dark tissue and is included in viewport screenshots.
        let painter = ui.painter().with_clip_rect(rect);
        let font = egui::FontId::proportional(18.0);
        let text = painter.layout_no_wrap(bar.label, font, egui::Color32::WHITE);
        let width = bar.width.max(text.size().x) + 24.0;
        let panel = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 30.0, rect.bottom() - 72.0),
            egui::vec2(width, 56.0),
        );
        painter.rect_filled(panel, 4.0, egui::Color32::from_black_alpha(210));
        let x = panel.center().x;
        painter.galley(
            egui::pos2(x - text.size().x / 2.0, panel.top() + 5.0),
            text,
            egui::Color32::WHITE,
        );
        let left = egui::pos2(x - bar.width / 2.0, panel.bottom() - 12.0);
        let right = egui::pos2(x + bar.width / 2.0, left.y);
        let stroke = egui::Stroke::new(2.0_f32, egui::Color32::WHITE);
        painter.line_segment([left, right], stroke);
        for p in [left, right] {
            painter.line_segment([p - egui::vec2(0.0, 4.0), p + egui::vec2(0.0, 4.0)], stroke);
        }
    }
    pub(crate) fn paint_annotation(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        a: &Annotation,
        color: egui::Color32,
        label_rects: &mut Vec<egui::Rect>,
    ) {
        let [first, last] = if a.tool == Tool::Polygon {
            polygon::bounds(&a.points)
        } else {
            [a.points[0], a.points[1]]
        };
        let start = egui::vec2(first[0], first[1]);
        let end = egui::vec2(last[0], last[1]);
        let p = self.screen_point(start, rect);
        let q = self.screen_point(end, rect);
        let [lo, hi] = polygon::bounds(&a.points);
        let mut visible = egui::Rect::NOTHING;
        for point in [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]] {
            visible.extend_with(self.screen_point(point.into(), rect));
        }
        if !visible.expand(3.0).intersects(rect) {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        let outline = egui::Stroke::new(
            self.annotation_width + 2.5,
            egui::Color32::from_black_alpha(190),
        );
        let stroke = egui::Stroke::new(self.annotation_width, color);
        if a.tool == Tool::Polygon {
            let vertices: Vec<_> = a
                .points
                .iter()
                .map(|p| self.screen_point(egui::vec2(p[0], p[1]), rect))
                .collect();
            painter.add(egui::Shape::closed_line(vertices.clone(), outline));
            painter.add(egui::Shape::closed_line(vertices, stroke));
        } else if a.tool == Tool::Circle {
            let bounds = egui::Rect::from_two_pos(start.to_pos2(), end.to_pos2());
            let vertices: Vec<_> = (0..128)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 128.0;
                    self.screen_point(
                        bounds.center().to_vec2()
                            + egui::vec2(angle.cos(), angle.sin()) * bounds.size() * 0.5,
                        rect,
                    )
                })
                .collect();
            painter.add(egui::Shape::closed_line(vertices.clone(), outline));
            painter.add(egui::Shape::closed_line(vertices, stroke));
        } else if a.tool == Tool::Pointer {
            let direction = if p.distance(q) > 0.0 {
                (q - p) / p.distance(q)
            } else {
                egui::Vec2::ZERO
            };
            let length = (q - p).length().min(16.0);
            let perpendicular = egui::vec2(-direction.y, direction.x);
            let head = [
                p + direction * length + perpendicular * length * 0.5,
                p,
                p + direction * length - perpendicular * length * 0.5,
            ];
            painter.line_segment([p, q], outline);
            painter.add(egui::Shape::line(head.to_vec(), outline));
            painter.line_segment([p, q], stroke);
            painter.add(egui::Shape::line(head.to_vec(), stroke));
        } else if a.tool == Tool::Line {
            painter.line_segment([p, q], outline);
            painter.line_segment([p, q], stroke);
        } else {
            let points = [
                start,
                egui::vec2(end.x, start.y),
                end,
                egui::vec2(start.x, end.y),
            ]
            .map(|p| self.screen_point(p, rect));
            painter.add(egui::Shape::closed_line(points.to_vec(), outline));
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
        }
        if !self.show_labels {
            return;
        }
        let label = if a.tool == Tool::Pointer {
            String::new()
        } else if a.tool == Tool::Circle {
            circle::label(&a.points, self.calibration)
        } else if a.tool == Tool::Polygon {
            let calibration = self.calibration;
            let (area, perimeter) = polygon::measures(&a.points, calibration);
            let area_label = if calibration.is_some() {
                calibration::area_label(area)
            } else {
                format!("{area:.2} px²")
            };
            let perimeter_label = if calibration.is_some() {
                calibration::length_label(perimeter)
            } else {
                format!("{perimeter:.2} px")
            };
            format!("Area {area_label}\nPerimeter {perimeter_label}")
        } else {
            calibration::measurement(end - start, a.tool == Tool::Line, self.calibration)
        };
        let text = [a.appearance.title.as_str(), label.as_str(), a.note.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            return;
        }
        let p = if a.tool == Tool::Pointer { q } else { p };
        let galley = painter.layout(
            text,
            egui::FontId::proportional(self.annotation_text_size),
            egui::Color32::WHITE,
            (rect.width() - 32.0).clamp(40.0, 420.0),
        );
        let size = galley.size() + egui::vec2(16.0, 12.0);
        let x = p.x.clamp(
            rect.left() + 4.0,
            (rect.right() - size.x - 4.0).max(rect.left() + 4.0),
        );
        let y = (p.y - size.y - 8.0).clamp(
            rect.top() + 4.0,
            (rect.bottom() - size.y - 4.0).max(rect.top() + 4.0),
        );
        let mut background = egui::Rect::from_min_size(egui::pos2(x, y), size);
        // Nearby annotations must not hide one another's measurements.
        let step = size.y + 6.0;
        'placement: for offset in 0..=(rect.height() / step) as usize {
            for direction in [-1.0, 1.0] {
                let candidate = egui::Rect::from_min_size(
                    egui::pos2(x, y + direction * offset as f32 * step),
                    size,
                );
                if rect.shrink(3.0).contains_rect(candidate)
                    && label_rects
                        .iter()
                        .all(|other| !other.expand(3.0).intersects(candidate))
                {
                    background = candidate;
                    break 'placement;
                }
            }
        }
        label_rects.push(background);
        if (background.top() - y).abs() > 1.0 {
            let nearest = p.clamp(background.min, background.max);
            painter.line_segment([p, nearest], egui::Stroke::new(1.0_f32, color));
        }
        painter.rect_filled(background, 5.0, egui::Color32::from_rgb(18, 23, 29));
        painter.galley(
            background.min + egui::vec2(8.0, 6.0),
            galley,
            egui::Color32::WHITE,
        );
    }
}
