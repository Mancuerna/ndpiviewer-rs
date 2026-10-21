//! Native-precision display transfer and channel values, independent of viewer state.
use bioformats::PixelType;
use eframe::egui;

#[derive(Clone, Debug)]
pub(super) struct Channel {
    pub name: String,
    pub color: [u8; 3],
}

pub(super) fn rgb_channels() -> Vec<Channel> {
    ["Red component", "Green component", "Blue component"]
        .into_iter()
        .zip([[255, 0, 0], [0, 255, 0], [0, 0, 255]])
        .map(|(name, color)| Channel {
            name: name.into(),
            color,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Setting {
    pub enabled: bool,
    pub brightness: f32,
    pub contrast: f32,
    #[serde(default = "default_gamma")]
    pub gamma: f32,
    pub color: [u8; 3],
}

impl Setting {
    pub(crate) fn is_valid(&self) -> bool {
        self.brightness.is_finite()
            && (-1.0..=1.0).contains(&self.brightness)
            && self.contrast.is_finite()
            && (0.1..=32.0).contains(&self.contrast)
            && self.gamma.is_finite()
            && (0.1..=5.0).contains(&self.gamma)
    }
}

fn default_gamma() -> f32 {
    1.0
}

pub(super) fn defaults(channels: &[Channel]) -> Vec<Setting> {
    channels
        .iter()
        .map(|c| Setting {
            enabled: true,
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            color: c.color,
        })
        .collect()
}

/// Change visibility without resetting intensities or pseudocolors.
/// An invalid solo index leaves the composite unchanged.
pub(crate) fn set_visibility(settings: &mut [Setting], solo: Option<usize>, enabled: bool) -> bool {
    if solo.is_some_and(|index| index >= settings.len()) {
        return false;
    }
    let mut changed = false;
    for (index, setting) in settings.iter_mut().enumerate() {
        let show = solo.map_or(enabled, |selected| index == selected);
        changed |= setting.enabled != show;
        setting.enabled = show;
    }
    changed
}

/// Adjust normalized native samples before quantizing to an 8-bit display.
/// Contrast is gain about black (preserves a black fluorescence background).
fn intensity(value: f32, setting: &Setting) -> f32 {
    if setting.enabled {
        (value * setting.contrast + setting.brightness).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub(super) fn render(
    raw: &[u8],
    size: [usize; 2],
    m: &bioformats::metadata::ImageMetadata,
    interleaved: bool,
    settings: &[Setting],
) -> Result<egui::ColorImage, String> {
    let channels = if m.is_rgb { 3 } else { 1 };
    let bytes = match m.pixel_type {
        PixelType::Uint8 => 1,
        PixelType::Uint16 => 2,
        _ => return Err("Only unsigned 8-bit and 16-bit display samples are supported.".into()),
    };
    let count = size[0]
        .checked_mul(size[1])
        .ok_or("Display dimensions overflow.")?;
    if size.contains(&0)
        || settings.len() != channels
        || count.checked_mul(channels * bytes) != Some(raw.len())
    {
        return Err("Channel count or decoded sample buffer is inconsistent.".into());
    }
    if settings.iter().any(|s| !s.is_valid()) {
        return Err("Display adjustments are outside their supported ranges.".into());
    }
    // Preserve the fast byte-to-texture path for unchanged 8-bit RGB slides.
    let identity = defaults(&if m.is_rgb {
        rgb_channels()
    } else {
        vec![Channel {
            name: String::new(),
            color: [255; 3],
        }]
    });
    if bytes == 1
        && m.bits_per_pixel == 8
        && settings.iter().zip(identity).all(|(s, d)| {
            s.enabled
                && s.brightness == 0.0
                && s.contrast == 1.0
                && s.gamma == 1.0
                && s.color == d.color
        })
    {
        if channels == 1 {
            return Ok(egui::ColorImage::from_gray(size, raw));
        }
        if interleaved {
            return Ok(egui::ColorImage::from_rgb(size, raw));
        }
    }
    let bits = u32::from(m.bits_per_pixel).clamp(1, (bytes * 8) as u32);
    let maximum = ((1u32 << bits) - 1) as f32;
    // At most three 65536-entry tables (~768 KiB), scoped to this decode.
    // No powf in the pixel loop; default gamma keeps the existing fast path.
    let gamma_tables: Vec<Option<Vec<f32>>> = settings
        .iter()
        .map(|setting| {
            (setting.enabled && setting.gamma != 1.0).then(|| {
                (0..(1u32 << (bytes * 8)))
                    .map(|sample| {
                        intensity(sample as f32 / maximum, setting).powf(1.0 / setting.gamma)
                    })
                    .collect()
            })
        })
        .collect();
    let mut pixels = Vec::with_capacity(count);
    for i in 0..count {
        let mut rgb = [0.0f32; 3];
        for (c, setting) in settings.iter().enumerate() {
            let index = (if interleaved {
                i * channels + c
            } else {
                c * count + i
            }) * bytes;
            let sample = if bytes == 1 {
                f32::from(raw[index])
            } else {
                let pair = [raw[index], raw[index + 1]];
                f32::from(if m.is_little_endian {
                    u16::from_le_bytes(pair)
                } else {
                    u16::from_be_bytes(pair)
                })
            };
            let value = gamma_tables[c].as_ref().map_or_else(
                || intensity(sample / maximum, setting),
                |table| table[sample as usize],
            );
            for (component, tint) in rgb.iter_mut().zip(setting.color) {
                *component += value * f32::from(tint);
            }
        }
        pixels.push(egui::Color32::from_rgb(
            rgb[0].round().clamp(0.0, 255.0) as u8,
            rgb[1].round().clamp(0.0, 255.0) as u8,
            rgb[2].round().clamp(0.0, 255.0) as u8,
        ));
    }
    Ok(egui::ColorImage { size, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_dimensions_and_transfer_values_return_errors_before_allocation() {
        let mut metadata = bioformats::metadata::ImageMetadata::default();
        let mut settings = defaults(&[Channel {
            name: String::new(),
            color: [255; 3],
        }]);
        assert!(render(&[], [usize::MAX, 2], &metadata, true, &settings).is_err());
        assert!(render(&[], [0, 1], &metadata, true, &settings).is_err());
        assert!(render(&[], [1, 1], &metadata, true, &settings).is_err());
        settings[0].brightness = f32::NAN;
        assert!(render(&[0], [1, 1], &metadata, true, &settings).is_err());
        settings[0].brightness = 0.0;
        metadata.pixel_type = PixelType::Float32;
        assert!(render(&[0], [1, 1], &metadata, true, &settings).is_err());
    }

    #[test]
    fn solo_and_group_visibility_preserve_display_adjustments() {
        let mut settings = defaults(&rgb_channels());
        settings[0].gamma = 2.2;
        settings[1].brightness = -0.2;
        settings[2].color = [42, 70, 150];
        let original = settings.clone();
        assert!(set_visibility(&mut settings, Some(1), true));
        assert_eq!(
            settings.iter().map(|s| s.enabled).collect::<Vec<_>>(),
            [false, true, false]
        );
        assert!(!set_visibility(&mut settings, Some(1), true));
        assert!(!set_visibility(&mut settings, Some(9), true));
        assert!(set_visibility(&mut settings, None, false));
        assert!(settings.iter().all(|s| !s.enabled));
        assert!(set_visibility(&mut settings, None, true));
        assert_eq!(settings, original);
    }

    #[test]
    fn gamma_preserves_black_white_native_precision_and_disabled_channels() {
        let mut m = bioformats::metadata::ImageMetadata {
            pixel_type: PixelType::Uint16,
            bits_per_pixel: 16,
            is_little_endian: true,
            ..Default::default()
        };
        let mut settings = defaults(&[Channel {
            name: "DAPI".into(),
            color: [255; 3],
        }]);
        settings[0].gamma = 2.0;
        let samples = [0u16, 1, 32, 16384, 32768, 65535];
        let raw: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let image = render(&raw, [samples.len(), 1], &m, true, &settings).unwrap();
        for (sample, pixel) in samples.iter().zip(&image.pixels) {
            let expected = ((*sample as f32 / 65535.0).sqrt() * 255.0).round() as u8;
            assert_eq!(pixel.r(), expected);
        }
        assert!(image.pixels[2].r() > 0);
        settings[0].enabled = false;
        assert!(render(&raw, [6, 1], &m, true, &settings)
            .unwrap()
            .pixels
            .iter()
            .all(|p| *p == egui::Color32::BLACK));
        settings[0].enabled = true;
        settings[0].gamma = 0.0;
        assert!(render(&raw, [6, 1], &m, true, &settings).is_err());
        settings[0].gamma = 2.0;
        m.pixel_type = PixelType::Uint8;
        m.bits_per_pixel = 8;
        assert_eq!(
            render(&[0, 64, 255], [3, 1], &m, true, &settings)
                .unwrap()
                .pixels[1]
                .r(),
            128
        );
        let old = r#"{"enabled":true,"brightness":0.0,"contrast":1.0,"color":[255,255,255]}"#;
        assert_eq!(serde_json::from_str::<Setting>(old).unwrap().gamma, 1.0);
    }

    #[test]
    fn native_precision_endianness_toggle_and_clipping() {
        let mut m = bioformats::metadata::ImageMetadata {
            pixel_type: PixelType::Uint16,
            bits_per_pixel: 16,
            ..Default::default()
        };
        let mut s = vec![Setting {
            enabled: true,
            brightness: 0.0,
            contrast: 32.0,
            gamma: 1.0,
            color: [0, 255, 0],
        }];
        // 16-bit value 32 would disappear if quantized before applying contrast.
        assert_eq!(
            render(&32u16.to_le_bytes(), [1, 1], &m, true, &s)
                .unwrap()
                .pixels[0]
                .g(),
            4
        );
        m.is_little_endian = false;
        assert_eq!(
            render(&32u16.to_be_bytes(), [1, 1], &m, true, &s)
                .unwrap()
                .pixels[0]
                .g(),
            4
        );
        s[0].enabled = false;
        s[0].brightness = 1.0;
        assert_eq!(
            render(&[255, 255], [1, 1], &m, true, &s).unwrap().pixels[0],
            egui::Color32::BLACK
        );
        s[0].enabled = true;
        assert_eq!(
            render(&[255, 255], [1, 1], &m, true, &s).unwrap().pixels[0].g(),
            255
        );
    }
    #[test]
    fn rgb_layout_and_identity() {
        let m = bioformats::metadata::ImageMetadata {
            is_rgb: true,
            size_c: 3,
            ..Default::default()
        };
        let s = defaults(&rgb_channels());
        let interleaved = render(&[10, 20, 30, 40, 50, 60], [2, 1], &m, true, &s).unwrap();
        let planar = render(&[10, 40, 20, 50, 30, 60], [2, 1], &m, false, &s).unwrap();
        assert_eq!(interleaved.pixels, planar.pixels);
        assert_eq!(planar.pixels[1], egui::Color32::from_rgb(40, 50, 60));
    }

    #[test]
    fn brightness_offset_and_significant_bits() {
        let mut m = bioformats::metadata::ImageMetadata::default();
        let mut s = vec![Setting {
            enabled: true,
            brightness: 0.25,
            contrast: 1.0,
            gamma: 1.0,
            color: [255; 3],
        }];
        assert_eq!(
            render(&[64], [1, 1], &m, true, &s).unwrap().pixels[0].r(),
            128
        );
        s[0].brightness = -0.5;
        assert_eq!(
            render(&[64], [1, 1], &m, true, &s).unwrap().pixels[0],
            egui::Color32::BLACK
        );
        s[0].brightness = 0.0;
        m.pixel_type = PixelType::Uint16;
        for bits in [12, 14, 16] {
            m.bits_per_pixel = bits;
            let maximum = ((1u32 << bits) - 1) as u16;
            assert_eq!(
                render(&maximum.to_le_bytes(), [1, 1], &m, true, &s)
                    .unwrap()
                    .pixels[0],
                egui::Color32::WHITE
            );
        }
    }
}
