//! Overview navigation and tracking controls, separate from annotation editing.
use crate::{same_region, Slide, Viewer};
use eframe::egui;

#[derive(Default)]
pub(super) struct State {
    pub start: Option<egui::Vec2>,
}

impl Viewer {
    fn fit_navigator_region(&mut self, start: egui::Vec2, end: egui::Vec2) {
        let area = (end - start).abs();
        if area.min_elem() <= 0.0
            || !area.is_finite()
            || self.viewport_size.min_elem() <= 0.0
            || !self.viewport_size.is_finite()
        {
            return;
        }
        self.center = (start + end) * 0.5;
        let size = self.world_view_size(self.viewport_size);
        self.zoom = (0.95 * (size.x / area.x).min(size.y / area.y)).clamp(0.0001, 8.0);
        self.fit = false;
        self.features.cancel_gesture();
        self.changed();
    }

    pub(crate) fn navigator_rect(&self, canvas: egui::Rect) -> Option<egui::Rect> {
        if self.features.workflow.hide_marks
            || !self.preferences.show_minimap
            || (self.chrome.clean_view && !self.preferences.clean_show_minimap)
        {
            return None;
        }
        self.slide.as_ref()?;
        let height = self.preferences.map_height.min(canvas.height() * 0.35);
        let width = (height * 1.4).min(canvas.width() * 0.4);
        Some(egui::Rect::from_min_size(
            egui::pos2(
                canvas.right() - width - 12.0,
                canvas.bottom() - height - 12.0,
            ),
            egui::vec2(width, height),
        ))
    }

