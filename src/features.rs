mod capture;
mod exact_shape;
mod freehand;
mod geometry_edit;
mod inspector;
mod navigator;
mod persistence;
mod polygon_input;
mod region_panel;
pub(crate) mod render;
mod report;
use super::*;
#[cfg(test)]
use persistence::{annotation_path, validate};
#[path = "circle.rs"]
pub(super) mod circle;
pub(super) use crate::polygon;

pub(crate) use crate::annotations::{Annotation, Tool};

enum CaptureTarget {
    File(PathBuf, image::ImageFormat),
    Clipboard,
}

pub(super) struct Features {
    exact_shape: exact_shape::Settings,
    edit_shape: bool,
    geometry_gesture: Option<geometry_edit::Gesture>,
    inspector: inspector::Tab,
    pub(super) history: navigation::History,
    pub(super) info_open: bool,
    tracking: tracking::Tracking,
    navigator: navigator::State,
    pub rotation: u8,
    pub mirrored: [bool; 2],
    pub overview_return: Option<(egui::Vec2, f32)>,
    pub tool: Tool,
    annotations: Vec<Annotation>,
    edits: crate::annotation_history::History,
    start: Option<egui::Vec2>,
    polygon_points: Vec<[f32; 2]>,
    freehand_points: Vec<[f32; 2]>,
    selected: Option<usize>,
    deleted_annotation: Option<(usize, Annotation)>,
    annotation_filter: String,
    show_annotations: bool,
    show_labels: bool,
    show_scale: bool,
    writable: bool,
    error: Option<String>,
    archive: Option<ndpa::Archive>,
    last_saved: Option<Vec<u8>>,
    status: Option<String>,
    canvas: Option<egui::Rect>,
    export: Option<(CaptureTarget, egui::Rect)>,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            exact_shape: Default::default(),
            edit_shape: false,
            geometry_gesture: None,
            inspector: inspector::Tab::default(),
            history: navigation::History::default(),
            info_open: false,
            tracking: tracking::Tracking::default(),
            navigator: navigator::State::default(),
            rotation: 0,
            mirrored: [false; 2],
            overview_return: None,
            tool: Tool::Pan,
            annotations: vec![],
            edits: Default::default(),
            start: None,
            polygon_points: Vec::new(),
            freehand_points: Vec::new(),
            selected: None,
            deleted_annotation: None,
            annotation_filter: String::new(),
            show_annotations: true,
            show_labels: true,
            show_scale: true,
            writable: true,
            error: None,
            archive: None,
            last_saved: None,
            status: None,
            canvas: None,
            export: None,
        }
    }
}

impl Features {
    fn add_annotation(&mut self, annotation: Annotation) {
        let index = self.annotations.len();
        self.edits.inserted(index, &annotation);
        self.annotations.push(annotation);
        self.inspector = inspector::Tab::Annotations;
        self.selected = Some(index);
        self.show_annotations = true;
    }

    fn undo_edit(&mut self) {
        if self.edits.can_undo() {
            self.selected = self.edits.undo(&mut self.annotations);
            self.deleted_annotation = None;
            self.cancel_gesture();
        }
    }
    fn redo_edit(&mut self) {
        if self.edits.can_redo() {
            self.selected = self.edits.redo(&mut self.annotations);
            self.deleted_annotation = None;
            self.cancel_gesture();
        }
    }

    fn remove_annotation(&mut self, index: usize) {
        if index < self.annotations.len() {
            let removed = self.annotations.remove(index);
            self.edits.removed(index, &removed);
            self.deleted_annotation = Some((index, removed));
            self.selected = None;
            self.cancel_gesture();
        }
    }

    fn restore_annotation(&mut self) {
        if self.annotations.len() >= 10000 {
            return;
        }
        if let Some((index, annotation)) = self.deleted_annotation.take() {
            let index = index.min(self.annotations.len());
            self.edits.inserted(index, &annotation);
            self.annotations.insert(index, annotation);
            self.selected = Some(index);
            self.show_annotations = true;
        }
    }

    pub(super) fn load_tracking(&mut self, slide: &Slide) {
        self.tracking.load(slide);
    }

    pub(super) fn save_tracking(&mut self) {
        self.tracking.flush();
    }

