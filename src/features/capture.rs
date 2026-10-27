//! Viewport capture and clipboard output, separate from annotation editing.
use super::CaptureTarget;
use crate::Viewer;
use eframe::egui;

fn crop_view(
    image: &egui::ColorImage,
    rect: egui::Rect,
    screen: egui::Rect,
) -> Result<image::RgbaImage, String> {
    if screen.width() <= 0.0
        || screen.height() <= 0.0
        || !screen.is_finite()
        || !rect.is_finite()
        || image.size.contains(&0)
        || image.size[0].checked_mul(image.size[1]) != Some(image.pixels.len())
    {
        return Err("Invalid capture dimensions.".into());
    }
    let rect = rect.intersect(screen);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return Err("The active view is outside the capture.".into());
    }
    let scale = egui::vec2(
        image.size[0] as f32 / screen.width(),
        image.size[1] as f32 / screen.height(),
    );
    let lower = ((rect.min - screen.min) * scale).round();
    let upper = ((rect.max - screen.min) * scale).round();
    let x = (lower.x.max(0.0) as usize).min(image.size[0]);
    let y = (lower.y.max(0.0) as usize).min(image.size[1]);
    let right = (upper.x.max(0.0) as usize).min(image.size[0]);
    let bottom = (upper.y.max(0.0) as usize).min(image.size[1]);
    let width = right.saturating_sub(x);
    let height = bottom.saturating_sub(y);
    if width == 0 || height == 0 || width > u32::MAX as usize || height > u32::MAX as usize {
        return Err("The active view has no exportable pixels.".into());
    }
    // Copy only the selected pane, not an extra full-window RGBA buffer.
    let mut crop = image::RgbaImage::new(width as u32, height as u32);
    for (row, output) in crop.as_mut().chunks_exact_mut(width * 4).enumerate() {
        let start = (y + row) * image.size[0] + x;
        for (pixel, color) in output
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&image.pixels[start..start + width])
        {
            pixel.copy_from_slice(&color.to_srgba_unmultiplied());
        }
    }
    Ok(crop)
}

fn encode_view<W: std::io::Write + std::io::Seek>(
    crop: &image::RgbaImage,
    output: &mut W,
    format: image::ImageFormat,
) -> Result<(), image::ImageError> {
    if format == image::ImageFormat::Jpeg {
        let rgb = image::RgbImage::from_fn(crop.width(), crop.height(), |x, y| {
            let p = crop.get_pixel(x, y);
            image::Rgb([p[0], p[1], p[2]])
        });
        image::codecs::jpeg::JpegEncoder::new_with_quality(output, 95).encode_image(&rgb)
    } else {
        crop.write_to(output, format)
    }
}

fn save_view(
    crop: &image::RgbaImage,
    path: &std::path::Path,
    format: image::ImageFormat,
) -> Result<(), String> {
    crate::export_file::write(path, |output| {
        encode_view(crop, output, format).map_err(|e| e.to_string())
    })
}

impl Viewer {
    pub(super) fn capture_ready(&self) -> bool {
        self.slide.is_some()
            && !self.features.info_open
            && !self.associated.open
            && self.channels.dirty_at.is_none()
            && self.channels.fallback.is_none()
            && self.shown_plane == Some(self.plane)
            && self
                .visible_tiles(self.viewport_size)
                .iter()
                .all(|r| self.cached(r))
    }

    pub(super) fn request_capture(&mut self, ctx: &egui::Context, target: CaptureTarget) {
        if !self.capture_ready() {
            self.status =
                "Wait for the active view to finish loading before copying or exporting.".into();
            return;
        }
        if let Some(rect) = self.features.canvas {
            self.features.export = Some((target, rect));
            self.features.capture_queued = true;
            ctx.request_repaint();
        }
    }

    pub(crate) fn capture_pending(&self) -> bool {
        self.features.export.is_some()
    }

    pub(crate) fn handle_export(&mut self, ctx: &egui::Context) {
        // Defer one frame so the overlay inspector is absent from the screenshot.
        if std::mem::take(&mut self.features.capture_queued) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        // Screenshot exports are viewport snapshots, not full-resolution crops.
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot {
            if let Some((target, rect)) = self.features.export.take() {
                match crop_view(&image, rect, ctx.screen_rect()) {
                    Ok(crop) => match target {
                        CaptureTarget::File(path, format) => {
                            match save_view(&crop, &path, format) {
                                Ok(()) => {
                                    self.status = format!("View exported: {}", path.display())
                                }
                                Err(e) => {
                                    crate::diagnostics::record(
                                        crate::diagnostics::Event::ViewExportFailed,
                                    );
                                    self.features.error = Some(format!("Export failed: {e}"));
                                }
                            }
                        }
                        CaptureTarget::Clipboard => {
                            ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied(
                                [crop.width() as usize, crop.height() as usize],
                                crop.as_raw(),
                            ));
                            self.status = "View sent to the clipboard.".into();
                        }
                    },
                    Err(error) => {
                        crate::diagnostics::record(crate::diagnostics::Event::ViewExportFailed);
                        self.features.error = Some(format!("Export failed: {error}"));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_crop_accounts_for_display_scale_origin_and_clipping() {
        let pixels = (0..32)
            .map(|n| egui::Color32::from_rgb(n, 50, 100))
            .collect();
        let image = egui::ColorImage {
            size: [8, 4],
            pixels,
        };
        let screen = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(4.0, 2.0));
        let pane = egui::Rect::from_min_max(egui::pos2(12.0, 20.0), screen.max);
        let crop = crop_view(&image, pane, screen).unwrap();
        assert_eq!(crop.dimensions(), (4, 4));
        assert_eq!(crop.get_pixel(0, 0).0, [4, 50, 100, 255]);
        assert_eq!(crop.get_pixel(3, 3).0, [31, 50, 100, 255]);
        let clipped = crop_view(&image, screen.expand(100.0), screen).unwrap();
        assert_eq!(clipped.dimensions(), (8, 4));
        assert!(crop_view(&image, pane.translate(egui::vec2(50.0, 0.0)), screen).is_err());
        assert!(crop_view(&image, pane, egui::Rect::NOTHING).is_err());
        let invalid = egui::ColorImage {
            size: [8, 4],
            pixels: Vec::new(),
        };
        assert!(crop_view(&invalid, pane, screen).is_err());
    }

    #[test]
    fn png_and_jpeg_encode_real_decodable_viewport_pixels() {
        let crop = image::RgbaImage::from_pixel(32, 24, image::Rgba([20, 80, 150, 255]));
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let mut output = std::io::Cursor::new(Vec::new());
            encode_view(&crop, &mut output, format).unwrap();
            let decoded = image::load_from_memory_with_format(output.get_ref(), format)
                .unwrap()
                .to_rgb8();
            assert_eq!(decoded.dimensions(), crop.dimensions());
            let pixel = decoded.get_pixel(15, 12);
            let tolerance = if format == image::ImageFormat::Png {
                0
            } else {
                2
            };
            for (actual, expected) in pixel.0.into_iter().zip([20u8, 80, 150]) {
                assert!(actual.abs_diff(expected) <= tolerance);
            }
        }
    }
}