    pub(crate) fn navigator_overlay(&mut self, ui: &mut egui::Ui, canvas: egui::Rect) {
        let Some(outer) = self.navigator_rect(canvas) else {
            return;
        };
        let Some(slide) = self.slide.clone() else {
            return;
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(outer));
        child.set_clip_rect(outer);
        self.paint_navigator(&mut child, &slide, outer);
        // Two neutral outlines remain visible over both brightfield and fluorescence.
        // Paint outside the map so the frame cannot cover tracking or viewport marks.
        for (padding, color) in [
            (4.0, egui::Color32::from_gray(12)),
            (2.0, egui::Color32::from_gray(225)),
        ] {
            ui.painter().rect_stroke(
                outer.expand(padding),
                0.0,
                egui::Stroke::new(2.0_f32, color),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn paint_navigator(&mut self, ui: &mut egui::Ui, slide: &Slide, outer: egui::Rect) {
        let response = ui.interact(
            outer,
            ui.id().with("floating-navigator"),
            egui::Sense::click_and_drag(),
        );
        let factor = (outer.width() / slide.width as f32).min(outer.height() / slide.height as f32);
        let map = egui::Rect::from_center_size(
            outer.center(),
            egui::vec2(slide.width as f32, slide.height as f32) * factor,
        );
        ui.painter()
            .rect_filled(outer, 0.0, egui::Color32::from_gray(30));
        if let Some(entry) = self
            .overview(self.plane)
            .and_then(|r| self.cache.iter().find(|e| same_region(&r, &e.region)))
        {
            ui.painter().image(
                entry.texture.id(),
                map,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        self.features.tracking.paint(ui.painter(), map, self.plane);
        let viewport = egui::Rect::from_center_size(
            map.min + self.center * factor,
            self.world_view_size(self.viewport_size) * factor / self.zoom,
        )
        .intersect(map);
        ui.painter().rect_stroke(
            viewport,
            0.0,
            egui::Stroke::new(1.5_f32, egui::Color32::YELLOW),
            egui::StrokeKind::Inside,
        );
        if response.clicked_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Primary)
        {
            if let Some(p) = response.interact_pointer_pos() {
                if map.contains(p) {
                    self.features.cancel_gesture();
                    self.fit = false;
                    self.center = ((p - map.min) / factor).clamp(
                        egui::Vec2::ZERO,
                        egui::vec2(slide.width as f32, slide.height as f32),
                    );
                    self.changed();
                }
            }
        }
        let secondary = egui::PointerButton::Secondary;
        if response.drag_started_by(secondary) {
            self.features.cancel_gesture();
            self.features.navigator.start = ui
                .input(|i| i.pointer.press_origin())
                .filter(|p| map.contains(*p))
                .map(|p| (p - map.min) / factor);
        }
        if let Some(start) = self.features.navigator.start {
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let end = (pointer.clamp(map.min, map.max) - map.min) / factor;
                ui.painter().rect_stroke(
                    egui::Rect::from_two_pos(map.min + start * factor, map.min + end * factor),
                    0.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE),
                    egui::StrokeKind::Inside,
                );
                if response.drag_stopped_by(secondary) {
                    self.features.navigator.start = None;
                    if ((end - start).abs() * factor).min_elem() >= 4.0 {
                        self.fit_navigator_region(start, end);
                    }
                }
            } else if !ui.input(|i| i.pointer.button_down(secondary)) {
                self.features.navigator.start = None;
            }
        }
        response.on_hover_text("Left-click or drag to navigate. Right-drag a rectangle to zoom to that region. The map keeps the original slide orientation.");
    }

    pub(super) fn navigator_panel(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        ui.separator();
        ui.heading("Navigator");
        ui.add(
            egui::Slider::new(&mut self.preferences.map_height, 100.0..=320.0).text("Map height"),
        );
        ui.checkbox(&mut self.features.tracking.show, "Show visited areas");
        ui.checkbox(&mut self.features.tracking.recording, "Record navigation");
        egui::ComboBox::from_id_salt("tracking-minimum")
            .selected_text(match self.features.tracking.minimum {
                2 => "Visited at 50% zoom or above",
                3 => "Visited at 100% zoom or above",
                4 => "Visited at 200% zoom or above",
                _ => "Visited at 25% zoom or above",
            })
            .show_ui(ui, |ui| {
                for (level, label) in [(1, "25%"), (2, "50%"), (3, "100%"), (4, "200%")] {
                    ui.selectable_value(&mut self.features.tracking.minimum, level, label);
                }
            });
        ui.label(format!(
            "Visited area · Z {}: ~{:.1}%",
            self.plane + 1,
            self.features.tracking.percent(self.plane)
        ));
        ui.small("Green marks loaded views held for 350 ms. Darker green means higher zoom. Approximate image area, not tissue reviewed. Saved per image and Z plane.");
        ui.horizontal_wrapped(|ui| {
            for (level, label) in [(1, "25%"), (2, "50%"), (3, "100%"), (4, "200%+")] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, crate::tracking::color(level));
                ui.small(label);
            }
        });
        if ui.button("Export track map SVG…")
            .on_hover_text("Export the current Z plane and zoom filter as a labelled map in the original slide orientation.")
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new().add_filter("SVG map", &["svg"])
                .set_file_name(format!("tracking-z{}.svg", self.plane + 1)).save_file()
            {
                let name = slide.path.file_name().unwrap_or_default().to_string_lossy();
                let svg = self.features.tracking.export_svg(&name, slide.width, slide.height, self.plane);
                match std::fs::write(&path, svg) {
                    Ok(()) => self.status = format!("Track map exported: {}", path.display()),
                    Err(error) => self.status = format!("Track map export failed: {error}"),
                }
            }
        }
        if ui.button("Clear tracking (all Z planes)").clicked() {
            self.features.tracking.clear();
            self.features.tracking.flush();
        }
        if let Some(message) = self.features.tracking.message() {
            ui.colored_label(egui::Color32::LIGHT_RED, message);
        }
        if self.features.tracking.limit_reached {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Tracking limit: 128 Z planes. Existing history retained.",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigator_region_fits_in_every_orientation_without_changing_z() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.viewport_size = egui::vec2(800.0, 600.0);
        viewer.plane = 3;
        let a = egui::vec2(100.0, 200.0);
        let b = egui::vec2(3100.0, 4200.0);
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), viewer.viewport_size);
        for rotation in 0..4 {
            for mirrored in [[false, false], [true, false], [false, true], [true, true]] {
                viewer.features.rotation = rotation;
                viewer.features.mirrored = mirrored;
                viewer.fit_navigator_region(b, a);
                for point in [a, b, egui::vec2(a.x, b.y), egui::vec2(b.x, a.y)] {
                    assert!(rect.contains(viewer.screen_point(point, rect)));
                }
                assert_eq!(viewer.center, (a + b) * 0.5);
                assert_eq!(viewer.plane, 3);
                assert!(!viewer.fit);
            }
        }
        let previous = (viewer.center, viewer.zoom);
        viewer.fit_navigator_region(a, a);
        assert_eq!((viewer.center, viewer.zoom), previous);
        viewer.fit_navigator_region(a, egui::vec2(f32::NAN, 10.0));
        assert_eq!((viewer.center, viewer.zoom), previous);
    }
}