    pub(super) fn cancel_gesture(&mut self) {
        self.geometry_gesture = None;
        self.navigator.start = None;
        self.start = None;
        self.polygon_points.clear();
        self.freehand_points.clear();
    }
}

pub(super) fn rotate(v: egui::Vec2, turns: u8) -> egui::Vec2 {
    match turns % 4 {
        1 => egui::vec2(-v.y, v.x),
        2 => -v,
        3 => egui::vec2(v.y, -v.x),
        _ => v,
    }
}

impl Viewer {
    pub(crate) fn export_region_visible(&self) -> bool {
        self.features.inspector == inspector::Tab::Export && self.features.export.is_none()
    }
    pub(super) fn pick_annotation(&self, pointer: egui::Pos2, rect: egui::Rect) -> Option<usize> {
        if !self.features.show_annotations || self.shown_plane != Some(self.plane) {
            return None;
        }
        let world = self.world_point(pointer, rect);
        self.features
            .annotations
            .iter()
            .enumerate()
            .rev()
            .find(|(_, a)| a.plane == self.plane && a.hit_test([world.x, world.y], 6.0 / self.zoom))
            .map(|(index, _)| index)
    }

    pub(super) fn visit_annotation(&mut self, index: usize, fit: bool) {
        let Some(a) = self.features.annotations.get(index) else {
            return;
        };
        let [lower, upper] = polygon::bounds(&a.points);
        let first = egui::vec2(lower[0], lower[1]);
        let last = egui::vec2(upper[0], upper[1]);
        self.center = (first + last) * 0.5;
        self.plane = a.plane;
        if fit {
            let area = (last - first).abs().max(egui::Vec2::splat(32.0));
            let available = self.world_view_size(self.viewport_size) * 0.8;
            self.zoom = (available.x / area.x)
                .min(available.y / area.y)
                .clamp(0.0001, 8.0);
        }
        self.features.cancel_gesture();
        self.features.edits.end_group();
        self.features.selected = Some(index);
        self.features.show_annotations = true;
        self.fit = false;
        self.changed();
    }

    pub(super) fn track_displayed_view(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        wanted: &[Region],
    ) {
        let Some(slide) = &self.slide else {
            return;
        };
        let size = egui::vec2(slide.width as f32, slide.height as f32);
        let world = egui::Rect::from_center_size(
            egui::pos2(self.center.x, self.center.y),
            self.world_view_size(rect.size()) / self.zoom,
        );
        let area = egui::Rect::from_min_max(
            egui::pos2(world.min.x / size.x, world.min.y / size.y),
            egui::pos2(world.max.x / size.x, world.max.y / size.y),
        );
        let ready = self.benchmark.is_none()
            && ctx.input(|i| i.focused)
            && self.shown_plane == Some(self.plane)
            && self.channels.dirty_at.is_none()
            && self.channels.active.iter().any(|s| s.enabled)
            && !wanted.is_empty()
            && wanted.iter().all(|r| self.cached(r));
        if self
            .features
            .tracking
            .observe(self.plane, area, self.zoom, ready, Instant::now())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.benchmark.is_none() {
            self.features.tracking.autosave(ctx);
        }
    }

