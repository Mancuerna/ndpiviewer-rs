//! Macro/label and acquisition-map previews are separate from diagnostic pyramid tiles.
use crate::{background::Task, channels, reader, Region};
use eframe::egui;
use std::path::PathBuf;

struct Images {
    images: Vec<(String, egui::ColorImage)>,
    warnings: Vec<String>,
}

#[cfg(test)]
pub(crate) fn load(path: PathBuf) -> Result<Vec<(String, egui::ColorImage)>, String> {
    load_images(path).map(|loaded| loaded.images)
}

fn load_images(path: PathBuf) -> Result<Images, String> {
    load_images_with_options(path, false)
}

pub(crate) fn label_thumbnail(path: PathBuf) -> Result<Option<egui::ColorImage>, String> {
    load_images_with_options(path, true)
        .map(|images| images.images.into_iter().next().map(|(_, image)| image))
}

fn load_images_with_options(path: PathBuf, label_only: bool) -> Result<Images, String> {
    if path.is_dir() {
        return Err("Image folders do not contain embedded slide images.".into());
    }
    let ndpi = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ndpis"))
    {
        crate::ndpis::files(&path)?
            .into_iter()
            .next()
            .ok_or("Empty channel group")?
    } else {
        path
    };
    let mut decoder = bioformats::ImageReader::open(&ndpi).map_err(|e| e.to_string())?;
    let base = decoder.metadata().clone();
    let file = std::fs::File::open(&ndpi).map_err(|e| e.to_string())?;
    let large = file.metadata().map_err(|e| e.to_string())?.len() >= 1u64 << 32;
    let mut parser = bioformats::tiff::parser::TiffParser::new(std::io::BufReader::new(file))
        .map_err(|e| e.to_string())?;
    let ifds = if large {
        parser.read_ifds_ndpi64()
    } else {
        parser.read_ifds()
    }
    .map_err(|e| e.to_string())?;
    let mut images = Vec::new();
    let mut warnings = Vec::new();
    for series in 0..decoder.series_count() {
        if images.len() >= if label_only { 1 } else { 8 } {
            break;
        }
        decoder.set_series(series).map_err(|e| e.to_string())?;
        let m = decoder.metadata().clone();
        if m.size_x == 0 || m.size_y == 0 {
            continue;
        }
        let ifd = ifds.iter().find(|i| {
            i.image_width() == Some(m.size_x)
                && i.image_length() == Some(m.size_y)
                && i.samples_per_pixel() as u32 == m.size_c
        });
        let lens = ifd.and_then(|i| i.get_vec_f64(65421).first().copied());
        // Vendor identification takes precedence over dimensions: macro images
        // may have the same aspect ratio as a single-plane diagnostic image.
        let named = m.series_metadata.contains_key("ndpi.image_type");
        let aspect_error = ((m.size_x as f64 / m.size_y as f64)
            / (base.size_x as f64 / base.size_y.max(1) as f64)
            - 1.0)
            .abs();
        if !named
            && !matches!(lens, Some(-1.0 | -2.0))
            && (series == 0 || (m.size_z == base.size_z && aspect_error < 0.01))
        {
            continue;
        }
        let kind = m
            .series_metadata
            .get("ndpi.image_type")
            .map(|v| v.to_string().to_ascii_lowercase())
            .unwrap_or_default();
        if label_only && lens != Some(-1.0) && !kind.contains("label") && !kind.contains("macro") {
            continue;
        }
        let title = match lens {
            Some(-1.0) => "Glass slide / label (as captured)".to_string(),
            Some(-2.0) => "Acquisition map / mask".to_string(),
            _ => format!("Associated image {}", series + 1),
        };
        let limit = if label_only { 192.0 } else { 1024.0 };
        let factor = (limit / m.size_x.max(m.size_y) as f64).min(1.0);
        let region = Region {
            sampling: 1,
            preview: true,
            series,
            interleaved: ifd.is_some_and(|i| i.get(65426).is_some()) || m.is_interleaved,
            plane: 0,
            x: 0,
            y: 0,
            w: (m.size_x as f64 * factor).round().max(1.0) as u32,
            h: (m.size_y as f64 * factor).round().max(1.0) as u32,
            world: [0.0, 0.0, m.size_x as f32, m.size_y as f32],
        };
        let bands = if m.is_rgb && m.size_c == 3 {
            channels::rgb_channels()
        } else {
            vec![channels::Channel {
                name: "Intensity".into(),
                color: [255; 3],
            }]
        };
        let decoded = if m.is_indexed {
            Err("Indexed embedded images are not supported.".into())
        } else {
            reader::decode_display(&mut decoder, &region, &channels::defaults(&bands))
        };
        match decoded {
            Ok((pixels, _)) => {
                images.push((format!("{title} · {} × {} px", m.size_x, m.size_y), pixels))
            }
            Err(error) => warnings.push(format!("{title}: {error}")),
        }
    }
    Ok(Images { images, warnings })
}

