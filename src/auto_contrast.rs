//! Bounded native-sample analysis, independent of display colors and adjustments.
use crate::{display::Setting, Level};
use bioformats::{ImageReader, PixelType};

pub(crate) fn sample(
    reader: &mut ImageReader,
    level: &Level,
    plane: u32,
    band: usize,
) -> Result<[f32; 3], String> {
    transfer(&histogram(reader, level, plane, band)?)
}
pub(crate) fn histogram(
    reader: &mut ImageReader,
    level: &Level,
    plane: u32,
    band: usize,
) -> Result<Vec<u32>, String> {
    reader.set_series(level.series).map_err(|e| e.to_string())?;
    let m = reader.metadata().clone();
    let bytes = match m.pixel_type {
        PixelType::Uint8 => 1,
        PixelType::Uint16 => 2,
        _ => return Err("Auto requires unsigned 8-bit or 16-bit samples.".into()),
    };
    let bands = if m.is_rgb { 3 } else { 1 };
    if band >= bands || plane >= m.size_z || m.size_x == 0 || m.size_y == 0 {
        return Err("Invalid channel or focus plane for Auto.".into());
    }
    let maximum = (1u32 << u32::from(m.bits_per_pixel).clamp(1, bytes * 8)) - 1;
    let mut histogram = vec![0u32; maximum as usize + 1];
    // At most 64 patches of 32 x 32 pixels; never decode a whole large plane.
    let w = m.size_x.min(32);
    let h = m.size_y.min(32);
    for gy in 0..8 {
        for gx in 0..8 {
            let x = ((m.size_x - w) as u64 * gx / 7) as u32;
            let y = ((m.size_y - h) as u64 * gy / 7) as u32;
            let raw = reader
                .open_bytes_region(plane, x, y, w, h)
                .map_err(|e| e.to_string())?;
            let count = (w * h) as usize;
            if raw.len() != count * bands * bytes as usize {
                return Err("Unexpected Auto sample buffer size.".into());
            }
            for i in 0..count {
                let offset = (if level.interleaved {
                    i * bands + band
                } else {
                    band * count + i
                }) * bytes as usize;
                let value = if bytes == 1 {
                    raw[offset] as u32
                } else {
                    let pair = [raw[offset], raw[offset + 1]];
                    if m.is_little_endian {
                        u16::from_le_bytes(pair) as u32
                    } else {
                        u16::from_be_bytes(pair) as u32
                    }
                };
                histogram[value.min(maximum) as usize] += 1;
            }
        }
    }
    Ok(histogram)
}

fn transfer(histogram: &[u32]) -> Result<[f32; 3], String> {
    let count: u64 = histogram.iter().map(|&n| u64::from(n)).sum();
    if count == 0 || histogram.len() < 2 {
        return Err("No samples available for Auto.".into());
    }
    let percentile = |numerator: u64| {
        let target = (count * numerator).div_ceil(1000).max(1);
        let mut sum = 0u64;
        histogram
            .iter()
            .position(|&n| {
                sum += u64::from(n);
                sum >= target
            })
            .unwrap()
    };
    let low = percentile(10); // 1st percentile rejects dark outliers.
    let high = percentile(995); // 99.5th percentile rejects isolated hot pixels.
    if high <= low {
        return Err("Auto found too little intensity variation; settings unchanged.".into());
    }
    let maximum = (histogram.len() - 1) as f32;
    // Respect the existing transfer limits, including its brightness offset range.
    let gain = (maximum / (high - low) as f32).min(32.0).min(if low == 0 {
        32.0
    } else {
        maximum / low as f32
    });
    Ok([-(low as f32 / maximum) * gain, gain, 1.0])
}

pub(crate) fn intensities(setting: &Setting) -> [f32; 3] {
    [setting.brightness, setting.contrast, setting.gamma]
}

pub(crate) fn apply(setting: &mut Setting, values: [f32; 3]) {
    [setting.brightness, setting.contrast, setting.gamma] = values;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires downloaded public fluorescence sample"]
    fn real_fluorescence_channels_produce_valid_independent_adjustments() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("example_images/Hamamatsu_fluorescence_Manuel_Stritt/test3.ndpis");
        let (mut source, slide) = crate::source::Source::open(path).unwrap();
        let level = slide.levels.last().unwrap();
        let mut gains = Vec::new();
        for index in 0..slide.channels.len() {
            let values = source.auto_contrast(index, 0, level).unwrap();
            let mut setting = crate::display::defaults(&slide.channels)[index].clone();
            apply(&mut setting, values);
            assert!(setting.is_valid());
            assert!(setting.contrast >= 1.0);
            gains.push(setting.contrast);
        }
        assert!(gains.iter().any(|&gain| gain > 1.0));
    }

    #[test]
    fn stale_results_cannot_change_another_plane_or_manual_adjustments() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.channels.settings = crate::display::defaults(&crate::display::rgb_channels());
        let before = viewer.channels.settings[0].clone();
        viewer.channels.auto_running = Some((viewer.revision, 0, viewer.plane, before.clone()));
        viewer.plane += 1;
        viewer.finish_auto_contrast(viewer.revision, Ok([0.0, 5.0, 1.0]));
        assert_eq!(viewer.channels.settings[0], before);
        viewer.channels.auto_running = Some((viewer.revision, 0, viewer.plane, before));
        viewer.channels.settings[0].gamma = 2.0;
        viewer.finish_auto_contrast(viewer.revision, Ok([0.0, 5.0, 1.0]));
        assert_eq!(viewer.channels.settings[0].gamma, 2.0);
        assert_eq!(viewer.channels.settings[0].contrast, 1.0);
    }
    #[test]
    fn percentiles_ignore_hot_pixels_and_flat_images_remain_unchanged() {
        let mut histogram = vec![0; 256];
        histogram[10] = 1000;
        histogram[110] = 1000;
        histogram[255] = 1;
        let [offset, gain, gamma] = transfer(&histogram).unwrap();
        assert!((gain - 2.55).abs() < 0.001);
        assert!((10.0 / 255.0 * gain + offset).abs() < 0.001);
        assert_eq!(gamma, 1.0);
        histogram.fill(0);
        histogram[0] = 100;
        assert!(transfer(&histogram).is_err());
    }
    #[test]
    fn native_16_bit_gain_is_bounded_and_restore_preserves_color_visibility() {
        let mut histogram = vec![0; 65536];
        histogram[100] = 100;
        histogram[200] = 100;
        let values = transfer(&histogram).unwrap();
        assert_eq!(values[1], 32.0);
        let mut setting = Setting {
            enabled: false,
            color: [12, 34, 56],
            brightness: 0.2,
            contrast: 2.0,
            gamma: 1.5,
        };
        let before = intensities(&setting);
        apply(&mut setting, values);
        assert!(setting.is_valid());
        apply(&mut setting, before);
        assert_eq!(intensities(&setting), before);
        assert!(!setting.enabled);
        assert_eq!(setting.color, [12, 34, 56]);
    }
}
