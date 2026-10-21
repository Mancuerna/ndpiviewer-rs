//! Annotation data independent of the UI, window state and storage format.
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum Tool {
    #[default]
    Pan,
    Freehand,
    Line,
    Pointer,
    Rectangle,
    Circle,
    Polygon,
}

#[derive(Clone, PartialEq)]
pub(crate) struct Annotation {
    pub(crate) appearance: Appearance,
    pub(crate) tool: Tool,
    pub(crate) points: Vec<[f32; 2]>,
    pub(crate) plane: u32,
    pub(crate) note: String,
    pub(crate) origin: Option<usize>,
}

impl Annotation {
    /// Source-space selection; callers convert the screen tolerance using zoom.
    pub fn hit_test(&self, point: [f32; 2], tolerance: f32) -> bool {
        if self.points.len() < 2 || !point.iter().all(|p| p.is_finite()) {
            return false;
        }
        let segment = |a: [f32; 2], b: [f32; 2]| {
            let d = [b[0] - a[0], b[1] - a[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            let t = if length > 0.0 {
                (((point[0] - a[0]) * d[0] + (point[1] - a[1]) * d[1]) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (point[0] - a[0] - t * d[0]).hypot(point[1] - a[1] - t * d[1]) <= tolerance
        };
        match self.tool {
            Tool::Pan | Tool::Freehand => false,
            Tool::Line | Tool::Pointer => segment(self.points[0], self.points[1]),
            Tool::Rectangle => (0..2).all(|axis| {
                point[axis] >= self.points[0][axis].min(self.points[1][axis]) - tolerance
                    && point[axis] <= self.points[0][axis].max(self.points[1][axis]) + tolerance
            }),
            Tool::Circle => {
                let center = [
                    (self.points[0][0] + self.points[1][0]) * 0.5,
                    (self.points[0][1] + self.points[1][1]) * 0.5,
                ];
                let radius = [
                    (self.points[1][0] - self.points[0][0]).abs() * 0.5,
                    (self.points[1][1] - self.points[0][1]).abs() * 0.5,
                ];
                if radius[0] <= 0.0 || radius[1] <= 0.0 {
                    return false;
                }
                let scaled = ((point[0] - center[0]) / (radius[0] + tolerance)).powi(2)
                    + ((point[1] - center[1]) / (radius[1] + tolerance)).powi(2);
                scaled <= 1.0
            }
            Tool::Polygon => {
                let mut inside = false;
                for (&a, &b) in self.points.iter().zip(self.points.iter().cycle().skip(1)) {
                    if segment(a, b) {
                        return true;
                    }
                    if (a[1] > point[1]) != (b[1] > point[1])
                        && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selecting_shapes_respects_edges_and_concave_regions() {
        let mut a = Annotation {
            appearance: Appearance::default(),
            tool: Tool::Line,
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            plane: 0,
            note: String::new(),
            origin: None,
        };
        assert!(a.hit_test([5.0, 1.0], 2.0));
        assert!(!a.hit_test([5.0, 3.0], 2.0));
        a.tool = Tool::Rectangle;
        a.points = vec![[10.0, 20.0], [0.0, 0.0]];
        assert!(a.hit_test([5.0, 10.0], 0.0));
        a.tool = Tool::Circle;
        assert!(a.hit_test([5.0, 10.0], 0.0));
        assert!(!a.hit_test([10.0, 20.0], 0.0));
        a.tool = Tool::Polygon;
        a.points = vec![
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 1.0],
            [1.0, 1.0],
            [1.0, 4.0],
            [0.0, 4.0],
        ];
        assert!(a.hit_test([0.5, 3.0], 0.0));
        assert!(!a.hit_test([3.0, 3.0], 0.0));
        assert!(a.hit_test([3.0, 1.1], 0.2));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Appearance {
    pub title: String,
    pub color: [u8; 3],
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            title: String::new(),
            color: [125, 210, 220],
        }
    }
}

impl Appearance {
    pub fn color_hex(&self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            self.color[0], self.color[1], self.color[2]
        )
    }
    pub fn parse_color(text: &str) -> Option<[u8; 3]> {
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ])
    }
}
