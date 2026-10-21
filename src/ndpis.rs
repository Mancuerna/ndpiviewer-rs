#[cfg(test)]
use crate::Region;
use crate::{channels, reader::inspect, source, Slide};
use bioformats::ImageReader;
#[cfg(test)]
use eframe::egui;
use std::path::PathBuf;

/// Parse the Hamamatsu NoImages / Image0..ImageN manifest layout.
/// Keep references local to the manifest directory, with bounded input.
pub(super) fn files(path: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 65536 {
        return Err("NDPIS manifest exceeds 64 KiB.".into());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("Invalid UTF-8 NDPIS manifest: {e}"))?;
    parse(&text, path.parent().ok_or("Missing manifest directory.")?)
}

fn parse(text: &str, parent: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let mut count = None;
    let mut entries = std::collections::BTreeMap::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "NoImages" {
            if count.is_some() {
                return Err("Duplicate NoImages entry.".into());
            }
            count = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| "Invalid channel count.")?,
            );
        } else if let Some(index) = key.strip_prefix("Image") {
            let index = index
                .parse::<usize>()
                .map_err(|_| "Invalid channel index.")?;
            let file = PathBuf::from(value);
            if file
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
                || !file
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ndpi"))
            {
                return Err(
                    "NDPIS channels must be relative NDPI paths inside the manifest directory."
                        .into(),
                );
            }
            if entries.insert(index, parent.join(file)).is_some() {
                return Err("Duplicate channel index.".into());
            }
        }
    }
    let count = count
        .filter(|n| (1..=16).contains(n))
        .ok_or("NDPIS requires 1–16 channels.")?;
    if entries.len() != count {
        return Err("NDPIS channel count does not match its files.".into());
    }
    (0..count)
        .map(|i| {
            entries
                .remove(&i)
                .ok_or("NDPIS channel indexes must start at zero and be consecutive.".into())
        })
        .collect()
}

pub(super) fn open(path: PathBuf) -> Result<(source::Source, Slide), String> {
    let paths = files(&path)?;
    let mut readers = Vec::new();
    let mut slides = Vec::new();
    for file in paths {
        let mut reader =
            ImageReader::open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let slide = inspect(&mut reader, file)?;
        if slide.rgb_components && slide.channel_band.is_none() {
            return Err("An RGB NDPIS channel has no supported emission-wavelength metadata. Its acquisition band cannot be determined safely.".into());
        }
        readers.push(reader);
        slides.push(slide);
    }
    let first = &slides[0];
    for s in &slides[1..] {
        let same_calibration = match (first.calibration, s.calibration) {
            (None, None) => true,
            (Some(a), Some(b)) => (a.x / b.x - 1.0).abs() < 1e-6 && (a.y / b.y - 1.0).abs() < 1e-6,
            _ => false,
        };
        if s.width != first.width
            || s.height != first.height
            || s.planes != first.planes
            || s.registration != first.registration
            || !same_calibration
            || s.levels.len() != first.levels.len()
            || !s
                .levels
                .iter()
                .zip(&first.levels)
                .all(|(a, b)| a.width == b.width && a.height == b.height)
        {
            return Err("Channel geometry, Z count, offsets, calibration or pyramid levels differ. Automatic channel registration is not supported.".into());
        }
    }
    let mut combined = first.clone();
    combined.path = path;
    combined.rgb_components = false;
    let palette = [
        [0, 0, 255],
        [0, 255, 0],
        [255, 0, 0],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 0],
    ];
    combined.channels = slides
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let name = if !s.rgb_components && s.channels[0].name != "Intensity" {
                s.channels[0].name.clone()
            } else {
                s.path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            };
            let color = s.channel_band.map_or(palette[i % palette.len()], |band| {
                let mut c = [0; 3];
                c[band] = 255;
                c
            });
            channels::Channel { name, color }
        })
        .collect();
    let channels = readers
        .into_iter()
        .zip(slides)
        .map(|(reader, slide)| source::AcquisitionChannel { reader, slide })
        .collect();
    Ok((source::Source::Channels(channels), combined))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_16bit_channels_and_z_decode_end_to_end() {
        let root = std::env::temp_dir().join(format!("ndpi-channel-test-{}", std::process::id()));
        crate::synthetic::write_dataset(&root).unwrap();
        let (mut source, slide) = open(root.join("Synthetic_fluorescence.ndpis")).unwrap();
        assert_eq!(slide.channels.len(), 3);
        assert_eq!(slide.planes, 3);
        assert_eq!(slide.calibration.unwrap().x, 0.25);
        let coordinates = slide
            .ndpa
            .as_ref()
            .expect("Synthetic NDPI has physical offsets");
        assert_eq!(coordinates.offset, [-10_000.0, 20_000.0]);
        assert_eq!(coordinates.z, vec![0.0, 1000.0, 2000.0]);
        let mut settings = channels::defaults(&slide.channels);
        let mut region = Region {
            preview: false,
            series: 0,
            interleaved: slide.levels[0].interleaved,
            plane: 0,
            x: 120,
            y: 120,
            w: 1,
            h: 1,
            world: [120.0, 120.0, 1.0, 1.0],
        };
        let first = source.read_display(&region, &settings).unwrap().0.pixels[0];
        assert!(first.b() > 0 && first.g() > 0 && first.r() > 0);
        settings[0].enabled = false;
        let no_blue = source.read_display(&region, &settings).unwrap().0.pixels[0];
        assert_eq!(no_blue.b(), 0);
        assert_eq!(no_blue.r(), first.r());
        assert_eq!(no_blue.g(), first.g());
        settings[1].contrast = 2.0;
        let brighter = source.read_display(&region, &settings).unwrap().0.pixels[0];
        assert!(brighter.g() > no_blue.g());
        region.plane = 2;
        let other_z = source.read_display(&region, &settings).unwrap().0.pixels[0];
        assert!(other_z.r() > brighter.r());
        for setting in &mut settings {
            setting.enabled = false;
        }
        assert_eq!(
            source.read_display(&region, &settings).unwrap().0.pixels[0],
            egui::Color32::BLACK
        );
        drop(source);
        for file in [
            "Blue.ndpi",
            "Green.ndpi",
            "Red.ndpi",
            "Synthetic_fluorescence.ndpis",
        ] {
            std::fs::remove_file(root.join(file)).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn manifest_indexes_and_paths_are_validated() {
        let root = std::path::Path::new("samples");
        let files = parse(
            "[NDPIS]\nImage1=green.ndpi\nNoImages=2\nImage0=blue.ndpi\n",
            root,
        )
        .unwrap();
        assert_eq!(files, vec![root.join("blue.ndpi"), root.join("green.ndpi")]);
        for text in [
            "NoImages=0",
            "NoImages=2\nImage0=a.ndpi",
            "NoImages=1\nImage1=a.ndpi",
            "NoImages=1\nImage0=../a.ndpi",
            "NoImages=1\nImage0=a.ndpi\nImage0=b.ndpi",
        ] {
            assert!(parse(text, root).is_err());
        }
    }
}
