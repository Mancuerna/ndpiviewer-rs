//! Physical circles are stored as source-pixel bounding corners.
use crate::{calibration::Calibration, Slide};
use eframe::egui;

fn spacing(calibration: Option<Calibration>) -> [f64; 2] {
    calibration.map_or([1.0, 1.0], |c| [c.x, c.y])
}

pub(super) fn from_drag(center: egui::Vec2, edge: egui::Vec2, slide: &Slide) -> [[f32; 2]; 2] {
    let [x, y] = spacing(slide.calibration);
    let delta = edge - center;
    let requested = (f64::from(delta.x) * x).hypot(f64::from(delta.y) * y);
    // Keep the whole physical circle inside the source, regardless of orientation.
    let limit = (f64::from(center.x.min(slide.width as f32 - center.x)) * x)
        .min(f64::from(center.y.min(slide.height as f32 - center.y)) * y)
        .max(0.0);
    let radius = requested.min(limit);
    let extent = egui::vec2((radius / x) as f32, (radius / y) as f32);
    [(center - extent).into(), (center + extent).into()]
}

pub(crate) fn valid(points: &[[f32; 2]], calibration: Option<Calibration>) -> bool {
    let [a, b] = points else {
        return false;
    };
    let [x, y] = spacing(calibration);
    let rx = f64::from(b[0] - a[0]) * x * 0.5;
    let ry = f64::from(b[1] - a[1]) * y * 0.5;
    points.iter().flatten().all(|v| v.is_finite())
        && rx.is_finite()
        && ry.is_finite()
        && rx > 0.0
        && ry > 0.0
        && (rx - ry).abs() <= (rx.max(ry) * 0.001).max(0.002)
}

pub(super) fn label(points: &[[f32; 2]], calibration: Option<Calibration>) -> String {
    let radius = f64::from(points[1][0] - points[0][0]).abs() * spacing(calibration)[0] * 0.5;
    let unit = if calibration.is_some() { "µm" } else { "px" };
    format!(
        "Radius {radius:.2} {unit}\nArea {:.2} {unit}²",
        std::f64::consts::PI * radius * radius
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires the public JNOC-GC2003 NDPI sample"]
    fn reference_circle_matches_the_actual_public_slide_and_focus_plane() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("example_images/JNOC-GC2003.ndpi");
        let (_, slide) = crate::source::Source::open(path).unwrap();
        let xml = include_str!("../tests/fixtures/ndpview2-circle.xml");
        let (_, annotations) = crate::ndpa::Archive::read(xml.into(), &slide).unwrap();
        assert_eq!(annotations.len(), 1);
        let a = &annotations[0];
        assert_eq!(slide.ndpa.as_ref().unwrap().z[a.plane as usize], 0.0);
        assert!(valid(&a.points, slide.calibration));
        let radius =
            f64::from(a.points[1][0] - a.points[0][0]) * slide.calibration.unwrap().x * 0.5;
        assert!((radius - 1080.484).abs() < 0.01);
        assert!(super::super::persistence::validate(&annotations, &slide));
    }

    #[test]
    fn imports_unmodified_reference_viewer_circle_and_preserves_vendor_fields() {
        let xml = include_str!("../tests/fixtures/ndpview2-circle.xml");
        let mut slide = crate::ndpa::test_slide();
        // A synthetic coordinate frame isolates format parsing from a large NDPI download.
        slide.width = 200;
        slide.height = 200;
        slide.calibration = Some(Calibration {
            x: 25_000.0,
            y: 25_000.0,
        });
        slide.ndpa.as_mut().unwrap().offset = [-10_823_583.0, 2_642_167.0];
        let (archive, annotations) = crate::ndpa::Archive::read(xml.into(), &slide).unwrap();
        assert_eq!(annotations.len(), 1);
        let a = &annotations[0];
        assert!(a.tool == crate::features::Tool::Circle);
        assert_eq!(a.plane, 1);
        let radius = f64::from(a.points[1][0] - a.points[0][0]) * 25_000.0 * 0.5;
        assert!((radius - 1080.484).abs() < 0.2);
        let saved = archive.write(&annotations, &slide).unwrap();
        assert!(saved.contains("<radius>1080484</radius>"));
        assert!(saved.contains("<lens>2,776042</lens>"));
        assert!(saved.contains("color=\"#000000\""));
    }

    #[test]
    fn anisotropic_circle_is_round_in_physical_coordinates_and_clipped_to_slide() {
        let slide = crate::ndpa::test_slide();
        let points = from_drag(egui::vec2(50.0, 40.0), egui::vec2(70.0, 40.0), &slide);
        assert_eq!(points, [[30.0, 30.0], [70.0, 50.0]]);
        assert!(valid(&points, slide.calibration));
        assert_eq!(
            label(&points, slide.calibration),
            "Radius 5.00 µm\nArea 78.54 µm²"
        );
        let points = from_drag(egui::vec2(2.0, 40.0), egui::vec2(70.0, 40.0), &slide);
        assert_eq!(points[0][0], 0.0);
        assert!(valid(&points, slide.calibration));
        assert!(!valid(&[[1.0, 1.0], [1.0, 1.0]], None));
        assert!(!valid(&[[f32::NAN, 0.0], [2.0, 2.0]], None));
    }

    #[test]
    fn official_circle_schema_round_trips_with_anisotropic_calibration() {
        let slide = crate::ndpa::test_slide();
        let annotation = crate::features::Annotation {
            appearance: Default::default(),
            tool: crate::features::Tool::Circle,
            points: vec![[30.0, 30.0], [70.0, 50.0]],
            plane: 1,
            note: "Circle & radius".into(),
            origin: None,
        };
        let xml = crate::ndpa::write_new(&[annotation], &slide).unwrap();
        assert!(xml.contains("type=\"circle\""));
        assert!(xml.contains("<radius>5000</radius>"));
        let (archive, restored) = crate::ndpa::Archive::read(xml.clone(), &slide).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].points, [[30.0, 30.0], [70.0, 50.0]]);
        assert_eq!(restored[0].plane, 1);
        assert_eq!(archive.write(&restored, &slide).unwrap(), xml);
    }
}
