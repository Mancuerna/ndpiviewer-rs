use bioformats::tiff::ifd::{Ifd, IfdValue};
use eframe::egui;

/// Micrometres per full-resolution source pixel, independently for each axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Calibration {
    pub x: f64,
    pub y: f64,
}

impl Calibration {
    fn from_ifd(ifd: &Ifd) -> Option<Self> {
        // NDPI uses TIFF XResolution/YResolution and ResolutionUnit.
        // Require explicit physical units; never infer scale from magnification.
        let unit_um = match ifd.get_u16(296)? {
            2 => 25_400.0,
            3 => 10_000.0,
            _ => return None,
        };
        let spacing = |tag| {
            let IfdValue::Rational(values) = ifd.get(tag)? else {
                return None;
            };
            let &[(n, d)] = values.as_slice() else {
                return None;
            };
            if n == 0 || d == 0 {
                return None;
            }
            Some(unit_um * f64::from(d) / f64::from(n))
        };
        Some(Self {
            x: spacing(282)?,
            y: spacing(283)?,
        })
    }

    pub fn from_base_ifds(ifds: &[Ifd], width: u32, height: u32) -> Option<Self> {
        let mut base = ifds
            .iter()
            .filter(|i| i.image_width() == Some(width) && i.image_length() == Some(height));
        let calibration = Self::from_ifd(base.next()?)?;
        // All full-resolution focal planes must agree. Never borrow macro or
        // pyramid-level calibration, or silently accept a partially calibrated stack.
        for ifd in base {
            let other = Self::from_ifd(ifd)?;
            if (other.x / calibration.x - 1.0).abs() > 1e-6
                || (other.y / calibration.y - 1.0).abs() > 1e-6
            {
                return None;
            }
        }
        Some(calibration)
    }
}

pub(super) struct ScaleBar {
    pub width: f32,
    pub label: String,
}

/// Format a calibrated length without changing the underlying micrometre value.
pub(super) fn length_label(microns: f64) -> String {
    if microns.abs() >= 1000.0 {
        format!("{:.3} mm", microns / 1000.0)
    } else {
        format!("{microns:.2} µm")
    }
}

/// Format a calibrated area without changing the underlying square-micrometre value.
pub(super) fn area_label(square_microns: f64) -> String {
    if square_microns >= 1_000_000.0 {
        format!("{:.3} mm²", square_microns / 1_000_000.0)
    } else {
        format!("{square_microns:.2} µm²")
    }
}

/// Choose a readable 1/2/5 length in viewport logical pixels, not device pixels.
/// A horizontal screen bar follows source Y after a quarter-turn rotation.
pub(super) fn scale_bar(
    calibration: Option<Calibration>,
    zoom: f32,
    rotation: u8,
    max_width: f32,
) -> Option<ScaleBar> {
    let spacing = calibration.map_or(1.0, |c| if rotation.is_multiple_of(2) { c.x } else { c.y });
    let units_per_point = spacing / f64::from(zoom);
    let available = f64::from(max_width) * units_per_point;
    if !available.is_finite()
        || available <= 0.0
        || !units_per_point.is_finite()
        || units_per_point <= 0.0
    {
        return None;
    }
    let exponent = available.log10().floor();
    let magnitude = 10.0_f64.powf(exponent);
    let normalized = available / magnitude;
    let length = magnitude
        * if normalized >= 5.0 {
            5.0
        } else if normalized >= 2.0 {
            2.0
        } else {
            1.0
        };
    let millimeters = calibration.is_some() && length >= 1000.0;
    let display_length = if millimeters { length / 1000.0 } else { length };
    let display_exponent = if millimeters {
        exponent - 3.0
    } else {
        exponent
    };
    let decimals = (-display_exponent).clamp(0.0, 12.0) as usize;
    let unit = if millimeters {
        "mm"
    } else if calibration.is_some() {
        "µm"
    } else {
        "px"
    };
    Some(ScaleBar {
        width: (length / units_per_point) as f32,
        label: format!("{display_length:.decimals$} {unit}"),
    })
}

