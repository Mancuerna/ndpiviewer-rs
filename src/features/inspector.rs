//! Task-based inspector: persistent navigation and tools, independently scrolled content.
use super::Tool;
use crate::Viewer;
use eframe::egui;

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) enum Tab {
    #[default]
    Annotations,
    Channels,
    Tracking,
    Export,
}

impl Viewer {
    pub(crate) fn prefer_channel_controls(&mut self) {
        self.features.inspector = Tab::Channels;
    }

    pub(crate) fn inspector_panel(&mut self, ui: &mut egui::Ui) {
        let Some(slide) = self.slide.clone() else {
            ui.heading("Start viewing");
            ui.label("Open an NDPI or NDPIS image using Open slide above.");
            ui.label("For an image sequence, choose Open Z stack.");
            return;
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(slide.path.file_name().unwrap_or_default().to_string_lossy())
                    .strong(),
            )
            .truncate(),
        )
        .on_hover_text(slide.path.display().to_string());
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Focus Z");
            ui.label(format!("{} / {}", self.plane + 1, slide.planes));
            if let Some(z) = slide.focus_microns(self.plane) {
                ui.label(format!("{z:+.3} µm"))
                    .on_hover_text("Scanner focus coordinate, not tissue depth.");
            }
        });
        let mut plane = self.plane + 1;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(plane > 1, egui::Button::new("− [Z]"))
                .on_hover_text("Previous focus plane (Z / Page Up)")
                .clicked()
            {
                plane -= 1;
            }
            ui.add_enabled(
                slide.planes > 1,
                egui::Slider::new(&mut plane, 1..=slide.planes).show_value(false),
            );
            if ui
                .add_enabled(plane < slide.planes, egui::Button::new("+ [X]"))
                .on_hover_text("Next focus plane (X / Page Down)")
                .clicked()
            {
                plane += 1;
            }
            if let Some(reference) = slide.reference_plane() {
                if ui
                    .small_button("Ref")
                    .on_hover_text("Go to the recorded plane closest to zero.")
                    .clicked()
                {
                    plane = reference + 1;
                }
            }
        });
        if self.plane != plane - 1 {
            self.features.cancel_gesture();
            self.plane = plane - 1;
            self.changed();
        }
        if self.shown_plane.is_some_and(|shown| shown != self.plane) {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!(
                    "Loading Z {} · showing Z {}",
                    self.plane + 1,
                    self.shown_plane.unwrap() + 1
                ),
            );
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Tools");
            if ui
                .add_enabled(self.features.edits.can_undo(), egui::Button::new("Undo"))
                .on_hover_text("Undo annotation change (Ctrl+Z)")
                .clicked()
            {
                self.features.undo_edit();
                self.features.save(&slide);
            }
            if ui
                .add_enabled(self.features.edits.can_redo(), egui::Button::new("Redo"))
                .on_hover_text("Redo annotation change (Ctrl+Y)")
                .clicked()
            {
                self.features.redo_edit();
                self.features.save(&slide);
            }
        });
        let previous = self.features.tool;
        ui.horizontal_wrapped(|ui| {
            for (tool, label, hint) in [
                (
                    Tool::Pan,
                    "Pan [Q]",
                    "Drag to move. Click a shape to select it. Escape returns to this tool.",
                ),
                (
                    Tool::Line,
                    "Length [W]",
                    "Drag between two points to measure length.",
                ),
                (
                    Tool::Pointer,
                    "Arrow [A]",
                    "Drag from the point of interest toward the arrow tail.",
                ),
                (
                    Tool::Rectangle,
                    "Box [E]",
                    "Drag a rectangle, or enable Exact size to place identical fields.",
                ),
                (
                    Tool::Circle,
                    "Circle [S]",
                    "Drag from the center, or enable Exact size to place identical fields.",
                ),
                (
                    Tool::Polygon,
                    "Polygon [C]",
                    "Click vertices; double-click or Enter to finish.",
                ),
                (
                    Tool::Freehand,
                    "Freehand [D]",
                    "Draw an outline; release to close it.",
                ),
            ] {
                ui.selectable_value(&mut self.features.tool, tool, label)
                    .on_hover_text(hint);
            }
        });
        if previous != self.features.tool {
            self.features.cancel_gesture();
            if self.features.tool != Tool::Pan {
                self.features.inspector = Tab::Annotations;
            }
        }
        self.exact_shape_controls(ui, &slide);
        if let Some(error) = &self.features.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        } else if !self.features.writable {
            ui.colored_label(egui::Color32::YELLOW, "Annotations: session only")
                .on_hover_text("Physical metadata required for NDPA saving is unavailable.");
        } else {
            ui.small("Annotations auto-save to NDPA");
            if let Some(status) = &self.features.status {
                if status.contains("unsupported") {
                    ui.colored_label(egui::Color32::YELLOW, status);
                }
            }
        }
        if let Some(error) = &self.channels.persistence.message {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.spacing_mut().button_padding = egui::vec2(5.0, 4.0);
            for (tab, label) in [
                (Tab::Annotations, "Annotate"),
                (Tab::Channels, "Channels"),
                (Tab::Tracking, "Tracking"),
                (Tab::Export, "Export"),
            ] {
                ui.selectable_value(&mut self.features.inspector, tab, label);
            }
        });
        ui.separator();
        let tab = self.features.inspector;
        egui::ScrollArea::vertical()
            .id_salt(("inspector", tab))
            .auto_shrink([false, false])
            .show(ui, |ui| match tab {
                Tab::Annotations => self.annotation_panel(ui, &slide),
                Tab::Channels => self.channel_panel(ui),
                Tab::Tracking => self.navigator_panel(ui, &slide),
                Tab::Export => self.export_panel(ui, &slide),
            });
    }
}
