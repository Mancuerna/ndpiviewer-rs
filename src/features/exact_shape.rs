//! Exact fields use physical coordinates; zoom and orientation affect only display.
use super::{Annotation, Tool};
use crate::{Slide, Viewer};
use eframe::egui;

pub(super) struct Settings {
    pub enabled: bool,
    millimeters: bool,
    circle_mode: usize,
    rectangle_area: bool,
    radius: f64,
    diameter: f64,
    circle_area: f64,
    width: f64,
    height: f64,
    area: f64,
    ratio: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            millimeters: false,
            circle_mode: 0,
            rectangle_area: false,
            radius: 100.0,
            diameter: 200.0,
            circle_area: 10_000.0,
            width: 200.0,
            height: 100.0,
            area: 20_000.0,
            ratio: 2.0,
        }
    }
}

impl Settings {
    fn dimensions(&self, tool: Tool, calibrated: bool) -> Result<[f64; 2], &'static str> {
        let scale = if calibrated && self.millimeters {
            1000.0
        } else {
            1.0
        };
        let dimensions = match tool {
            Tool::Circle => {
                let diameter = match self.circle_mode {
                    0 => self.radius * 2.0,
                    1 => self.diameter,
                    _ => 2.0 * (self.circle_area / std::f64::consts::PI).sqrt(),
                };
                [diameter * scale; 2]
            }
            Tool::Rectangle if self.rectangle_area => {
                if !self.ratio.is_finite() || self.ratio <= 0.0 || self.area <= 0.0 {
                    return Err("Enter a positive area and width / height ratio.");
                }
                [
                    (self.area * self.ratio).sqrt() * scale,
                    (self.area / self.ratio).sqrt() * scale,
                ]
            }
            Tool::Rectangle => [self.width * scale, self.height * scale],
            _ => return Err("Choose Circle or Box."),
        };
        if dimensions.iter().all(|v| v.is_finite() && *v > 0.0) {
            Ok(dimensions)
        } else {
            Err("Enter finite dimensions greater than zero.")
        }
    }

    fn change_units(&mut self, millimeters: bool) {
        if millimeters != self.millimeters {
            let factor = if millimeters { 0.001 } else { 1000.0 };
            for value in [
                &mut self.radius,
                &mut self.diameter,
                &mut self.width,
                &mut self.height,
            ] {
                *value *= factor;
            }
            self.area *= factor * factor;
            self.circle_area *= factor * factor;
            self.millimeters = millimeters;
        }
    }
}

fn corners(
    center: egui::Vec2,
    dimensions: [f64; 2],
    slide: &Slide,
) -> Result<[[f32; 2]; 2], &'static str> {
    let spacing = slide.calibration.map_or([1.0; 2], |c| [c.x, c.y]);
    let mut points = [[0.0; 2]; 2];
    for axis in 0..2 {
        let half = dimensions[axis] / spacing[axis] / 2.0;
        if !spacing[axis].is_finite() || spacing[axis] <= 0.0 || !half.is_finite() || half <= 0.0 {
            return Err("Invalid dimensions or image calibration.");
        }
        points[0][axis] = (f64::from(center[axis]) - half) as f32;
        points[1][axis] = (f64::from(center[axis]) + half) as f32;
        if !points[0][axis].is_finite()
            || !points[1][axis].is_finite()
            || points[1][axis] <= points[0][axis]
        {
            return Err("Dimensions are outside the supported precision range.");
        }
    }
    Ok(points)
}

fn fits(points: &[[f32; 2]; 2], slide: &Slide) -> bool {
    points[0][0] >= 0.0
        && points[0][1] >= 0.0
        && points[1][0] <= slide.width as f32
        && points[1][1] <= slide.height as f32
}

fn value(ui: &mut egui::Ui, label: &str, value: &mut f64, unit: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(value)
                .speed(0.1)
                .max_decimals(6)
                .suffix(format!(" {unit}")),
        );
    });
}

