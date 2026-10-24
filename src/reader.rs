//! NDPI metadata inspection and bounded pixel decoding. No window or GPU ownership.
use crate::{calibration, channels, ndpa, Level, Region, Slide, MAX_REGION};
use bioformats::{ImageReader, PixelType};
use eframe::egui;
use std::{path::PathBuf, time::Instant};

pub(crate) fn inspect(reader: &mut ImageReader, path: PathBuf) -> Result<Slide, String> {
    reader.set_series(0).map_err(|e| e.to_string())?;
    let base = reader.metadata().clone();
    if base.size_x == 0 || base.size_y == 0 || base.size_z == 0 || base.size_t != 1 {
        return Err(format!(
            "Unsupported slide dimensions: {} × {}, Z {}, T {}.",
            base.size_x, base.size_y, base.size_z, base.size_t
        ));
    }
    // NDPI stores grayscale or packed RGB data per focus plane. Independent
    // acquisition channels are grouped explicitly through an NDPIS manifest.
    if base.image_count != base.size_z
        || !matches!(base.pixel_type, PixelType::Uint8 | PixelType::Uint16)
        || base.is_indexed
        || !(base.size_c == 1 || (base.is_rgb && base.size_c == 3))
    {
        return Err("Supported NDPI data: unsigned 8/16-bit grayscale or RGB, one image per Z plane. Open the .ndpis manifest for separate acquisition channels.".into());
    }
    // bioformats 0.1.8 interleaves marked NDPI IFDs regardless of the
    // is_interleaved flag. Inspect marker tags to honor its actual output layout.
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let large = file.metadata().map_err(|e| e.to_string())?.len() >= (1u64 << 32);
    let mut parser = bioformats::tiff::parser::TiffParser::new(std::io::BufReader::new(file))
        .map_err(|e| e.to_string())?;
    let ifds = if large {
        parser.read_ifds_ndpi64()
    } else {
        parser.read_ifds()
    }
    .map_err(|e| e.to_string())?;
    let mut levels = Vec::new();
    for series in 0..reader.series_count() {
        reader.set_series(series).map_err(|e| e.to_string())?;
        let m = reader.metadata();
        let aspect_error = ((m.size_x as f64 / m.size_y.max(1) as f64)
            / (base.size_x as f64 / base.size_y as f64)
            - 1.0)
            .abs();
        if m.size_x > 0
            && m.size_y > 0
            && m.size_x <= base.size_x
            && m.size_y <= base.size_y
            && m.size_z == base.size_z
            && m.image_count == base.image_count
            && m.pixel_type == base.pixel_type
            && m.size_c == base.size_c
            && m.is_rgb == base.is_rgb
            && aspect_error < 0.01
        {
            levels.push(Level {
                series,
                interleaved: ifds
                    .iter()
                    .find(|ifd| {
                        ifd.image_width() == Some(m.size_x) && ifd.image_length() == Some(m.size_y)
                    })
                    .is_some_and(|ifd| ifd.get(65426).is_some()),
                width: m.size_x,
                height: m.size_y,
            });
        }
    }
    levels.sort_by_key(|l| std::cmp::Reverse(l.width));
    if levels.is_empty() {
        return Err("No usable pyramid levels found.".into());
    }
    let base_ifd = ifds
        .iter()
        .find(|i| i.image_width() == Some(base.size_x) && i.image_length() == Some(base.size_y))
        .ok_or("Missing full-resolution IFD.")?;
    let wavelength = base_ifd.get_vec_f64(65451).first().copied();
    let channel_band = wavelength.and_then(|nm| {
        if nm > 380.0 && nm <= 490.0 {
            Some(2)
        } else if nm > 490.0 && nm <= 580.0 {
            Some(1)
        } else if nm > 580.0 && nm <= 780.0 {
            Some(0)
        } else {
            None
        }
    });
    let color = channel_band.map_or([255, 255, 255], |band| {
        let mut color = [0; 3];
        color[band] = 255;
        color
    });
    let channels = if base.is_rgb {
        channels::rgb_channels()
    } else {
        vec![channels::Channel {
            name: base_ifd
                .get_str(65434)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Intensity")
                .trim_matches('\0')
                .to_string(),
            color,
        }]
    };
    Ok(Slide {
        channels,
        rgb_components: base.is_rgb,
        channel_band,
        ndpa: ndpa::Geometry::from_ifds(&ifds, base.size_x, base.size_y, base.size_z),
        registration: ifds
            .iter()
            .filter(|i| {
                i.image_width() == Some(base.size_x) && i.image_length() == Some(base.size_y)
            })
            .flat_map(|i| [65422, 65423, 65424].map(|tag| i.get(tag).map(|v| format!("{v:?}"))))
            .collect(),
        calibration: calibration::Calibration::from_base_ifds(&ifds, base.size_x, base.size_y),
        path,
        width: base.size_x,
        height: base.size_y,
        planes: base.size_z,
        levels,
    })
}

