//! Image metadata and source-space tile requests, shared by readers and rendering.
use crate::{calibration, channels, ndpa};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct Level {
    pub(crate) series: usize,
    pub(crate) interleaved: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
#[derive(Clone, Debug)]
pub(crate) struct Slide {
    pub(crate) channels: Vec<channels::Channel>,
    pub(crate) rgb_components: bool,
    pub(crate) channel_band: Option<usize>,
    pub(crate) registration: Vec<Option<String>>,
    pub(crate) ndpa: Option<ndpa::Geometry>,
    pub(crate) calibration: Option<calibration::Calibration>,
    pub(crate) path: PathBuf,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) planes: u32,
    pub(crate) levels: Vec<Level>,
}

impl Slide {
    /// Scanner focus coordinate, not anatomical depth or section thickness.
    pub(crate) fn focus_microns(&self, plane: u32) -> Option<f64> {
        if plane >= self.planes {
            return None;
        }
        self.ndpa
            .as_ref()?
            .z
            .get(plane as usize)
            .filter(|z| z.is_finite())
            .map(|z| z / 1000.0)
    }

    pub(crate) fn reference_plane(&self) -> Option<u32> {
        (0..self.planes)
            .filter_map(|plane| self.focus_microns(plane).map(|z| (plane, z.abs())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(plane, _)| plane)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn focus_coordinates_use_metadata_not_index_or_assumed_spacing() {
        let mut slide = crate::ndpa::test_slide();
        slide.ndpa.as_mut().unwrap().z = vec![2400.0, -500.0, -3300.0];
        assert_eq!(slide.focus_microns(0), Some(2.4));
        assert_eq!(slide.focus_microns(2), Some(-3.3));
        assert_eq!(slide.focus_microns(3), None);
        assert_eq!(slide.reference_plane(), Some(1));
        slide.ndpa.as_mut().unwrap().z = vec![f64::NAN];
        assert_eq!(slide.focus_microns(0), None);
        assert_eq!(slide.reference_plane(), None);
        slide.ndpa = None;
        assert_eq!(slide.reference_plane(), None);
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Region {
    pub(crate) preview: bool,
    pub(crate) series: usize,
    pub(crate) interleaved: bool,
    pub(crate) plane: u32,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) world: [f32; 4],
}
