use super::*;
use std::fmt::Write;

impl Viewer {
    pub(super) fn slide_info(&mut self, ctx: &egui::Context) {
        if !self.features.info_open {
            return;
        }
        let Some(slide) = &self.slide else {
            return;
        };
        let mut details = format!(
            "File: {}\nDimensions: {} x {} pixels\nFocus planes: {}\nPyramid levels: {}\n",
            slide.path.display(),
            slide.width,
            slide.height,
            slide.planes,
            slide.levels.len()
        );
        if let Ok(metadata) = std::fs::metadata(&slide.path) {
            if metadata.is_file() {
                let _ = writeln!(
                    details,
                    "File size: {:.2} MiB ({} bytes)",
                    metadata.len() as f64 / 1048576.0,
                    metadata.len()
                );
            }
        }
        if let Some(c) = slide.calibration {
            let _ = writeln!(
                details,
                "Pixel spacing: X {:.6}, Y {:.6} µm/px\nImage extent: {:.3} x {:.3} mm",
                c.x,
                c.y,
                c.x * f64::from(slide.width) / 1000.0,
                c.y * f64::from(slide.height) / 1000.0
            );
        } else {
            details.push_str("Physical calibration: unavailable\n");
        }
        if let Some(g) = &slide.ndpa {
            let _ = writeln!(
                details,
                "Acquisition magnification: {}x\nNDPA slide offsets: X {}, Y {} nm",
                g.lens, g.offset[0], g.offset[1]
            );
        }
        details.push_str(if slide.rgb_components {
            "Channel model: RGB components\n"
        } else {
            "Channel model: independent acquisition channels\n"
        });
        for (index, channel) in slide.channels.iter().enumerate() {
            let _ = writeln!(details, "Channel {}: {}", index + 1, channel.name);
        }
        for plane in 0..slide.planes {
            if let Some(z) = slide.focus_microns(plane) {
                let _ = writeln!(
                    details,
                    "Focus plane {}: {z:+.3} µm (scanner coordinate)",
                    plane + 1
                );
            }
        }
        details.push_str("\nPyramid dimensions:\n");
        for (index, level) in slide.levels.iter().enumerate() {
            let _ = writeln!(
                details,
                "{}: {} x {} pixels",
                index + 1,
                level.width,
                level.height
            );
        }
        egui::Window::new("Slide information")
            .open(&mut self.features.info_open)
            .default_width(640.0)
            .show(ctx, |ui| {
                if ui.button("Copy details").clicked() {
                    ctx.copy_text(details.clone());
                }
                egui::ScrollArea::both().max_height(520.0).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&details).monospace().size(17.0))
                            .selectable(true),
                    );
                });
            });
    }
}
