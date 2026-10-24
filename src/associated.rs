//! Macro/label and acquisition-map previews are separate from diagnostic pyramid tiles.
use crate::{background::Task, channels, reader, Region};
use eframe::egui;
use std::path::PathBuf;

pub(crate) fn load(path: PathBuf) -> Result<Vec<(String, egui::ColorImage)>, String> {
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
    let slide = reader::inspect(&mut decoder, ndpi.clone())?;
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
    for series in 0..decoder.series_count() {
        if slide.levels.iter().any(|l| l.series == series) {
            continue;
        }
        if images.len() >= 8 {
            break;
        }
        decoder.set_series(series).map_err(|e| e.to_string())?;
        let m = decoder.metadata().clone();
        if m.size_x == 0 || m.size_y == 0 || m.is_indexed {
            continue;
        }
        let ifd = ifds
            .iter()
            .find(|i| i.image_width() == Some(m.size_x) && i.image_length() == Some(m.size_y));
        let lens = ifd.and_then(|i| i.get_vec_f64(65421).first().copied());
        let title = match lens {
            Some(-1.0) => "Glass slide / label (as captured)".to_string(),
            Some(-2.0) => "Acquisition map / mask".to_string(),
            _ => format!("Associated image {}", series + 1),
        };
        let factor = (1024.0 / m.size_x.max(m.size_y) as f64).min(1.0);
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
        let bands = if m.is_rgb {
            channels::rgb_channels()
        } else {
            vec![channels::Channel {
                name: "Intensity".into(),
                color: [255; 3],
            }]
        };
        let (pixels, _) =
            reader::decode_display(&mut decoder, &region, &channels::defaults(&bands))?;
        images.push((format!("{title} · {} × {} px", m.size_x, m.size_y), pixels));
    }
    Ok(images)
}

type Loaded = (PathBuf, Result<Vec<(String, egui::ColorImage)>, String>);
#[derive(Default)]
pub(crate) struct Associated {
    pub open: bool,
    path: Option<PathBuf>,
    task: Option<Task<Loaded>>,
    images: Vec<(String, egui::TextureHandle)>,
    error: Option<String>,
}
impl Associated {
    pub fn show(&mut self, ctx: &egui::Context, path: Option<&PathBuf>) {
        if self.path.as_ref() != path {
            self.open = false;
            self.images.clear();
            self.error = None;
            self.path = path.cloned();
        }
        if let Some((path, result)) = self.task.as_ref().and_then(Task::take) {
            self.task = None;
            if self.path.as_ref() == Some(&path) {
                match result {
                    Ok(images) => {
                        if images.is_empty() {
                            self.error =
                                Some("This file has no embedded slide or label image.".into());
                        }
                        self.images = images
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
                    let images = load(path.clone());
                    (path, images)
                }));
            }
        }
        egui::Window::new("Glass slide and label").open(&mut self.open).default_size([760.0, 580.0]).show(ctx, |ui| {
            ui.small("Embedded images from the file. The label is shown only if it was captured; it may be part of the glass-slide image. For NDPIS, these images come from its first channel.");
            if self.task.is_some() { ui.label("Loading embedded images…"); }
            if let Some(error) = &self.error { ui.label(error); }
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
