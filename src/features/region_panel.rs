use super::*;
use std::sync::atomic::Ordering;

impl Viewer {
    pub(super) fn region_export_panel(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        ui.strong("Native-resolution region");
        ui.small("PNG at one output pixel per source pixel, in the original slide orientation. Uses the current Z plane and channel adjustments (8-bit display colors).");
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(self.region_export.selecting, "Select region")
                .clicked()
            {
                self.features.cancel_gesture();
                self.features.tool = Tool::Pan;
                self.region_export.selecting = !self.region_export.selecting;
            }
            if ui.button("Use visible area").clicked() {
                let rect = egui::Rect::from_center_size(
                    self.center.to_pos2(),
                    self.world_view_size(self.viewport_size) / self.zoom,
                );
                self.region_export.area = crate::region_export::bounds(rect, slide);
                self.region_export.selecting = false;
            }
            if ui.button("Clear region").clicked() {
                self.region_export.reset_selection();
            }
        });
        if self.region_export.selecting {
            ui.label("Drag a rectangle on the image. Escape cancels selection.");
        }
        let size_ok = self
            .region_export
            .area
            .is_some_and(|[_, _, w, h]| u64::from(w) * u64::from(h) <= 16_777_216);
        if let Some([x, y, w, h]) = self.region_export.area {
            ui.label(format!(
                "X {x}, Y {y} · {w} × {h} px · Z {}",
                self.plane + 1
            ));
            if !size_ok {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "Region exceeds 16 megapixels. Select a smaller area.",
                );
            }
        }
        ui.checkbox(
            &mut self.region_export.annotations,
            "Include annotations on this Z plane",
        );
        ui.checkbox(&mut self.region_export.labels, "Include annotation labels");
        ui.add_enabled(
            slide.calibration.is_some(),
            egui::Checkbox::new(&mut self.region_export.scale, "Include scale bar"),
        );
        ui.small("Output buffer: at most 64 MiB, plus decoder and encoding memory.");
        if ui
            .add_enabled(
                size_ok && self.region_export.task.is_none(),
                egui::Button::new("Export region PNG…"),
            )
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name("region.png")
                .save_file()
            {
                let annotations = if self.region_export.annotations {
                    self.features
                        .annotations
                        .iter()
                        .filter(|a| a.plane == self.plane)
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                self.region_export.start(
                    ui.ctx(),
                    crate::region_export::Job {
                        slide: slide.clone(),
                        area: self.region_export.area.unwrap(),
                        plane: self.plane,
                        settings: self.channels.settings.clone(),
                        annotations,
                        scale: self.region_export.scale && slide.calibration.is_some(),
                        labels: self.region_export.labels,
                        path,
                    },
                );
            }
        }
        if self.region_export.task.is_some() {
            ui.add(
                egui::ProgressBar::new(
                    self.region_export.progress.load(Ordering::Relaxed) as f32 / 100.0,
                )
                .show_percentage(),
            );
            if ui.button("Cancel region export").clicked() {
                self.region_export.cancel.store(true, Ordering::Relaxed);
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(message) = &self.region_export.message {
            ui.label(message);
        }
    }
}
