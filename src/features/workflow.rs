//! Manual review aids. Cell marks remain ordinary, interoperable NDPA pointers.
use super::*;

pub(super) struct State {
    pub locked: bool,
    pub current_plane: bool,
    pub kind: String,
    pub counting: bool,
    pub hide_marks: bool,
    pub help: bool,
    category: usize,
    names: [String; 3],
    loupe_return: Option<(egui::Vec2, f32)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            locked: false,
            current_plane: false,
            kind: "All types".into(),
            counting: false,
            hide_marks: false,
            help: false,
            category: 0,
            names: [
                "Category A".into(),
                "Category B".into(),
                "Category C".into(),
            ],
            loupe_return: None,
        }
    }
}
pub(super) fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Line => "Line",
        Tool::Rectangle => "Rectangle",
        Tool::Circle => "Circle",
        Tool::Pointer => "Pointer",
        Tool::Polygon => "Polygon",
        _ => "Other",
    }
}
/// Serpentine traversal in source coordinates, including a final edge-aligned field.
fn field_center(
    dimensions: egui::Vec2,
    field: egui::Vec2,
    overlap: f32,
    current: egui::Vec2,
    forward: bool,
) -> egui::Vec2 {
    let field = field.min(dimensions).max(egui::Vec2::splat(1.0));
    let step = field * (1.0 - overlap / 100.0);
    let counts = ((dimensions - field) / step).ceil() + egui::Vec2::splat(1.0);
    let cols = counts.x as i64;
    let rows = counts.y as i64;
    let mut cell = ((current - field * 0.5) / step)
        .round()
        .max(egui::Vec2::ZERO)
        .min(counts - egui::Vec2::splat(1.0));
    for axis in 0..2 {
        if current[axis] >= dimensions[axis] - field[axis] * 0.5 - 0.01 {
            cell[axis] = counts[axis] - 1.0;
        }
    }
    let y = cell.y as i64;
    let x = cell.x as i64;
    let index = y * cols + if y % 2 == 0 { x } else { cols - 1 - x };
    let index = (index + if forward { 1 } else { -1 }).clamp(0, cols * rows - 1);
    let row = index / cols;
    let col = if row % 2 == 0 {
        index % cols
    } else {
        cols - 1 - index % cols
    };
    (field * 0.5 + egui::vec2(col as f32, row as f32) * step).min(dimensions - field * 0.5)
}
impl Features {
    pub(crate) fn workflow_counting(&self) -> bool {
        self.workflow.counting
    }
}
impl Viewer {
    pub(crate) fn step_field(&mut self, forward: bool) {
        let Some(slide) = &self.slide else { return };
        self.center = field_center(
            egui::vec2(slide.width as f32, slide.height as f32),
            self.world_view_size(self.viewport_size) / self.zoom,
            self.preferences.field_overlap,
            self.center,
            forward,
        );
        self.fit = false;
        self.features.cancel_gesture();
        self.changed();
    }
    pub(crate) fn workflow_input(&mut self, ctx: &egui::Context) {
        let enabled = !self.chrome.settings_open
            && !ctx.wants_keyboard_input()
            && !ctx.memory(|m| m.any_popup_open());
        self.features.workflow.hide_marks =
            enabled && ctx.input(|i| i.modifiers.is_none() && i.key_down(egui::Key::T));
        let loupe = enabled && ctx.input(|i| i.modifiers.is_none() && i.key_down(egui::Key::L));
        if loupe && self.features.workflow.loupe_return.is_none() && self.slide.is_some() {
            self.features.workflow.loupe_return = Some((self.center, self.zoom));
            if let (Some(rect), Some(pointer)) =
                (self.features.canvas, ctx.input(|i| i.pointer.hover_pos()))
            {
                if rect.contains(pointer) {
                    self.center = self.world_point(pointer, rect);
                }
            }
            self.zoom = (self.zoom * 2.0).min(8.0);
            self.fit = false;
            self.changed();
        } else if !loupe {
            if let Some((center, zoom)) = self.features.workflow.loupe_return.take() {
                self.center = center;
                self.zoom = zoom;
                self.changed();
            }
        }
        let mut open = self.features.workflow.help;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .vscroll(true)
            .max_height(550.0)
            .show(ctx, |ui| {
                for action in crate::keybindings::ACTIONS {
                    ui.label(format!(
                        "{} — {}",
                        self.preferences.bindings.get(action).label(),
                        self.preferences.action_label(action)
                    ));
                }
                for text in [
                    "N / P — next / previous field",
                    "Hold T — hide annotations and tracking",
                    "Hold L — temporary 2× magnification",
                    "Hold Space — pan",
                    "Ctrl+Z / Ctrl+Y — undo / redo",
                    "Ctrl+wheel — focus Z",
                    "F11 — full screen",
                    "? — this shortcut sheet",
                ] {
                    ui.label(text);
                }
            });
        self.features.workflow.help = open;
    }
    pub(super) fn workflow_panel(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        ui.checkbox(
            &mut self.features.workflow.locked,
            "Lock annotation geometry and deletion (this session)",
        );
        ui.collapsing("Field review and cell counter",|ui| {
            ui.horizontal(|ui| {
                if ui.button("Previous field [P]").clicked() { self.step_field(false); }
                if ui.button("Next field [N]").clicked() { self.step_field(true); }
            });
            if ui.button("Start at top-left field").clicked() { self.center=(self.world_view_size(self.viewport_size)/self.zoom).min(egui::vec2(slide.width as f32,slide.height as f32))*0.5; self.fit=false; self.changed(); }
            ui.small("Serpentine review in slide coordinates; overlap is configurable in Settings.");
            if ui.checkbox(&mut self.features.workflow.counting,"Count cells with clicks").changed() { self.features.tool=Tool::Pan; self.features.cancel_gesture(); }
            for index in 0..3 {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.features.workflow.category,index,format!("{}",index+1));
                    ui.add(egui::TextEdit::singleline(&mut self.features.workflow.names[index]).char_limit(80).desired_width(125.0));
                    let prefix=format!("Cell {}: ",index+1);
                    let count=self.features.annotations.iter().filter(|a|a.appearance.title.starts_with(&prefix)).count();
                    ui.label(format!("{count}"));
                });
            }
            ui.small("Counts cover all Z planes. Marks are saved as NDPA pointers; Ctrl+Z undoes the last click. Renaming a category affects new marks.");
        });
    }
    pub(crate) fn counting_canvas(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) {
        if !self.features.workflow.counting
            || self.features.workflow.hide_marks
            || self.shown_plane != Some(self.plane)
            || !response.clicked()
            || self.temporary_pan(ui.ctx())
            || self.features.annotations.len() >= 10000
        {
            return;
        }
        let Some(slide) = self.slide.clone() else {
            return;
        };
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let point = self.world_point(pos, rect);
        if point.x < 0.0
            || point.y < 0.0
            || point.x > slide.width as f32
            || point.y > slide.height as f32
        {
            return;
        }
        let category = self.features.workflow.category;
        let prefix = format!("Cell {}: ", category + 1);
        let number = self
            .features
            .annotations
            .iter()
            .filter(|a| a.tool == Tool::Pointer)
            .filter_map(|a| {
                a.appearance
                    .title
                    .strip_prefix(&prefix)
                    .and_then(|v| v.parse::<usize>().ok())
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let offset = (8.0 / self.zoom).max(1.0);
        let end = (point + egui::Vec2::splat(offset))
            .min(egui::vec2(slide.width as f32, slide.height as f32));
        self.features.add_annotation(Annotation {
            tool: Tool::Pointer,
            points: vec![point.into(), end.into()],
            plane: self.plane,
            note: self.features.workflow.names[category].clone(),
            origin: None,
            appearance: crate::annotations::Appearance {
                title: format!("{prefix}{number}"),
                color: [[100, 220, 255], [255, 140, 130], [170, 230, 130]][category],
            },
        });
        self.features.save(&slide);
    }
    pub(super) fn duplicate_annotation(&mut self, index: usize, slide: &Slide) {
        if self.features.annotations.len() >= 10000 {
            return;
        }
        let mut a = self.features.annotations[index].clone();
        let [lo, hi] = polygon::bounds(&a.points);
        let size = egui::Vec2::from(hi) - egui::Vec2::from(lo);
        let dimensions = egui::vec2(slide.width as f32, slide.height as f32);
        if size.x > dimensions.x || size.y > dimensions.y {
            self.status = "Annotation is larger than the slide.".into();
            return;
        }
        let center = self.center.clamp(size * 0.5, dimensions - size * 0.5);
        let delta = center - (egui::Vec2::from(lo) + egui::Vec2::from(hi)) * 0.5;
        for p in &mut a.points {
            *p = (egui::Vec2::from(*p) + delta).into();
        }
        a.origin = None;
        a.plane = self.plane;
        a.appearance.title = format!("{} (copy)", a.appearance.title);
        self.features.add_annotation(a);
        self.features.save(slide);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cell_marks_roundtrip_through_standard_ndpa() {
        let slide = crate::ndpa::test_slide();
        let mark = Annotation {
            tool: Tool::Pointer,
            points: vec![[10.0, 20.0], [18.0, 28.0]],
            plane: 1,
            note: "Category A".into(),
            origin: None,
            appearance: crate::annotations::Appearance {
                title: "Cell 1: 7".into(),
                color: [100, 220, 255],
            },
        };
        let xml = crate::ndpa::write_new(std::slice::from_ref(&mark), &slide).unwrap();
        let (_, marks) = crate::ndpa::Archive::read(xml, &slide).unwrap();
        assert_eq!(marks.len(), 1);
        assert!(marks[0].tool == Tool::Pointer);
        assert_eq!(marks[0].appearance.title, mark.appearance.title);
        assert_eq!(marks[0].plane, 1);
        assert_eq!(marks[0].points, mark.points);
    }
    #[test]
    fn locked_geometry_cannot_be_deleted_or_undone_and_duplicate_preserves_size() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let slide = viewer.slide.clone().unwrap();
        viewer.features.add_annotation(Annotation {
            tool: Tool::Rectangle,
            points: vec![[10.0, 20.0], [40.0, 80.0]],
            plane: 0,
            note: String::new(),
            origin: None,
            appearance: Default::default(),
        });
        viewer.features.workflow.locked = true;
        viewer.features.remove_annotation(0);
        viewer.features.undo_edit();
        assert_eq!(viewer.features.annotations.len(), 1);
        viewer.center = egui::vec2(300.0, 400.0);
        viewer.duplicate_annotation(0, &slide);
        assert_eq!(viewer.features.annotations.len(), 2);
        let p = &viewer.features.annotations[1].points;
        assert_eq!(
            egui::Vec2::from(p[1]) - egui::Vec2::from(p[0]),
            egui::vec2(30.0, 60.0)
        );
    }
    #[test]
    fn arbitrary_rotation_screen_world_roundtrip_and_bounds() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.features.angle = 0.63;
        viewer.features.rotation = 1;
        viewer.features.mirrored = [true, false];
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let p = egui::vec2(400.0, 500.0);
        assert!((viewer.world_point(viewer.screen_point(p, rect), rect) - p).length() < 0.01);
        let bounds = viewer.tile_rect([100.0, 200.0, 300.0, 400.0], rect);
        for p in [
            [100.0, 200.0],
            [400.0, 200.0],
            [100.0, 600.0],
            [400.0, 600.0],
        ] {
            assert!(bounds
                .expand(0.01)
                .contains(viewer.screen_point(p.into(), rect)));
        }
    }
    #[test]
    fn narrow_final_field_advances_to_next_row() {
        let d = egui::vec2(210.0, 200.0);
        let f = egui::vec2(100.0, 100.0);
        assert_eq!(
            field_center(d, f, 0.0, egui::vec2(160.0, 50.0), true),
            egui::vec2(160.0, 150.0)
        );
    }
    #[test]
    fn traversal_reaches_edges_and_reverses_rows() {
        let d = egui::vec2(300.0, 200.0);
        let f = egui::vec2(100.0, 100.0);
        let mut c = f * 0.5;
        for expected in [
            [150., 50.],
            [250., 50.],
            [250., 150.],
            [150., 150.],
            [50., 150.],
            [50., 150.],
        ] {
            c = field_center(d, f, 0., c, true);
            assert_eq!(c, egui::Vec2::from(expected));
        }
        assert_eq!(field_center(d, f, 0., c, false), egui::vec2(150., 150.));
    }
}
