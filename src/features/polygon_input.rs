//! Interactive polygon capture; source-coordinate geometry lives in polygon.
use super::{Annotation, Tool};
use crate::{
    polygon::{valid, MAX_VERTICES},
    Viewer,
};
use eframe::egui;

impl Viewer {
    pub(super) fn finish_polygon(&mut self) {
        if !valid(&self.features.polygon_points) {
            self.features.error = Some("A polygon needs 3–256 vertices and a nonzero area, with no crossing or touching edges.".into());
            return;
        }
        if self.features.annotations.len() >= 10000 {
            return;
        }
        let Some(slide) = &self.slide else { return };
        let points = std::mem::take(&mut self.features.polygon_points);
        self.features.add_annotation(Annotation {
            appearance: Default::default(),
            tool: Tool::Polygon,
            points,
            plane: self.plane,
            note: String::new(),
            origin: None,
        });
        self.features.show_annotations = true;
        self.features.selected = Some(self.features.annotations.len() - 1);
        self.features.save(slide);
    }

    pub(super) fn polygon_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) {
        if self.features.tool != Tool::Polygon || self.shown_plane != Some(self.plane) {
            return;
        }
        if response.clicked_by(egui::PointerButton::Primary) {
            if let (Some(pointer), Some(slide)) =
                (response.interact_pointer_pos(), self.slide.as_ref())
            {
                let point = self.world_point(pointer, rect).clamp(
                    egui::Vec2::ZERO,
                    egui::vec2(slide.width as f32, slide.height as f32),
                );
                let distinct = self.features.polygon_points.last().is_none_or(|last| {
                    (point - egui::vec2(last[0], last[1])).length() * self.zoom > 2.0
                });
                if distinct && self.features.polygon_points.len() < MAX_VERTICES {
                    self.features.polygon_points.push([point.x, point.y]);
                }
            }
        }
        let enter = !self.features.polygon_points.is_empty()
            && !ui.ctx().wants_keyboard_input()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if response.double_clicked_by(egui::PointerButton::Primary) || enter {
            self.finish_polygon();
        }
        let mut points: Vec<_> = self
            .features
            .polygon_points
            .iter()
            .map(|p| self.screen_point(egui::vec2(p[0], p[1]), rect))
            .collect();
        let painter = ui.painter().with_clip_rect(rect);
        for &point in &points {
            painter.circle_filled(point, 4.0, egui::Color32::LIGHT_BLUE);
        }
        if response.hovered() {
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                points.push(pointer);
            }
        }
        if points.len() >= 2 {
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE),
            ));
        }
    }
}
