//! Preview geometry while dragging; commit one undoable NDPA edit on release.
use super::*;

pub(super) struct Gesture {
    index: usize,
    before: Annotation,
    pub preview: Annotation,
    anchor: egui::Vec2,
    handle: Option<usize>,
}

fn handles(a: &Annotation) -> Vec<[f32; 2]> {
    if a.tool == Tool::Circle {
        let [lo, hi] = polygon::bounds(&a.points);
        vec![[hi[0], (lo[1] + hi[1]) * 0.5]]
    } else {
        a.points.clone()
    }
}

fn adjusted(a: &Annotation, handle: Option<usize>, delta: egui::Vec2, slide: &Slide) -> Annotation {
    let mut result = a.clone();
    if let Some(index) = handle {
        if a.tool == Tool::Circle {
            let center = (egui::Vec2::from(a.points[0]) + egui::Vec2::from(a.points[1])) * 0.5;
            result.points =
                circle::from_drag(center, egui::Vec2::from(handles(a)[0]) + delta, slide).to_vec();
        } else {
            result.points[index] = (egui::Vec2::from(a.points[index]) + delta)
                .clamp(
                    egui::Vec2::ZERO,
                    egui::vec2(slide.width as f32, slide.height as f32),
                )
                .into();
        }
    } else {
        let [lo, hi] = polygon::bounds(&a.points);
        // Imported shapes may extend into slide margins: preserve their geometry.
        let mut offset = delta;
        for axis in 0..2 {
            let limit = [slide.width, slide.height][axis] as f32;
            if lo[axis] >= 0.0 && hi[axis] <= limit {
                offset[axis] = offset[axis].clamp(-lo[axis], limit - hi[axis]);
            }
        }
        for point in &mut result.points {
            *point = (egui::Vec2::from(*point) + offset).into();
        }
    }
    result
}

fn valid(a: &Annotation, slide: &Slide) -> bool {
    if a.points.iter().flatten().any(|v| !v.is_finite()) {
        return false;
    }
    match a.tool {
        Tool::Polygon => polygon::valid(&a.points),
        Tool::Circle => circle::valid(&a.points, slide.calibration),
        Tool::Rectangle => {
            a.points.len() == 2
                && (0..2).all(|axis| (a.points[1][axis] - a.points[0][axis]).abs() >= 1.0)
        }
        Tool::Line | Tool::Pointer => {
            a.points.len() == 2
                && (egui::Vec2::from(a.points[1]) - egui::Vec2::from(a.points[0])).length() >= 1.0
        }
        _ => false,
    }
}

impl Viewer {
    pub(crate) fn edit_geometry(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        if !self.features.edit_shape
            || self.features.tool != Tool::Pan
            || !self.features.show_annotations
            || self.shown_plane != Some(self.plane)
        {
            self.features.geometry_gesture = None;
            return false;
        }
        let Some(slide) = self.slide.as_ref() else {
            return false;
        };
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(p) = ui.ctx().input(|i| i.pointer.press_origin()) {
                let world = self.world_point(p, rect);
                let selected = self.features.selected.filter(|&index| {
                    self.features
                        .annotations
                        .get(index)
                        .is_some_and(|a| a.plane == self.plane)
                });
                let handle = selected.and_then(|index| {
                    handles(&self.features.annotations[index])
                        .iter()
                        .position(|h| {
                            self.screen_point(egui::Vec2::from(*h), rect).distance(p) <= 10.0
                        })
                });
                let index = if handle.is_some() {
                    selected
                } else {
                    self.pick_annotation(p, rect)
                };
                if let Some(index) = index {
                    let before = self.features.annotations[index].clone();
                    self.features.selected = Some(index);
                    self.features.edits.end_group();
                    self.features.geometry_gesture = Some(Gesture {
                        index,
                        preview: before.clone(),
                        before,
                        anchor: world,
                        handle,
                    });
                }
            }
        }
        let dragging = self.features.geometry_gesture.is_some();
        if let Some(p) = response.interact_pointer_pos() {
            let world = self.world_point(p, rect);
            if let Some(g) = &mut self.features.geometry_gesture {
                g.preview = adjusted(&g.before, g.handle, world - g.anchor, slide);
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(g) = self.features.geometry_gesture.take() {
                if valid(&g.preview, slide)
                    && self.features.annotations.get(g.index) == Some(&g.before)
                {
                    self.features.annotations[g.index] = g.preview.clone();
                    self.features.edits.edited(g.index, g.before, g.preview);
                    self.features.edits.end_group();
                    self.features.save(slide);
                } else {
                    self.features.status =
                        Some("Shape unchanged: the proposed geometry is invalid.".into());
                }
            }
        }
        dragging
    }

    pub(super) fn paint_edit_handles(&self, ui: &egui::Ui, rect: egui::Rect) {
        if self.features.export.is_some() {
            return;
        }
        if !self.features.edit_shape
            || self.features.tool != Tool::Pan
            || !self.features.show_annotations
        {
            return;
        }
        let Some(index) = self.features.selected else {
            return;
        };
        let Some(a) = self.features.annotations.get(index) else {
            return;
        };
        let a = self
            .features
            .geometry_gesture
            .as_ref()
            .map_or(a, |g| &g.preview);
        if Some(a.plane) != self.shown_plane {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        for p in handles(a) {
            let p = self.screen_point(egui::Vec2::from(p), rect);
            painter.circle_filled(p, 6.0, egui::Color32::WHITE);
            painter.circle_stroke(p, 6.0, egui::Stroke::new(2.0_f32, egui::Color32::BLACK));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translation_resize_and_invalid_contours_preserve_identity() {
        let slide = crate::ndpa::test_slide();
        let a = Annotation {
            appearance: Default::default(),
            tool: Tool::Line,
            points: vec![[10.0, 10.0], [30.0, 30.0]],
            plane: 0,
            note: "Keep".into(),
            origin: Some(4),
        };
        let moved = adjusted(&a, None, egui::vec2(-100.0, 5.0), &slide);
        assert_eq!(moved.points, vec![[0.0, 15.0], [20.0, 35.0]]);
        assert_eq!(moved.origin, a.origin);
        assert_eq!(moved.note, a.note);
        let resized = adjusted(&a, Some(1), egui::vec2(-20.0, -20.0), &slide);
        assert!(!valid(&resized, &slide));
        let mut polygon = a.clone();
        polygon.tool = Tool::Polygon;
        polygon.points = vec![[0.0, 0.0], [20.0, 20.0], [0.0, 20.0], [20.0, 0.0]];
        assert!(!valid(&polygon, &slide));
    }
}