#[cfg(test)]
pub(crate) fn decode(
    reader: &mut ImageReader,
    region: &Region,
) -> Result<(egui::ColorImage, f64), String> {
    reader
        .set_series(region.series)
        .map_err(|e| e.to_string())?;
    let channels = if reader.metadata().is_rgb {
        channels::rgb_channels()
    } else {
        vec![channels::Channel {
            name: "Intensity".into(),
            color: [255; 3],
        }]
    };
    decode_display(reader, region, &channels::defaults(&channels))
}

pub(crate) fn decode_display(
    reader: &mut ImageReader,
    region: &Region,
    settings: &[channels::Setting],
) -> Result<(egui::ColorImage, f64), String> {
    let start = Instant::now();
    reader
        .set_series(region.series)
        .map_err(|e| e.to_string())?;
    let m = reader.metadata().clone();
    if region.sampling > 1 && !region.preview {
        let image = crate::resample::tile(region, [m.size_x, m.size_y], |x, y, w, h| {
            let tile = Region {
                sampling: 1,
                x,
                y,
                w,
                h,
                ..region.clone()
            };
            decode_display(reader, &tile, settings).map(|(image, _)| image)
        })?;
        return Ok((image, start.elapsed().as_secs_f64() * 1000.0));
    }
    if region.preview {
        // Some slides have no screen-sized pyramid level. Build a bounded
        // overview from small source regions instead of decoding a full plane.
        let mut pixels = vec![egui::Color32::BLACK; region.w as usize * region.h as usize];
        for y in (0..m.size_y).step_by(MAX_REGION as usize) {
            for x in (0..m.size_x).step_by(MAX_REGION as usize) {
                let w = MAX_REGION.min(m.size_x - x);
                let h = MAX_REGION.min(m.size_y - y);
                let tile = Region {
                    sampling: 1,
                    preview: false,
                    x,
                    y,
                    w,
                    h,
                    ..region.clone()
                };
                let (image, _) = decode_display(reader, &tile, settings)?;
                let ox0 = (x as u64 * region.w as u64).div_ceil(m.size_x as u64) as u32;
                let ox1 = ((x + w) as u64 * region.w as u64).div_ceil(m.size_x as u64) as u32;
                let oy0 = (y as u64 * region.h as u64).div_ceil(m.size_y as u64) as u32;
                let oy1 = ((y + h) as u64 * region.h as u64).div_ceil(m.size_y as u64) as u32;
                for oy in oy0..oy1.min(region.h) {
                    let sy = (oy as u64 * m.size_y as u64 / region.h as u64) as u32 - y;
                    for ox in ox0..ox1.min(region.w) {
                        let sx = (ox as u64 * m.size_x as u64 / region.w as u64) as u32 - x;
                        pixels[(oy * region.w + ox) as usize] =
                            image.pixels[(sy * w + sx) as usize];
                    }
                }
            }
        }
        return Ok((
            egui::ColorImage {
                size: [region.w as usize, region.h as usize],
                pixels,
            },
            start.elapsed().as_secs_f64() * 1000.0,
        ));
    }
    let raw = reader
        .open_bytes_region(region.plane, region.x, region.y, region.w, region.h)
        .map_err(|e| e.to_string())?;
    let image = channels::render(
        &raw,
        [region.w as usize, region.h as usize],
        &m,
        region.interleaved,
        settings,
    )?;
    Ok((image, start.elapsed().as_secs_f64() * 1000.0))
}