impl Viewer {
    pub(super) fn exact_shape_controls(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        let tool = self.features.tool;
        if !matches!(tool, Tool::Circle | Tool::Rectangle) {
            return;
        }
        if ui
            .checkbox(
                &mut self.features.exact_shape.enabled,
                "Exact size · click to place",
            )
            .changed()
        {
            self.features.cancel_gesture();
        }
        let settings = &mut self.features.exact_shape;
        if !settings.enabled {
            return;
        }
        if slide.calibration.is_some() {
            let mut mm = settings.millimeters;
            ui.horizontal(|ui| {
                ui.label("Units");
                ui.selectable_value(&mut mm, false, "µm");
                ui.selectable_value(&mut mm, true, "mm");
            });
            settings.change_units(mm);
        } else {
            ui.small("No calibration: dimensions are in pixels.");
        }
        let unit = if slide.calibration.is_none() {
            "px"
        } else if settings.millimeters {
            "mm"
        } else {
            "µm"
        };
        let squared = format!("{unit}²");
        if tool == Tool::Circle {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut settings.circle_mode, 0, "Radius");
                ui.selectable_value(&mut settings.circle_mode, 1, "Diameter");
                ui.selectable_value(&mut settings.circle_mode, 2, "Area");
            });
            match settings.circle_mode {
                0 => value(ui, "Radius", &mut settings.radius, unit),
                1 => value(ui, "Diameter", &mut settings.diameter, unit),
                _ => value(ui, "Area", &mut settings.circle_area, &squared),
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut settings.rectangle_area, false, "Width × height");
                ui.selectable_value(&mut settings.rectangle_area, true, "Area + ratio");
            });
            if settings.rectangle_area {
                value(ui, "Area", &mut settings.area, &squared);
                value(ui, "Width / height", &mut settings.ratio, "");
            } else {
                value(ui, "Width", &mut settings.width, unit);
                value(ui, "Height", &mut settings.height, unit);
            }
        }
        if let Err(message) = settings.dimensions(tool, slide.calibration.is_some()) {
            ui.colored_label(egui::Color32::LIGHT_RED, message);
        }
        ui.small("Click to center a field. Repeat clicks for identical fields. Red preview: outside image. Escape: Pan.");
    }

    pub(super) fn exact_shape_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
        slide: &Slide,
    ) {
        if self.shown_plane != Some(self.plane)
            || !response.hovered()
            || ui.ctx().memory(|m| m.any_popup_open())
            || ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary))
            || response.dragged()
        {
            return;
        }
        let Some(pointer) = response.hover_pos() else {
            return;
        };
        let tool = self.features.tool;
        let result = self
            .features
            .exact_shape
            .dimensions(tool, slide.calibration.is_some())
            .and_then(|dimensions| corners(self.world_point(pointer, rect), dimensions, slide));
        let Ok(points) = result else {
            response.clone().on_hover_text(result.unwrap_err());
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
            return;
        };
        let allowed = fits(&points, slide) && self.features.annotations.len() < 10000;
        let annotation = Annotation {
            appearance: Default::default(),
            tool,
            points: points.to_vec(),
            plane: self.plane,
            note: String::new(),
            origin: None,
        };
        self.paint_annotation(
            ui,
            rect,
            &annotation,
            if allowed {
                egui::Color32::LIGHT_BLUE
            } else {
                egui::Color32::LIGHT_RED
            },
            &mut Vec::new(),
        );
        ui.ctx().set_cursor_icon(if allowed {
            egui::CursorIcon::Crosshair
        } else {
            egui::CursorIcon::NotAllowed
        });
        if !allowed {
            response.clone().on_hover_text("The entire field must fit inside the image. Its size will not be reduced. Maximum: 10,000 annotations.");
        } else if response.clicked_by(egui::PointerButton::Primary) {
            self.features.add_annotation(annotation);
            self.features.save(slide);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_clicks_place_independent_fields_and_respect_loaded_plane() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.features.tool = Tool::Circle;
        viewer.features.exact_shape.enabled = true;
        viewer.features.writable = false;
        viewer.shown_plane = Some(0);
        viewer.center = egui::vec2(1000.0, 1000.0);
        viewer.zoom = 2.0;
        viewer.features.rotation = 1;
        viewer.features.mirrored = [true, false];
        let ctx = egui::Context::default();
        let point = egui::pos2(258.0, 258.0);
        let mut time = 0.0;
        let frame = |viewer: &mut Viewer, events, time| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(700.0, 600.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(500.0, 500.0),
                            egui::Sense::click_and_drag(),
                        );
                        viewer.canvas_tools(ui, rect, &response);
                    });
                },
            );
        };
        frame(&mut viewer, vec![egui::Event::PointerMoved(point)], time);
        for _ in 0..2 {
            for pressed in [true, false] {
                time += 0.05;
                frame(
                    &mut viewer,
                    vec![egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    time,
                );
            }
        }
        assert_eq!(viewer.features.annotations.len(), 2);
        assert_eq!(
            viewer.features.annotations[0].points,
            vec![[900.0, 900.0], [1100.0, 1100.0]]
        );
        assert_eq!(
            viewer.features.annotations[0].points,
            viewer.features.annotations[1].points
        );
        viewer.features.undo_edit();
        assert_eq!(viewer.features.annotations.len(), 1);
        viewer.features.redo_edit();
        assert_eq!(viewer.features.annotations.len(), 2);
        viewer.features.tool = Tool::Rectangle;
        for pressed in [true, false] {
            time += 0.05;
            frame(
                &mut viewer,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                time,
            );
        }
        assert_eq!(viewer.features.annotations.len(), 3);
        assert_eq!(
            viewer.features.annotations[2].points,
            vec![[900.0, 950.0], [1100.0, 1050.0]]
        );
        viewer.plane = 1;
        for pressed in [true, false] {
            time += 0.05;
            frame(
                &mut viewer,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                time,
            );
        }
        assert_eq!(viewer.features.annotations.len(), 3);
    }

    #[test]
    fn circle_area_and_rectangle_ratio_preserve_physical_size() {
        let mut settings = Settings {
            circle_mode: 2,
            circle_area: 1.0,
            millimeters: true,
            ..Default::default()
        };
        let size = settings.dimensions(Tool::Circle, true).unwrap();
        assert!((std::f64::consts::PI * (size[0] / 2.0).powi(2) - 1_000_000.0).abs() < 1e-6);
        settings.rectangle_area = true;
        settings.area = 1.0;
        settings.ratio = 4.0;
        assert_eq!(
            settings.dimensions(Tool::Rectangle, true).unwrap(),
            [2000.0, 500.0]
        );
        settings.change_units(false);
        assert_eq!(
            settings.dimensions(Tool::Rectangle, true).unwrap(),
            [2000.0, 500.0]
        );
    }

    #[test]
    fn anisotropic_fields_are_centered_and_never_shrunk_at_edges() {
        let slide = crate::ndpa::test_slide();
        let points = corners(egui::vec2(50.0, 40.0), [10.0, 10.0], &slide).unwrap();
        assert_eq!(points, [[30.0, 30.0], [70.0, 50.0]]);
        assert!(fits(&points, &slide));
        assert!(super::super::circle::valid(&points, slide.calibration));
        let edge = corners(egui::vec2(2.0, 40.0), [10.0, 10.0], &slide).unwrap();
        assert!(!fits(&edge, &slide));
        assert_eq!(edge[1][0] - edge[0][0], 40.0);
        assert!(corners(egui::Vec2::ZERO, [f64::NAN, 10.0], &slide).is_err());
        assert!(Settings {
            radius: -1.0,
            ..Default::default()
        }
        .dimensions(Tool::Circle, true)
        .is_err());
    }
}
