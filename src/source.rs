#[cfg(test)]
use crate::reader::decode;
use crate::{
    channels, ndpis,
    reader::{decode_display, inspect},
    Level, Region, Slide,
};
use bioformats::ImageReader;
use eframe::egui;
use image::RgbImage;
use std::{path::PathBuf, time::Instant};

/// Keep the decoder and its geometry together; they must never be reordered independently.
pub(super) struct AcquisitionChannel {
    pub reader: ImageReader,
    pub slide: Slide,
}

pub(super) enum Source {
    Ndpi(ImageReader),
    Channels(Vec<AcquisitionChannel>),
    Stack {
        files: Vec<PathBuf>,
        dimensions: (u32, u32),
        decoded: Option<(u32, RgbImage)>,
    },
}

impl Source {
    pub(crate) fn auto_contrast(
        &mut self,
        index: usize,
        plane: u32,
        level: &Level,
    ) -> Result<[f32; 3], String> {
        match self {
            Self::Channels(channels) => {
                let channel = channels
                    .get_mut(index)
                    .ok_or("Invalid acquisition channel.")?;
                let level = channel.slide.levels.last().ok_or("No pyramid level.")?;
                let band = channel.slide.channel_band.unwrap_or(0);
                crate::auto_contrast::sample(&mut channel.reader, level, plane, band)
            }
            Self::Ndpi(reader) => crate::auto_contrast::sample(reader, level, plane, index),
            Self::Stack { .. } => Err("Auto is available for NDPI fluorescence channels.".into()),
        }
    }
    pub(super) fn open(path: PathBuf) -> Result<(Self, Slide), String> {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ndpis"))
        {
            return ndpis::open(path);
        }
        if !path.is_dir() {
            let mut reader = ImageReader::open(&path).map_err(|e| e.to_string())?;
            let slide = inspect(&mut reader, path)?;
            return Ok((Self::Ndpi(reader), slide));
        }
        let mut numbered = Vec::new();
        for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
            let file = entry.map_err(|e| e.to_string())?.path();
            if file.extension().is_some_and(|ext| {
                ["jpg", "jpeg", "png"]
                    .iter()
                    .any(|s| ext.eq_ignore_ascii_case(s))
            }) {
                let index = file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<u32>().ok())
                    .ok_or("Stack images must have numeric filenames, such as 00000.jpg.")?;
                numbered.push((index, file));
            }
        }
        numbered.sort_by_key(|(n, _)| *n);
        if numbered.is_empty() || numbered.len() > 1000 {
            return Err("Choose a folder containing 1–1000 numbered JPEG/PNG planes.".into());
        }
        if numbered
            .windows(2)
            .any(|w| w[0].0.checked_add(1) != Some(w[1].0))
        {
            return Err("Stack plane numbers must be unique and consecutive.".into());
        }
        let files: Vec<_> = numbered.into_iter().map(|(_, p)| p).collect();
        let dimensions = image::image_dimensions(&files[0]).map_err(|e| e.to_string())?;
        if dimensions.0 == 0
            || dimensions.1 == 0
            || u64::from(dimensions.0) * u64::from(dimensions.1) > 16_000_000
        {
            return Err("Folder stacks are limited to 16 megapixels per plane.".into());
        }
        for file in &files[1..] {
            if image::image_dimensions(file).map_err(|e| e.to_string())? != dimensions {
                return Err("All stack planes must have identical dimensions.".into());
            }
        }
        let slide = Slide {
            calibration: None,
            channels: channels::rgb_channels(),
            rgb_components: true,
            channel_band: None,
            registration: vec![],
            ndpa: None,
            path,
            width: dimensions.0,
            height: dimensions.1,
            planes: files.len() as u32,
            levels: vec![Level {
                series: 0,
                interleaved: true,
                width: dimensions.0,
                height: dimensions.1,
            }],
        };
        Ok((
            Self::Stack {
                files,
                dimensions,
                decoded: None,
            },
            slide,
        ))
    }

    #[cfg(test)]
    pub(super) fn read(&mut self, r: &Region) -> Result<(egui::ColorImage, f64), String> {
        if let Self::Ndpi(reader) = self {
            return decode(reader, r);
        }
        self.read_display(r, &channels::defaults(&channels::rgb_channels()))
    }

    pub(super) fn read_display(
        &mut self,
        r: &Region,
        settings: &[channels::Setting],
    ) -> Result<(egui::ColorImage, f64), String> {
        if let Self::Ndpi(reader) = self {
            return decode_display(reader, r, settings);
        }
        if let Self::Channels(channels) = self {
            let start = Instant::now();
            if channels.is_empty() || settings.len() != channels.len() {
                return Err("Invalid acquisition channel settings.".into());
            }
            let mut output =
                egui::ColorImage::new([r.w as usize, r.h as usize], egui::Color32::BLACK);
            let level_index = channels[0]
                .slide
                .levels
                .iter()
                .position(|l| l.series == r.series)
                .ok_or("Invalid channel level.")?;
            for (channel, setting) in channels.iter_mut().zip(settings) {
                let AcquisitionChannel { reader, slide } = channel;
                if !setting.enabled {
                    continue;
                }
                let level = &slide.levels[level_index];
                let region = Region {
                    series: level.series,
                    interleaved: level.interleaved,
                    ..r.clone()
                };
                let per_band = if slide.rgb_components {
                    (0..3)
                        .map(|band| channels::Setting {
                            enabled: Some(band) == slide.channel_band,
                            ..setting.clone()
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![setting.clone()]
                };
                let (image, _) = decode_display(reader, &region, &per_band)?;
                for (out, pixel) in output.pixels.iter_mut().zip(image.pixels) {
                    *out = egui::Color32::from_rgb(
                        out.r().saturating_add(pixel.r()),
                        out.g().saturating_add(pixel.g()),
                        out.b().saturating_add(pixel.b()),
                    );
                }
            }
            return Ok((output, start.elapsed().as_secs_f64() * 1000.0));
        }
        let started = Instant::now();
        let Self::Stack {
            files,
            dimensions,
            decoded,
        } = self
        else {
            unreachable!()
        };
        if r.series != 0 || r.plane as usize >= files.len() {
            return Err("Invalid stack plane.".into());
        }
        if decoded.as_ref().is_none_or(|(z, _)| *z != r.plane) {
            // Only one decoded source plane is retained, outside the tile cache.
            *decoded = None;
            let rgb = image::open(&files[r.plane as usize])
                .map_err(|e| e.to_string())?
                .to_rgb8();
            if rgb.dimensions() != *dimensions {
                return Err("Stack dimensions changed on disk.".into());
            }
            *decoded = Some((r.plane, rgb));
        }
        let rgb = &decoded.as_ref().unwrap().1;
        let output = if r.preview {
            image::imageops::resize(rgb, r.w, r.h, image::imageops::FilterType::Triangle)
        } else {
            if r.w == 0
                || r.h == 0
                || r.x.checked_add(r.w).is_none_or(|x| x > dimensions.0)
                || r.y.checked_add(r.h).is_none_or(|y| y > dimensions.1)
            {
                return Err("Stack region is outside the image.".into());
            }
            image::imageops::crop_imm(rgb, r.x, r.y, r.w, r.h).to_image()
        };
        Ok((
            channels::render(
                output.as_raw(),
                [r.w as usize, r.h as usize],
                &bioformats::metadata::ImageMetadata {
                    is_rgb: true,
                    size_c: 3,
                    ..Default::default()
                },
                true,
                settings,
            )?,
            started.elapsed().as_secs_f64() * 1000.0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires extracted public CISD samples"]
    fn cisd_samples_decode_all_twenty_one_planes() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images/CISD_samples");
        let mut checked = 0;
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if !path.is_dir() {
                continue;
            }
            let (mut reader, slide) = Source::open(path.clone()).unwrap();
            assert_eq!(slide.planes, 21);
            let mut checksums = vec![];
            let started = Instant::now();
            for plane in 0..21 {
                let region = Region {
                    preview: false,
                    series: 0,
                    interleaved: true,
                    plane,
                    x: 0,
                    y: 0,
                    w: slide.width,
                    h: slide.height,
                    world: [0.0; 4],
                };
                let (image, _) = reader.read(&region).unwrap();
                assert_eq!(
                    image.pixels.len(),
                    slide.width as usize * slide.height as usize
                );
                checksums.push(image.pixels.iter().fold(0u64, |hash, pixel| {
                    hash.wrapping_mul(31)
                        .wrapping_add(u32::from_le_bytes(pixel.to_array()) as u64)
                }));
                checked += 1;
            }
            assert!(checksums.windows(2).all(|w| w[0] != w[1]));
            eprintln!(
                "{} {}x{}: 21 planes in {:.1} ms",
                path.file_name().unwrap().to_string_lossy(),
                slide.width,
                slide.height,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        assert_eq!(checked, 126);
    }

    #[test]
    fn numbered_stack_planes_are_ordered_and_validated() {
        let root = std::env::temp_dir().join(format!("ndpi-stack-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for z in (0..21).rev() {
            RgbImage::from_pixel(32, 24, image::Rgb([z as u8, 20, 30]))
                .save(root.join(format!("{z:05}.png")))
                .unwrap();
        }
        let (mut source, slide) = Source::open(root.clone()).unwrap();
        assert_eq!(slide.planes, 21);
        for z in 0..21 {
            let r = Region {
                preview: false,
                series: 0,
                interleaved: true,
                plane: z,
                x: 2,
                y: 3,
                w: 8,
                h: 9,
                world: [0.0; 4],
            };
            let (image, _) = source.read(&r).unwrap();
            assert_eq!(image.pixels[0].r(), z as u8);
        }
        drop(source);
        std::fs::rename(root.join("00010.png"), root.join("00099.png")).unwrap();
        assert!(Source::open(root.clone()).is_err());
        for entry in std::fs::read_dir(&root).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
