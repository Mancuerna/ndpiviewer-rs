use super::*;
use std::fmt::Write;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

impl Tracking {
    /// Standalone, source-oriented report. SVG exports are not tracking sidecars.
    pub fn export_svg(&self, name: &str, width: u32, height: u32, plane: u32) -> String {
        let ratio = width.max(1) as f64 / height.max(1) as f64;
        let (w, h) = if ratio >= 1.0 {
            (960.0, 960.0 / ratio)
        } else {
            (700.0 * ratio, 700.0)
        };
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1040\" height=\"{}\" viewBox=\"0 0 1040 {}\">\
            <rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\
            <g font-family=\"sans-serif\" fill=\"#17232b\">\
            <text x=\"40\" y=\"32\" font-size=\"22\">Visited-area map</text>\
            <text x=\"40\" y=\"58\" font-size=\"14\">{}</text>\
            <text x=\"40\" y=\"82\" font-size=\"14\">Z {} | Source: {} x {} pixels | Visited: {:.1}%</text>",
            h + 250.0, h + 250.0, escape(name), plane + 1, width, height, self.percent(plane)
        );
        let _ = write!(svg, "<svg x=\"40\" y=\"100\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 256 256\" preserveAspectRatio=\"none\" shape-rendering=\"crispEdges\"><rect width=\"256\" height=\"256\" fill=\"#edf0f2\"/>");
        let colors = ["#edf0f2", "#b6e5d2", "#70c8a3", "#29996e", "#075c40"];
        if let Some(grid) = self.planes.get(&plane) {
            for y in 0..SIDE {
                let mut x = 0;
                while x < SIDE {
                    let start = x;
                    let level = grid[y * SIDE + x];
                    x += 1;
                    while x < SIDE && grid[y * SIDE + x] == level {
                        x += 1;
                    }
                    if level > 0 && level >= self.minimum {
                        let _ = write!(
                            svg,
                            "<rect x=\"{start}\" y=\"{y}\" width=\"{}\" height=\"1\" fill=\"{}\"/>",
                            x - start,
                            colors[level.min(4) as usize]
                        );
                    }
                }
            }
        }
        svg.push_str("</svg>");
        let _ = write!(svg, "<rect x=\"40\" y=\"100\" width=\"{w}\" height=\"{h}\" fill=\"none\" stroke=\"#62717b\"/>");
        for (i, label) in [
            "Unrecorded / filtered",
            "25–49%",
            "50–99%",
            "100–199%",
            "200%+",
        ]
        .iter()
        .enumerate()
        {
            let x = 40 + i * 195;
            let y = h + 124.0;
            let _ = write!(svg, "<rect x=\"{x}\" y=\"{y}\" width=\"18\" height=\"18\" fill=\"{}\"/><text x=\"{}\" y=\"{}\" font-size=\"12\">{label}</text>", colors[i], x+24, y+14.0);
        }
        let threshold = [0, 25, 50, 100, 200][self.minimum.min(4) as usize];
        let _ = write!(svg, "<text x=\"40\" y=\"{}\" font-size=\"14\">Minimum pixel zoom: {threshold}% | Grid: 256 x 256 | Original slide orientation</text>\
            <text x=\"40\" y=\"{}\" font-size=\"13\">Approximate image area, including background; not tissue coverage or proof of review.</text>\
            <text x=\"40\" y=\"{}\" font-size=\"13\">Recorded views only. This report contains no slide pixels and cannot restore navigation history.</text></g></svg>", h+175.0,h+198.0,h+221.0);
        svg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_is_xml_safe_and_respects_plane_filter_and_aspect_ratio() {
        let mut tracking = Tracking::default();
        tracking.planes.insert(1, vec![3; SIDE * SIDE]);
        let exported = tracking.export_svg("A<&\".ndpi", 2000, 1000, 1);
        assert!(exported.contains("A&lt;&amp;&quot;.ndpi"));
        assert!(exported.contains("Visited: 100.0%"));
        assert!(exported.contains("width=\"960\" height=\"480\""));
        let mut reader = quick_xml::Reader::from_str(&exported);
        loop {
            if reader.read_event().unwrap() == quick_xml::events::Event::Eof {
                break;
            }
        }
        assert!(tracking
            .export_svg("slide", 2000, 1000, 0)
            .contains("Visited: 0.0%"));
        tracking.minimum = 4;
        assert!(tracking
            .export_svg("slide", 2000, 1000, 1)
            .contains("Visited: 0.0%"));
    }
}