type Loaded = (PathBuf, Result<Images, String>);
#[derive(Default)]
pub(crate) struct Associated {
    pub open: bool,
    path: Option<PathBuf>,
    task: Option<Task<Loaded>>,
    images: Vec<(String, egui::TextureHandle)>,
    error: Option<String>,
    warnings: Vec<String>,
}
impl Associated {
    pub fn show(&mut self, ctx: &egui::Context, path: Option<&PathBuf>) {
        if self.path.as_ref() != path {
            self.open = false;
            self.images.clear();
            self.error = None;
            self.warnings.clear();
            self.path = path.cloned();
        }
        if let Some((path, result)) = self.task.as_ref().and_then(Task::take) {
            self.task = None;
            if self.path.as_ref() == Some(&path) {
                match result {
                    Ok(loaded) => {
                        self.warnings = loaded.warnings;
                        if loaded.images.is_empty() {
                            self.error = Some(if self.warnings.is_empty() {
                                "No embedded glass-slide or label image was recognized in this file."
                            } else {
                                "Embedded images could not be decoded. See the details below."
                            }.into());
                        }
                        self.images = loaded
                            .images
                            .into_iter()
                            .map(|(name, image)| {
                                let texture = ctx.load_texture(
                                    name.clone(),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                );
                                (name, texture)
                            })
                            .collect();
                    }
                    Err(error) => {
                        crate::diagnostics::record(
                            crate::diagnostics::Event::AssociatedImageFailed,
                        );
                        self.error = Some(error);
                    }
                }
            }
        }
        if !self.open {
            return;
        }
        if self.images.is_empty() && self.error.is_none() && self.task.is_none() {
            if let Some(path) = path {
                let path = path.clone();
                self.task = Some(Task::spawn(ctx, move || {
                    let images = load_images(path.clone());
                    (path, images)
                }));
            }
        }
        egui::Window::new("Glass slide and label").open(&mut self.open).default_size([760.0, 580.0]).show(ctx, |ui| {
            ui.small("Embedded images from the file. The label is shown only if it was captured; it may be part of the glass-slide image. For NDPIS, these images come from its first channel.");
            if self.task.is_some() { ui.label("Loading embedded images…"); }
            if let Some(error) = &self.error { ui.label(error); }
            for warning in &self.warnings { ui.colored_label(egui::Color32::LIGHT_RED, warning); }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, texture) in &self.images {
                    ui.heading(name);
                    ui.add(egui::Image::new(texture).fit_to_exact_size(texture.size_vec2() * (ui.available_width()/texture.size_vec2().x).min(1.0)));
                    ui.separator();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires the public large cytology sample"]
    fn cytology_keeps_label_and_single_channel_mask() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images/02B0669241M.ndpi");
        let thumb = label_thumbnail(path.clone()).unwrap().unwrap();
        assert!(thumb.size.iter().all(|&n| n <= 192));
        assert!(thumb.pixels.iter().any(|p| p.r() > 100 && p.g() > 100));
        let result = load_images(path).unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.images.len(), 2);
        assert!(result.images[0].0.starts_with("Glass slide"));
        assert!(result.images[1].0.starts_with("Acquisition map"));
        assert!(result.images[0]
            .1
            .pixels
            .iter()
            .any(|p| p.r() > 100 && p.g() > 100));
    }
}
