use crate::calibration;

pub(crate) const MAX_VERTICES: usize = 256;

pub(crate) fn bounds(points: &[[f32; 2]]) -> [[f32; 2]; 2] {
    let mut lower = [f32::INFINITY; 2];
    let mut upper = [f32::NEG_INFINITY; 2];
    for p in points {
        for axis in 0..2 {
            lower[axis] = lower[axis].min(p[axis]);
            upper[axis] = upper[axis].max(p[axis]);
        }
    }
    [lower, upper]
}

fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f64 {
    (f64::from(b[0]) - f64::from(a[0])) * (f64::from(c[1]) - f64::from(a[1]))
        - (f64::from(b[1]) - f64::from(a[1])) * (f64::from(c[0]) - f64::from(a[0]))
}

fn on_segment(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    cross(a, b, p).abs() < 1e-8
        && (0..2).all(|axis| p[axis] >= a[axis].min(b[axis]) && p[axis] <= a[axis].max(b[axis]))
}

fn intersect(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    (cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0)
        || on_segment(a, b, c)
        || on_segment(a, b, d)
        || on_segment(c, d, a)
        || on_segment(c, d, b)
}

pub(crate) fn valid(points: &[[f32; 2]]) -> bool {
    if !(3..=MAX_VERTICES).contains(&points.len())
        || points.iter().flatten().any(|v| !v.is_finite())
    {
        return false;
    }
    for i in 0..points.len() {
        let next = (i + 1) % points.len();
        if points[i] == points[next] {
            return false;
        }
        for j in (i + 1)..points.len() {
            let end = (j + 1) % points.len();
            if next == j || end == i {
                continue;
            }
            if intersect(points[i], points[next], points[j], points[end]) {
                return false;
            }
        }
    }
    measures(points, None).0 > 1e-8
}

pub(crate) fn measures(
    points: &[[f32; 2]],
    calibration: Option<calibration::Calibration>,
) -> (f64, f64) {
    let spacing = calibration.map_or([1.0, 1.0], |c| [c.x, c.y]);
    let mut twice_area = 0.0;
    let mut perimeter = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        twice_area += f64::from(a[0]) * f64::from(b[1]) - f64::from(b[0]) * f64::from(a[1]);
        perimeter += ((f64::from(b[0]) - f64::from(a[0])) * spacing[0])
            .hypot((f64::from(b[1]) - f64::from(a[1])) * spacing[1]);
    }
    (twice_area.abs() * 0.5 * spacing[0] * spacing[1], perimeter)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concave_area_and_anisotropic_perimeter_and_invalid_contours() {
        let points = [
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 2.0],
            [2.0, 2.0],
            [2.0, 4.0],
            [0.0, 4.0],
        ];
        assert!(valid(&points));
        assert_eq!(measures(&points, None), (12.0, 16.0));
        assert_eq!(
            measures(&points, Some(calibration::Calibration { x: 2.0, y: 3.0 })),
            (72.0, 40.0)
        );
        let reversed: Vec<_> = points.iter().rev().copied().collect();
        assert_eq!(measures(&reversed, None), (12.0, 16.0));
        assert!(!valid(&[[0.0, 0.0], [3.0, 3.0], [0.0, 3.0], [3.0, 0.0]]));
        assert!(!valid(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]));
        assert!(!valid(&[[0.0, 0.0], [1.0, 0.0], [f32::NAN, 1.0]]));
    }
}
