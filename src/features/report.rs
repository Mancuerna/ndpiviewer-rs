//! Streaming measurement reports. NDPA remains the annotation storage format.
use super::{circle, persistence, Annotation, Tool};
use crate::{calibration::Calibration, polygon, Slide, Viewer};
use eframe::egui;
use std::io::Write;

#[derive(Default)]
struct Measures {
    length: Option<f64>,
    area: Option<f64>,
    perimeter: Option<f64>,
    radius: Option<f64>,
}

fn measures(a: &Annotation, calibration: Option<Calibration>) -> Measures {
    let spacing = calibration.map_or([1.0, 1.0], |c| [c.x, c.y]);
    if a.tool == Tool::Polygon {
        let (area, perimeter) = polygon::measures(&a.points, calibration);
        return Measures {
            area: Some(area),
            perimeter: Some(perimeter),
            ..Default::default()
        };
    }
    let [start, end] = a.points.as_slice() else {
        return Measures::default();
    };
    let dx = (f64::from(end[0]) - f64::from(start[0])).abs() * spacing[0];
    let dy = (f64::from(end[1]) - f64::from(start[1])).abs() * spacing[1];
    match a.tool {
        Tool::Line => Measures {
            length: Some(dx.hypot(dy)),
            ..Default::default()
        },
        Tool::Rectangle => Measures {
            area: Some(dx * dy),
            perimeter: Some(2.0 * (dx + dy)),
            ..Default::default()
        },
        Tool::Circle if circle::valid(&a.points, calibration) => {
            let radius = dx * 0.5;
            Measures {
                area: Some(std::f64::consts::PI * radius * radius),
                perimeter: Some(std::f64::consts::TAU * radius),
                radius: Some(radius),
                ..Default::default()
            }
        }
        _ => Measures::default(),
    }
}

fn text_cell(value: &str) -> String {
    // Imported titles and details must remain text when opened in a spreadsheet.
    let prefix = if value.trim_start().starts_with(['=', '+', '-', '@']) {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}

fn number(value: Option<f64>) -> String {
    value.map_or_else(String::new, |v| format!("{v:.6}"))
}

fn write_report(
    output: &mut impl Write,
    annotations: &[Annotation],
    slide: &Slide,
) -> Result<(), String> {
    if !persistence::validate(annotations, slide) {
        return Err("Invalid annotation coordinates.".into());
    }
    let write = |error: std::io::Error| error.to_string();
    output.write_all(b"\xEF\xBB\xBFIndex,Type,Title,Notes,Color,Z index (1-based),Z position (nm),Length,Perimeter,Radius,Length unit,Area,Area unit,Center X (px),Center Y (px)\r\n").map_err(write)?;
    let (unit, area_unit) = if slide.calibration.is_some() {
        ("um", "um^2")
    } else {
        ("px", "px^2")
    };
    for (index, a) in annotations.iter().enumerate() {
        let kind = match a.tool {
            Tool::Line => "Line",
            Tool::Pointer => "Pointer",
            Tool::Rectangle => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Polygon => "Polygon",
            Tool::Pan | Tool::Freehand => continue,
        };
        let m = measures(a, slide.calibration);
        let [min, max] = polygon::bounds(&a.points);
        let z = slide
            .ndpa
            .as_ref()
            .and_then(|g| g.z.get(a.plane as usize))
            .copied();
        let fields = [
            (index + 1).to_string(),
            kind.to_owned(),
            text_cell(&a.appearance.title),
            text_cell(&a.note),
            a.appearance.color_hex(),
            (a.plane + 1).to_string(),
            number(z),
            number(m.length),
            number(m.perimeter),
            number(m.radius),
            unit.to_owned(),
            number(m.area),
            area_unit.to_owned(),
            number(Some((f64::from(min[0]) + f64::from(max[0])) * 0.5)),
            number(Some((f64::from(min[1]) + f64::from(max[1])) * 0.5)),
        ];
        output
            .write_all(fields.join(",").as_bytes())
            .map_err(write)?;
        output.write_all(b"\r\n").map_err(write)?;
    }
    output.flush().map_err(write)
}

impl Viewer {
    pub(super) fn report_button(&mut self, ui: &mut egui::Ui, slide: &Slide) {
        if ui.add_enabled(!self.features.annotations.is_empty(), egui::Button::new("Export measurements CSV…"))
            .on_hover_text("Export all supported annotations on all Z planes, including hidden marks. NDPA remains the annotation file.")
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new().add_filter("CSV report", &["csv"]).set_file_name("measurements.csv").save_file() {
                let result = crate::export_file::write(&path, |output| {
                    write_report(output, &self.features.annotations, slide)
                });
                match result {
                    Ok(()) => self.status = format!("Measurements exported: {}", path.display()),
                    Err(error) => {
                        crate::diagnostics::record(crate::diagnostics::Event::ReportExportFailed);
                        self.features.error = Some(format!("Measurement export failed: {error}"));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn annotation(tool: Tool) -> Annotation {
        Annotation {
            appearance: Default::default(),
            tool,
            points: vec![[0.0, 0.0], [3.0, 4.0]],
            plane: 0,
            note: String::new(),
            origin: None,
        }
    }
    #[test]
    fn measures_apply_independent_xy_calibration_and_keep_pixel_fallback() {
        let c = Some(Calibration { x: 2.0, y: 3.0 });
        assert_eq!(measures(&annotation(Tool::Line), None).length, Some(5.0));
        assert_eq!(
            measures(&annotation(Tool::Line), c).length,
            Some(6.0f64.hypot(12.0))
        );
        let rectangle = measures(&annotation(Tool::Rectangle), c);
        assert_eq!(rectangle.area, Some(72.0));
        assert_eq!(rectangle.perimeter, Some(36.0));
        let mut circle = annotation(Tool::Circle);
        circle.points = vec![[0.0, 0.0], [6.0, 4.0]];
        let result = measures(&circle, c);
        assert_eq!(result.radius, Some(6.0));
        assert_eq!(result.area, Some(std::f64::consts::PI * 36.0));
    }
    #[test]
    fn csv_preserves_unicode_quotes_and_z_and_neutralizes_formulas() {
        let mut a = annotation(Tool::Line);
        a.appearance.title = "=1+1".into();
        a.note = "Cell, \"A\"\nµm".into();
        let mut slide = crate::ndpa::test_slide();
        let mut output = Vec::new();
        write_report(&mut output, &[a.clone()], &slide).unwrap();
        let csv = String::from_utf8(output).unwrap();
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("\"'=1+1\""));
        assert!(csv.contains("\"Cell, \"\"A\"\"\nµm\""));
        assert!(csv.contains(",um,"));
        assert_eq!(text_cell(" @SUM(A1)"), "\"' @SUM(A1)\"");
        assert_eq!(a.appearance.title, "=1+1");
        slide.calibration = None;
        let mut pixels = Vec::new();
        write_report(&mut pixels, &[a], &slide).unwrap();
        assert!(String::from_utf8(pixels)
            .unwrap()
            .contains(",5.000000,,,px,,px^2,"));
    }
}