    pub(super) fn inverse_transform(&self, v: egui::Vec2) -> egui::Vec2 {
        orientation::inverse(v, self.features.rotation, self.features.mirrored)
    }
    pub(super) fn world_view_size(&self, size: egui::Vec2) -> egui::Vec2 {
        if self.features.rotation % 2 == 1 {
            egui::vec2(size.y, size.x)
        } else {
            size
        }
    }
    pub(super) fn screen_point(&self, p: egui::Vec2, rect: egui::Rect) -> egui::Pos2 {
        rect.center()
            + orientation::forward(
                (p - self.center) * self.zoom,
                self.features.rotation,
                self.features.mirrored,
            )
    }
    pub(crate) fn world_point(&self, p: egui::Pos2, rect: egui::Rect) -> egui::Vec2 {
        self.center + self.inverse_transform(p - rect.center()) / self.zoom
    }
    pub(super) fn tile_rect(&self, w: [f32; 4], rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_two_pos(
            self.screen_point(egui::vec2(w[0], w[1]), rect),
            self.screen_point(egui::vec2(w[0] + w[2], w[1] + w[3]), rect),
        )
    }
    pub(super) fn paint_tile(&self, ui: &egui::Ui, rect: egui::Rect, e: &CachedRegion) {
        let w = e.region.world;
        let corners = [
            egui::vec2(w[0], w[1]),
            egui::vec2(w[0] + w[2], w[1]),
            egui::vec2(w[0] + w[2], w[1] + w[3]),
            egui::vec2(w[0], w[1] + w[3]),
        ];
        let uv = [
            egui::pos2(0.0, 0.0),
            egui::pos2(1.0, 0.0),
            egui::pos2(1.0, 1.0),
            egui::pos2(0.0, 1.0),
        ];
        let mut mesh = egui::Mesh::with_texture(e.texture.id());
        for (p, uv) in corners.into_iter().zip(uv) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: self.screen_point(p, rect),
                uv,
                color: egui::Color32::WHITE,
            });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        ui.painter()
            .with_clip_rect(rect)
            .add(egui::Shape::mesh(mesh));
    }

    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.benchmark.is_some() || ctx.wants_keyboard_input() || self.slide.is_none() {
            return;
        }
        let undo = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Z));
        let redo = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y));
        if undo || redo {
            if undo {
                self.features.undo_edit();
            } else {
                self.features.redo_edit();
            }
            if let Some(slide) = &self.slide {
                self.features.save(slide);
            }
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            if let Some(index) = self.features.selected {
                self.features.remove_annotation(index);
                if let Some(slide) = &self.slide {
                    self.features.save(slide);
                }
            }
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::M)) {
            self.toggle_mirror(0);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::F)) {
            self.toggle_mirror(1);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::W)) {
            self.toggle_overview();
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::B)) {
            self.features.show_scale = !self.features.show_scale;
            return;
        }
        // egui-winit translates Ctrl+C (including Ctrl+Shift+C) to Copy.
        // Text inputs are excluded by the guard above.
        if !self.features.info_open
            && ctx.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy))
            })
        {
            self.request_capture(ctx, CaptureTarget::Clipboard);
            return;
        }
        // Match the modifiers attached to the key event, even if Alt was
        // released before this frame. Consume arrows before the pan bindings.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)) {
            self.navigate_history(false);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight)) {
            self.navigate_history(true);
            return;
        }
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F) {
                self.fit = true;
            }
            if i.key_pressed(egui::Key::R) {
                self.features.cancel_gesture();
                self.features.rotation = (self.features.rotation + 1) % 4;
                self.changed();
            }
            if i.key_pressed(egui::Key::Escape) {
                self.features.tool = Tool::Pan;
                self.features.cancel_gesture();
            }
            if i.key_pressed(egui::Key::Num1) {
                self.zoom = 1.0;
                self.changed();
            }
            if i.key_pressed(egui::Key::PageUp) {
                self.features.cancel_gesture();
                self.plane = self.plane.saturating_sub(1);
                self.changed();
            }
            if i.key_pressed(egui::Key::PageDown) {
                self.features.cancel_gesture();
                self.plane = (self.plane + 1).min(self.slide.as_ref().unwrap().planes - 1);
                self.changed();
            }
            let mut delta = egui::Vec2::ZERO;
            for (key, step) in [
                (egui::Key::ArrowLeft, egui::vec2(-100.0, 0.0)),
                (egui::Key::ArrowRight, egui::vec2(100.0, 0.0)),
                (egui::Key::ArrowUp, egui::vec2(0.0, -100.0)),
                (egui::Key::ArrowDown, egui::vec2(0.0, 100.0)),
            ] {
                if i.key_pressed(key) {
                    delta += step;
                }
            }
            if delta != egui::Vec2::ZERO {
                self.center += self.inverse_transform(delta) / self.zoom;
                self.changed();
            }
        });
    }

    fn annotation_panel(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        ui.heading("Annotations");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.features.show_annotations, "Show shapes");
            ui.checkbox(&mut self.features.show_labels, "Show labels");
        });
        if self.features.tool == Tool::Pointer {
            ui.small("Drag from the point of interest toward the label. The arrowhead stays at the starting point.");
        }
        if self.features.tool == Tool::Freehand {
            ui.small("Drag around a region; release to close it. The outline is simplified within 1.5 screen pixels. Escape cancels.");
        }
        if self.features.tool == Tool::Circle && !self.features.exact_shape.enabled {
            ui.small("Drag from the center to set the radius. Physical circles account for X/Y calibration.");
        }
        if self.features.tool == Tool::Polygon {
            ui.small("Click vertices; double-click or Enter to close. Escape cancels. Up to 256 vertices.");
            ui.small(format!(
                "Vertices: {} / 256",
                self.features.polygon_points.len()
            ));
            if ui
                .add_enabled(
                    self.features.polygon_points.len() >= 3,
                    egui::Button::new("Finish polygon"),
                )
                .clicked()
            {
                self.finish_polygon();
            }
            if ui
                .add_enabled(
                    !self.features.polygon_points.is_empty(),
                    egui::Button::new("Remove last vertex"),
                )
                .clicked()
            {
                self.features.polygon_points.pop();
            }
        }
        if let Some(c) = slide.calibration {
            ui.small("Measurements in µm and µm²")
                .on_hover_text(format!("Pixel spacing: X {:.6}, Y {:.6} µm/px", c.x, c.y));
        } else {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Uncalibrated: measurements use pixels",
            );
        }
        if self.features.annotations.is_empty() {
            ui.label("No annotations yet. Choose a drawing tool above to mark a region.");
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.features.annotation_filter)
                .hint_text("Search annotations…"),
        );
        let query = self.features.annotation_filter.trim().to_lowercase();
        let matching: Vec<usize> = self
            .features
            .annotations
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                query.is_empty()
                    || format!(
                        "Annotation {} Z {} {} {}",
                        i + 1,
                        a.plane + 1,
                        a.appearance.title,
                        a.note
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        let mut jump = None;
        if matching.is_empty() && !self.features.annotations.is_empty() {
            ui.label("No matching annotations. Try another search.");
        }
        ui.horizontal(|ui| {
            let current = self
                .features
                .selected
                .and_then(|selected| matching.iter().position(|&index| index == selected));
            let previous = current
                .and_then(|i| i.checked_sub(1))
                .or_else(|| current.is_none().then_some(0))
                .and_then(|i| matching.get(i))
                .copied();
            let next = current
                .map_or(Some(0), |i| Some(i + 1))
                .and_then(|i| matching.get(i))
                .copied();
            if ui
                .add_enabled(previous.is_some(), egui::Button::new("Previous"))
                .clicked()
            {
                jump = previous;
            }
            if ui
                .add_enabled(next.is_some(), egui::Button::new("Next"))
                .clicked()
            {
                jump = next;
            }
            ui.label(format!(
                "{} shown · {} total",
                matching.len(),
                self.features.annotations.len()
            ));
        });
        egui::ScrollArea::vertical()
            .max_height(110.0)
            .show(ui, |ui| {
                for &i in &matching {
                    let a = &self.features.annotations[i];
                    let label = if !a.appearance.title.is_empty() {
                        a.appearance.title.clone()
                    } else if a.note.is_empty() {
                        format!("Annotation {}", i + 1)
                    } else {
                        a.note.clone()
                    };
                    if ui
                        .selectable_label(
                            self.features.selected == Some(i),
                            format!("Z {} · {label}", a.plane + 1),
                        )
                        .clicked()
                    {
                        jump = Some(i);
                    }
                }
            });
        if let Some(index) = jump {
            self.visit_annotation(index, false);
        }
        if let Some(index) = self
            .features
            .selected
            .filter(|&i| i < self.features.annotations.len())
        {
            if ui.button("Fit selected annotation").clicked() {
                self.visit_annotation(index, true);
            }
            if ui
                .checkbox(&mut self.features.edit_shape, "Edit shape")
                .changed()
            {
                self.features.cancel_gesture();
                self.features.tool = Tool::Pan;
            }
            if self.features.edit_shape {
                ui.small("Drag white handles to resize; drag the shape to move it. Escape cancels the current drag. Ctrl+Z undoes an edit.");
            }
            let annotation = &mut self.features.annotations[index];
            let before = annotation.clone();
            ui.label("Title");
            let title_changed = ui
                .add(egui::TextEdit::singleline(&mut annotation.appearance.title).char_limit(1000))
                .changed();
            ui.label("Color");
            let color_changed = ui
                .color_edit_button_srgb(&mut annotation.appearance.color)
                .changed();
            ui.label("Notes");
            let note_changed = ui
                .add(
                    egui::TextEdit::multiline(&mut annotation.note)
                        .desired_rows(3)
                        .char_limit(10000),
                )
                .changed();
            if title_changed || color_changed || note_changed {
                self.features
                    .edits
                    .edited(index, before, annotation.clone());
                self.features.save(slide);
            }
            if ui.button("Delete selected annotation").clicked() {
                self.features.remove_annotation(index);
                self.features.save(slide);
            }
        }
        if ui
            .add_enabled(
                self.features.deleted_annotation.is_some()
                    && self.features.annotations.len() < 10000,
                egui::Button::new("Restore deleted annotation"),
            )
            .clicked()
        {
            self.features.restore_annotation();
            self.features.save(slide);
        }
        ui.small("Pan tool: click a shape to select it; double-click to fit it. Delete removes the selected annotation. Saved beside the slide.");
    }

    fn export_panel(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        ui.heading("Export");
        self.region_export_panel(ui, slide);
        ui.separator();
        let ready = self.capture_ready();
        if !ready {
            ui.label("Image export is available when the current view has finished loading. Close Slide information if it is open.");
        }
        self.report_button(ui, slide);
        for (label, extension, format) in [
            ("PNG", "png", image::ImageFormat::Png),
            ("JPEG", "jpg", image::ImageFormat::Jpeg),
        ] {
            if ui
                .add_enabled(ready, egui::Button::new(format!("Export view {label}…")))
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter(label, &[extension])
                    .set_file_name(format!("view.{extension}"))
                    .save_file()
                {
                    self.request_capture(ui.ctx(), CaptureTarget::File(path, format));
                }
            }
        }
        if ui
            .add_enabled(ready, egui::Button::new("Copy view to clipboard"))
            .on_hover_text(
                "Ctrl+C outside text fields. Copy the active pane with its visible annotations and scale bar.",
            )
            .clicked()
        {
            self.request_capture(ui.ctx(), CaptureTarget::Clipboard);
        }
        ui.small("Exports include visible annotations and scale at window resolution. PNG is lossless; JPEG uses quality 95.");
        ui.checkbox(&mut self.features.show_scale, "Show scale bar (B)");
        ui.collapsing("Keyboard & mouse",|ui|{
            ui.label("W: overview / return · Alt+M / Alt+F: mirror / flip\nF11: full screen · Escape: leave full screen");
            ui.label("F: fit · R: rotate · 1: 100%\nArrows: pan · Page Up/Down: Z\nRight-drag: zoom to area\nEscape: pan tool");
        });
    }

    pub(super) fn canvas_tools(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) {
        self.features.canvas = Some(rect);
        if self.features.tool == Tool::Pan
            && response.clicked_by(egui::PointerButton::Primary)
            && !response.double_clicked()
        {
            self.features.edits.end_group();
            self.features.selected = response
                .interact_pointer_pos()
                .and_then(|p| self.pick_annotation(p, rect));
        }
        response.context_menu(|ui| {
            if ui.button("Fit slide").clicked() {
                self.fit = true;
                ui.close_menu();
            }
            ui.separator();
            let previous_tool = self.features.tool;
            for (tool, label) in [
                (Tool::Pan, "Pan"),
                (Tool::Line, "Measure line"),
                (Tool::Pointer, "Draw pointer"),
                (Tool::Rectangle, "Draw box"),
                (Tool::Circle, "Draw circle"),
                (Tool::Polygon, "Draw polygon"),
                (Tool::Freehand, "Draw freehand region"),
            ] {
                if ui
                    .selectable_value(&mut self.features.tool, tool, label)
                    .clicked()
                {
                    ui.close_menu();
                }
            }
            if previous_tool != self.features.tool {
                self.features.cancel_gesture();
            }
            ui.separator();
            ui.checkbox(&mut self.features.show_annotations, "Show annotations");
            ui.checkbox(&mut self.features.show_labels, "Show measurement labels");
            ui.checkbox(&mut self.features.show_scale, "Show scale bar");
        });
        let Some(slide) = self.slide.clone() else {
            return;
        };
        let secondary = egui::PointerButton::Secondary;
        let primary = egui::PointerButton::Primary;
        self.polygon_input(ui, rect, response);
        self.freehand_input(ui, rect, response);
        let exact = self.features.exact_shape.enabled
            && matches!(self.features.tool, Tool::Circle | Tool::Rectangle);
        if exact {
            self.exact_shape_input(ui, rect, response, &slide);
        }
        let drawing = matches!(
            self.features.tool,
            Tool::Line | Tool::Pointer | Tool::Rectangle | Tool::Circle
        ) && self.shown_plane == Some(self.plane)
            && !exact;
        if self.shown_plane != Some(self.plane) {
            self.features.start = None;
        }
        let clamp = |v: egui::Vec2| {
            v.clamp(
                egui::Vec2::ZERO,
                egui::vec2(slide.width as f32, slide.height as f32),
            )
        };
        if response.drag_started_by(secondary) || (drawing && response.drag_started_by(primary)) {
            if let Some(p) = ui.ctx().input(|i| i.pointer.press_origin()) {
                self.features.start = Some(clamp(self.world_point(p, rect)));
            }
        }
        let pointer = response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().input(|i| i.pointer.hover_pos()));
        let mut label_rects = Vec::new();
        if let (Some(start), Some(p)) = (self.features.start, pointer) {
            let end = clamp(self.world_point(p, rect));
            let zoom_box = ui.ctx().input(|i| i.pointer.button_down(secondary))
                || response.drag_stopped_by(secondary);
            let tool = if zoom_box {
                Tool::Rectangle
            } else {
                self.features.tool
            };
            let points = if tool == Tool::Circle {
                circle::from_drag(start, end, &slide).to_vec()
            } else {
                vec![[start.x, start.y], [end.x, end.y]]
            };
            self.paint_annotation(
                ui,
                rect,
                &Annotation {
                    appearance: Default::default(),
                    tool,
                    points: points.clone(),
                    plane: self.plane,
                    note: String::new(),
                    origin: None,
                },
                egui::Color32::LIGHT_BLUE,
                &mut label_rects,
            );
            if response.drag_stopped_by(secondary) || response.drag_stopped_by(primary) {
                self.features.start = None;
                if (end - start).length() * self.zoom >= 4.0 {
                    if zoom_box {
                        self.center = (start + end) * 0.5;
                        let area = (end - start).abs().max(egui::Vec2::splat(1.0));
                        let size = self.world_view_size(rect.size());
                        self.zoom =
                            (0.95 * (size.x / area.x).min(size.y / area.y)).clamp(0.0001, 8.0);
                        self.changed();
                    } else if tool != Tool::Pan
                        && self.features.annotations.len() < 10000
                        && (tool != Tool::Circle || circle::valid(&points, slide.calibration))
                    {
                        self.features.show_annotations = true;
                        self.features.add_annotation(Annotation {
                            appearance: Default::default(),
                            tool,
                            points,
                            plane: self.plane,
                            note: String::new(),
                            origin: None,
                        });
                        self.features.selected = Some(self.features.annotations.len() - 1);
                        self.features.save(&slide);
                    }
                }
            }
        }
        for (index, original) in self.features.annotations.iter().enumerate() {
            let a = self
                .features
                .geometry_gesture
                .as_ref()
                .filter(|_| self.features.selected == Some(index))
                .map_or(original, |g| &g.preview);
            if self.features.show_annotations && Some(a.plane) == self.shown_plane {
                self.paint_annotation(
                    ui,
                    rect,
                    a,
                    egui::Color32::from_rgb(
                        a.appearance.color[0],
                        a.appearance.color[1],
                        a.appearance.color[2],
                    ),
                    &mut label_rects,
                );
            }
        }
        self.paint_scale_bar(ui, rect, &slide);
        self.paint_edit_handles(ui, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotation() -> Annotation {
        Annotation {
            appearance: Default::default(),
            tool: Tool::Rectangle,
            points: vec![[10.0, 20.0], [60.0, 70.0]],
            plane: 2,
            note: "Saved μm".into(),
            origin: None,
        }
    }

    #[test]
    fn saves_ndpa_and_preserves_external_or_corrupt_files() {
        let root = std::env::temp_dir().join(format!("ndpa-storage-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut slide = ndpa::test_slide();
        slide.path = root.join("sample.ndpi");
        // Obsolete sidecars must be ignored and never migrated or overwritten.
        let obsolete = root.join("sample.ndpi.annotations.json");
        std::fs::write(&obsolete, b"obsolete file").unwrap();
        let mut state = Features::default();
        state.load_annotations(&slide);
        assert!(state.annotations.is_empty());
        let path = annotation_path(&slide);
        assert!(!path.exists());
        state.annotations.push(annotation());
        state.save(&slide);
        assert!(state.error.is_none(), "{:?}", state.error);
        assert_eq!(std::fs::read(&obsolete).unwrap(), b"obsolete file");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("<z>1000</z>"));
        let mut loaded = Features::default();
        loaded.load_annotations(&slide);
        assert_eq!(loaded.annotations[0].note, "Saved μm");
        assert_eq!(loaded.annotations[0].plane, 2);
        loaded.annotations[0].note = "Edited note".into();
        loaded.save(&slide);
        assert!(loaded.error.is_none());
        let mut reopened = Features::default();
        reopened.load_annotations(&slide);
        assert_eq!(reopened.annotations[0].note, "Edited note");
        let original = std::fs::read(&path).unwrap();
        let origin = loaded.annotations[0].origin;
        loaded.remove_annotation(0);
        loaded.save(&slide);
        assert!(loaded.error.is_none());
        let mut deleted = Features::default();
        deleted.load_annotations(&slide);
        assert!(deleted.annotations.is_empty());
        loaded.restore_annotation();
        assert_eq!(loaded.annotations[0].origin, origin);
        loaded.save(&slide);
        assert!(loaded.error.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let external = b"<annotations><!--external edit--></annotations>";
        std::fs::write(&path, external).unwrap();
        loaded.save(&slide);
        assert!(!loaded.writable);
        assert_eq!(std::fs::read(&path).unwrap(), external);
        std::fs::write(&path, b"invalid XML").unwrap();
        let mut invalid = Features::default();
        invalid.load_annotations(&slide);
        assert!(!invalid.writable);
        invalid.save(&slide);
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid XML");
        assert!(invalid.annotations.is_empty());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(obsolete).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn undo_redo_preserves_imported_ndpa_through_file_saves() {
        let root = std::env::temp_dir().join(format!("ndpa-undo-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut slide = ndpa::test_slide();
        slide.path = root.join("history.ndpi");
        let path = annotation_path(&slide);
        std::fs::write(&path, ndpa::write_new(&[annotation()], &slide).unwrap()).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut state = Features::default();
        state.load_annotations(&slide);
        let before = state.annotations[0].clone();
        state.annotations[0].appearance.title = "Epithelium".into();
        state.annotations[0].appearance.color = [12, 80, 190];
        state.annotations[0].note = "Region of interest".into();
        state.edits.edited(0, before, state.annotations[0].clone());
        state.save(&slide);
        assert!(state.error.is_none());
        let edited = std::fs::read(&path).unwrap();
        assert_ne!(edited, original);
        state.remove_annotation(0);
        state.save(&slide);
        state.undo_edit();
        state.save(&slide);
        assert_eq!(std::fs::read(&path).unwrap(), edited);
        state.undo_edit();
        state.save(&slide);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        state.redo_edit();
        state.save(&slide);
        assert_eq!(std::fs::read(&path).unwrap(), edited);
        let mut reopened = Features::default();
        reopened.load_annotations(&slide);
        assert_eq!(reopened.annotations[0].appearance.title, "Epithelium");
        assert_eq!(reopened.annotations[0].appearance.color, [12, 80, 190]);
        assert_eq!(reopened.annotations[0].plane, 2);
        assert!(!reopened.edits.can_undo());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn missing_metadata_never_creates_a_sidecar() {
        let root = std::env::temp_dir().join(format!("ndpa-no-metadata-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut slide = ndpa::test_slide();
        slide.path = root.clone();
        slide.ndpa = None;
        let mut state = Features::default();
        state.load_annotations(&slide);
        assert!(!state.writable);
        assert!(state.status.as_ref().unwrap().contains("session-only"));
        state.annotations.push(annotation());
        state.save(&slide);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        // Saving without a prior load must also reject incomplete metadata.
        let mut direct = Features::default();
        direct.annotations.push(annotation());
        direct.save(&slide);
        assert!(direct.error.is_some());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn rotations_round_trip_and_annotation_validation() {
        for turns in 0..4 {
            let p = egui::vec2(123.5, -37.25);
            assert_eq!(rotate(rotate(p, turns), (4 - turns) % 4), p);
        }
        let slide = ndpa::test_slide();
        let mut annotations = vec![annotation()];
        assert!(validate(&annotations, &slide));
        annotations[0].plane = slide.planes;
        assert!(!validate(&annotations, &slide));
        annotations[0].plane = 0;
        annotations[0].points[0][0] = f32::NAN;
        assert!(!validate(&annotations, &slide));
    }

    #[test]
    fn picking_respects_orientation_topmost_shape_visibility_and_loaded_z() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.zoom = 0.5;
        viewer.center = egui::vec2(150.0, 150.0);
        viewer.plane = 0;
        viewer.shown_plane = Some(0);
        let rect = egui::Rect::from_min_size(egui::pos2(320.0, 50.0), egui::vec2(800.0, 600.0));
        let shape = Annotation {
            appearance: Default::default(),
            tool: Tool::Rectangle,
            points: vec![[100.0, 100.0], [200.0, 200.0]],
            plane: 0,
            note: String::new(),
            origin: None,
        };
        viewer.features.annotations = vec![shape.clone(), shape];
        for rotation in 0..4 {
            for mirrored in [[false, false], [true, false], [false, true], [true, true]] {
                viewer.features.rotation = rotation;
                viewer.features.mirrored = mirrored;
                let pointer = viewer.screen_point(egui::vec2(120.0, 180.0), rect);
                assert_eq!(viewer.pick_annotation(pointer, rect), Some(1));
                viewer.features.annotations[1].plane = 1;
                assert_eq!(viewer.pick_annotation(pointer, rect), Some(0));
                viewer.shown_plane = Some(1);
                assert_eq!(viewer.pick_annotation(pointer, rect), None);
                viewer.shown_plane = Some(0);
                viewer.features.show_annotations = false;
                assert_eq!(viewer.pick_annotation(pointer, rect), None);
                viewer.features.show_annotations = true;
                viewer.features.annotations[1].plane = 0;
            }
        }
    }

    #[test]
    fn fitting_an_annotation_preserves_source_geometry_and_selects_its_plane() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.viewport_size = egui::vec2(800.0, 600.0);
        viewer.features.annotations.push(Annotation {
            appearance: Default::default(),
            tool: Tool::Polygon,
            points: vec![
                [1000.0, 2000.0],
                [3000.0, 2000.0],
                [3000.0, 5000.0],
                [1000.0, 4000.0],
            ],
            plane: 0,
            note: "Target".into(),
            origin: Some(3),
        });
        for rotation in 0..4 {
            for mirrored in [[false, false], [true, false], [false, true], [true, true]] {
                viewer.features.rotation = rotation;
                viewer.features.mirrored = mirrored;
                viewer.visit_annotation(0, true);
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, viewer.viewport_size);
                for p in &viewer.features.annotations[0].points {
                    assert!(rect.contains(viewer.screen_point(egui::vec2(p[0], p[1]), rect)));
                }
                assert_eq!(viewer.center, egui::vec2(2000.0, 3500.0));
                assert_eq!(viewer.features.annotations[0].origin, Some(3));
                assert_eq!(viewer.features.selected, Some(0));
                assert!(!viewer.fit);
            }
        }
        let center = viewer.center;
        viewer.visit_annotation(99, true);
        assert_eq!(viewer.center, center);
    }
}
