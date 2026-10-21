//! Bounded freehand capture, simplified to the existing closed NDPA contour.
use super::{polygon, Tool};
use crate::Viewer;
use eframe::egui;

const MAX_SAMPLES: usize = 2048;

fn simplify(mut points: Vec<[f32; 2]>, tolerance: f32) -> Result<Vec<[f32; 2]>, &'static str> {
    if points.len() > MAX_SAMPLES
        || !tolerance.is_finite()
        || tolerance <= 0.0
        || points.iter().flatten().any(|v| !v.is_finite())
    {
        return Err("Invalid freehand contour.");
    }
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return Err("Draw a closed region with at least three points.");
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut work = vec![(0, points.len() - 1)];
    while let Some((start, end)) = work.pop() {
        let a = egui::vec2(points[start][0], points[start][1]);
        let b = egui::vec2(points[end][0], points[end][1]);
        let delta = b - a;
        let mut farthest = None;
        let mut distance = tolerance * tolerance;
        for (index, point) in points.iter().enumerate().take(end).skip(start + 1) {
            let p = egui::vec2(point[0], point[1]);
            let t = if delta.length_sq() > 0.0 {
                ((p - a).dot(delta) / delta.length_sq()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = (p - a - t * delta).length_sq();
            if d > distance {
                distance = d;
                farthest = Some(index);
            }
        }
        if let Some(index) = farthest {
            keep[index] = true;
            work.push((start, index));
            work.push((index, end));
        }
    }
    let reduced: Vec<_> = points
        .into_iter()
        .zip(keep)
        .filter_map(|(p, keep)| keep.then_some(p))
        .collect();
    if reduced.len() > polygon::MAX_VERTICES {
        return Err("Contour too detailed. Draw a smaller region or use Polygon.");
    }
    if !polygon::valid(&reduced) {
        return Err("The contour must enclose an area without crossing itself.");
    }
    Ok(reduced)
}

impl Viewer {
    pub(super) fn freehand_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) {
        if self.features.tool != Tool::Freehand || self.shown_plane != Some(self.plane) {
            return;
        }
        let Some(slide) = &self.slide else {
            return;
        };
        let size = egui::vec2(slide.width as f32, slide.height as f32);
        let primary = egui::PointerButton::Primary;
        if response.drag_started_by(primary) {
            self.features.freehand_points.clear();
            if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                self.features.freehand_points.push(
                    self.world_point(p, rect)
                        .clamp(egui::Vec2::ZERO, size)
                        .into(),
                );
            }
        }
        if !self.features.freehand_points.is_empty()
            && (response.dragged_by(primary) || response.drag_stopped_by(primary))
        {
            if let Some(p) = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.hover_pos()))
            {
                let world = self.world_point(p, rect).clamp(egui::Vec2::ZERO, size);
                let last = self.features.freehand_points.last().unwrap();
                if (world - egui::vec2(last[0], last[1])).length() * self.zoom >= 2.0 {
                    if self.features.freehand_points.len() >= MAX_SAMPLES {
                        self.features.freehand_points.clear();
                        self.features.error =
                            Some("Freehand limit reached. Draw a shorter contour.".into());
                        return;
                    }
                    self.features.freehand_points.push(world.into());
                }
            }
        }
        let vertices: Vec<_> = self
            .features
            .freehand_points
            .iter()
            .map(|p| self.screen_point(egui::vec2(p[0], p[1]), rect))
            .collect();
        ui.painter().with_clip_rect(rect).add(egui::Shape::line(
            vertices,
            egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE),
        ));
        if response.drag_stopped_by(primary) && !self.features.freehand_points.is_empty() {
            let points = std::mem::take(&mut self.features.freehand_points);
            match simplify(points, 1.5 / self.zoom) {
                Ok(points) => {
                    self.features.polygon_points = points;
                    self.finish_polygon();
                }
                Err(message) => self.features.error = Some(message.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dense_contour_simplification_preserves_area_and_rejects_invalid_geometry() {
        let circle: Vec<_> = (0..1000)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 1000.0;
                [100.0 + 100.0 * a.cos(), 100.0 + 100.0 * a.sin()]
            })
            .collect();
        let reduced = simplify(circle, 0.25).unwrap();
        assert!(reduced.len() < 100);
        let (area, _) = polygon::measures(&reduced, None);
        assert!((area / (std::f64::consts::PI * 10000.0) - 1.0).abs() < 0.01);
        assert!(simplify(
            vec![[0.0, 0.0], [10.0, 10.0], [0.0, 10.0], [10.0, 0.0]],
            0.1
        )
        .is_err());
        assert!(simplify(vec![[0.0, 0.0]; MAX_SAMPLES + 1], 1.0).is_err());
    }
}