pub(super) fn measurement(
    delta: egui::Vec2,
    line: bool,
    calibration: Option<Calibration>,
) -> String {
    let dx = f64::from(delta.x).abs();
    let dy = f64::from(delta.y).abs();
    if let Some(c) = calibration {
        let (x, y) = (dx * c.x, dy * c.y);
        if line {
            length_label(x.hypot(y))
        } else {
            let dimensions = if x < 1000.0 && y < 1000.0 {
                format!("{x:.2} × {y:.2} µm")
            } else if x >= 1000.0 && y >= 1000.0 {
                format!("{:.3} × {:.3} mm", x / 1000.0, y / 1000.0)
            } else {
                format!("{} × {}", length_label(x), length_label(y))
            };
            format!("{dimensions} · {}", area_label(x * y))
        }
    } else if line {
        format!("{:.1} px", dx.hypot(dy))
    } else {
        format!("{dx:.0} × {dy:.0} px · {:.0} px²", dx * dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn area_units_switch_at_one_million_square_microns() {
        for (area, expected) in [
            (0.0, "0.00 µm²"),
            (999_999.0, "999999.00 µm²"),
            (1_000_000.0, "1.000 mm²"),
            (1_250_000.0, "1.250 mm²"),
        ] {
            assert_eq!(area_label(area), expected);
        }
        assert_eq!(
            measurement(egui::vec2(1000.0, 1000.0), false, None),
            "1000 × 1000 px · 1000000 px²"
        );
    }
    #[test]
    fn length_units_switch_at_one_thousand_microns_only() {
        let c = Some(Calibration { x: 0.5, y: 1.0 });
        for (pixels, expected) in [
            (1999.0, "999.50 µm"),
            (2000.0, "1.000 mm"),
            (2500.0, "1.250 mm"),
        ] {
            assert_eq!(measurement(egui::vec2(pixels, 0.0), true, c), expected);
        }
        assert_eq!(
            measurement(egui::vec2(2500.0, 0.0), true, None),
            "2500.0 px"
        );
        assert_eq!(
            measurement(egui::vec2(2500.0, 500.0), false, c),
            "1.250 mm × 500.00 µm · 625000.00 µm²"
        );
        assert_eq!(
            measurement(egui::vec2(2000.0, 1000.0), false, c),
            "1.000 × 1.000 mm · 1.000 mm²"
        );
        let unit = Some(Calibration { x: 1.0, y: 1.0 });
        for (width, expected) in [(999.0, "500 µm"), (1000.0, "1 mm"), (2000.0, "2 mm")] {
            let bar = scale_bar(unit, 1.0, 0, width).unwrap();
            assert_eq!(bar.label, expected);
            assert_eq!(bar.width, if width < 1000.0 { 500.0 } else { width });
        }
        assert_eq!(scale_bar(None, 1.0, 0, 2000.0).unwrap().label, "2000 px");
    }
    #[test]
    fn scale_bar_tracks_zoom_rotation_and_missing_calibration() {
        let c = Some(Calibration { x: 0.25, y: 0.5 });
        for (zoom, rotation, label, width) in [
            (1.0, 0, "20 µm", 80.0),
            (2.0, 0, "20 µm", 160.0),
            (1.0, 1, "50 µm", 100.0),
            (1.0, 2, "20 µm", 80.0),
            (1.0, 3, "50 µm", 100.0),
            (0.01, 0, "2 mm", 80.0),
        ] {
            let bar = scale_bar(c, zoom, rotation, 160.0).unwrap();
            assert_eq!(bar.label, label);
            assert!((bar.width - width).abs() < 0.001);
        }
        let bar = scale_bar(None, 2.0, 1, 160.0).unwrap();
        assert_eq!(bar.label, "50 px");
        assert_eq!(bar.width, 100.0);
        let bar = scale_bar(Some(Calibration { x: 0.001, y: 0.001 }), 8.0, 0, 160.0).unwrap();
        assert_eq!(bar.label, "0.02 µm");
        for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(scale_bar(c, zoom, 0, 160.0).is_none());
        }
    }
    #[test]
    #[ignore = "Requires downloaded public NDPI samples"]
    fn public_ndpi_calibration() {
        let folder = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images");
        let mut checked = 0;
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ndpi"))
            {
                continue;
            }
            let mut reader = bioformats::ImageReader::open(&path).unwrap();
            let slide = crate::inspect(&mut reader, path.clone()).unwrap();
            let c = slide
                .calibration
                .expect("Public NDPI should carry physical calibration");
            println!(
                "{}: X {:.9}, Y {:.9} µm/px",
                path.file_name().unwrap().to_string_lossy(),
                c.x,
                c.y
            );
            assert!(c.x > 0.01 && c.x < 10.0 && c.y > 0.01 && c.y < 10.0);
            checked += 1;
        }
        assert!(checked >= 5);
    }

    fn ifd(unit: u16, x: (u32, u32), y: (u32, u32)) -> Ifd {
        let mut i = Ifd::default();
        for (tag, value) in [
            (256, IfdValue::Long(vec![100])),
            (257, IfdValue::Long(vec![80])),
            (296, IfdValue::Short(vec![unit])),
            (282, IfdValue::Rational(vec![x])),
            (283, IfdValue::Rational(vec![y])),
        ] {
            i.entries.insert(tag, value);
        }
        i
    }

    #[test]
    fn physical_units_and_anisotropic_measurements() {
        let c = Calibration::from_ifd(&ifd(3, (40_000, 1), (20_000, 1))).unwrap();
        assert_eq!(c, Calibration { x: 0.25, y: 0.5 });
        assert_eq!(
            Calibration::from_ifd(&ifd(2, (101_600, 1), (50_800, 1))),
            Some(c)
        );
        assert_eq!(
            measurement(egui::vec2(-12.0, 8.0), true, Some(c)),
            "5.00 µm"
        );
        assert_eq!(
            measurement(egui::vec2(12.0, -8.0), false, Some(c)),
            "3.00 × 4.00 µm · 12.00 µm²"
        );
        assert_eq!(measurement(egui::vec2(3.0, 4.0), true, None), "5.0 px");
    }

    #[test]
    fn rejects_invalid_missing_or_inconsistent_calibration() {
        for i in [
            ifd(1, (1, 1), (1, 1)),
            ifd(3, (0, 1), (1, 1)),
            ifd(3, (1, 0), (1, 1)),
            Ifd::default(),
        ] {
            assert!(Calibration::from_ifd(&i).is_none());
        }
        let a = ifd(3, (40_000, 1), (40_000, 1));
        let b = ifd(3, (20_000, 1), (40_000, 1));
        assert!(Calibration::from_base_ifds(&[a.clone(), b], 100, 80).is_none());
        assert!(Calibration::from_base_ifds(std::slice::from_ref(&a), 50, 40).is_none());
        assert!(Calibration::from_base_ifds(&[a.clone(), a], 100, 80).is_some());
    }
}
