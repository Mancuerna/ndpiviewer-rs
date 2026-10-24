//! Bounded, per-image navigation history. This records displayed areas,
//! not tissue coverage, attention, or diagnostic review.
use eframe::egui;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub(super) const COLORS: [[u8; 3]; 5] = [
    [237, 240, 242],
    [182, 229, 210],
    [112, 200, 163],
    [41, 153, 110],
    [7, 92, 64],
];
pub(super) fn color(level: u8) -> egui::Color32 {
    let [r, g, b] = COLORS[level.min(4) as usize];
    egui::Color32::from_rgb(r, g, b)
}
const SIDE: usize = 256;
const MAX_PLANES: usize = 128; // At most 8 MiB, independent of slide dimensions.
const DWELL: Duration = Duration::from_millis(350);
#[path = "tracking_export.rs"]
mod export;
#[path = "tracking_store.rs"]
mod storage;

pub(super) struct Tracking {
    pub show: bool,
    pub recording: bool,
    pub minimum: u8,
    pub limit_reached: bool,
    planes: BTreeMap<u32, Vec<u8>>,
    candidate: Option<([u32; 7], Instant, bool)>,
    storage: storage::Storage,
    dirty: bool,
    save_at: Option<Instant>,
}

impl Default for Tracking {
    fn default() -> Self {
        Self {
            show: true,
            recording: true,
            minimum: 1,
            limit_reached: false,
            planes: BTreeMap::new(),
            candidate: None,
            storage: storage::Storage::default(),
            dirty: false,
            save_at: None,
        }
    }
}

impl Tracking {
    pub fn load(&mut self, slide: &super::Slide) {
        self.planes = self.storage.load(slide);
        self.candidate = None;
        self.limit_reached = false;
        self.dirty = false;
        self.save_at = None;
    }

    pub fn message(&self) -> Option<&str> {
        self.storage.message.as_deref()
    }

    pub fn flush(&mut self) {
        if self.dirty && self.storage.save(&self.planes) {
            self.dirty = false;
            self.save_at = None;
        }
    }

    pub fn autosave(&mut self, ctx: &egui::Context) {
        if !self.dirty || !self.storage.writable() {
            return;
        }
        let deadline = *self
            .save_at
            .get_or_insert_with(|| Instant::now() + Duration::from_secs(5));
        if Instant::now() >= deadline {
            self.flush();
            self.save_at = Some(Instant::now() + Duration::from_secs(5));
        }
        if self.dirty {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
    }

    pub fn clear(&mut self) {
        self.planes.clear();
        self.candidate = None;
        self.limit_reached = false;
        self.dirty = true;
    }

    /// Normalized source-space viewport; 90-degree rotation is handled by caller.
    /// Repaint during the dwell interval and once after recording, so a sidebar
    /// rendered earlier in this frame gets the updated overlay on the next frame.
    pub fn observe(
        &mut self,
        plane: u32,
        area: egui::Rect,
        zoom: f32,
        ready: bool,
        now: Instant,
    ) -> bool {
        let level = if zoom >= 2.0 {
            4
        } else if zoom >= 1.0 {
            3
        } else if zoom >= 0.5 {
            2
        } else if zoom >= 0.25 {
            1
        } else {
            0
        };
        if !self.recording || !ready || level == 0 || !area.is_finite() || !area.is_positive() {
            self.candidate = None;
            return false;
        }
        let key = [
            plane,
            area.min.x.to_bits(),
            area.min.y.to_bits(),
            area.max.x.to_bits(),
            area.max.y.to_bits(),
            zoom.to_bits(),
            self.minimum as u32,
        ];
        if !self
            .candidate
            .as_ref()
            .is_some_and(|(previous, _, _)| *previous == key)
        {
            self.candidate = Some((key, now, false));
        }
        let (_, since, recorded) = self.candidate.as_mut().unwrap();
        if *recorded {
            return false;
        }
        if now.duration_since(*since) < DWELL {
            return true;
        }
        *recorded = true;
        if !self.planes.contains_key(&plane) && self.planes.len() >= MAX_PLANES {
            self.limit_reached = true;
            return true;
        }
        let clipped = area.intersect(egui::Rect::from_min_max(
            egui::Pos2::ZERO,
            egui::pos2(1.0, 1.0),
        ));
        if !clipped.is_positive() {
            return false;
        }
        let grid = self
            .planes
            .entry(plane)
            .or_insert_with(|| vec![0; SIDE * SIDE]);
        let lower = |v: f32| ((v * SIDE as f32).floor() as usize).min(SIDE);
        let upper = |v: f32| ((v * SIDE as f32).ceil() as usize).min(SIDE);
        for y in lower(clipped.min.y)..upper(clipped.max.y) {
            for x in lower(clipped.min.x)..upper(clipped.max.x) {
                let cell = &mut grid[y * SIDE + x];
                if *cell < level {
                    *cell = level;
                    self.dirty = true;
                }
            }
        }
        true
    }

    pub fn percent(&self, plane: u32) -> f32 {
        self.planes.get(&plane).map_or(0.0, |grid| {
            100.0 * grid.iter().filter(|&&v| v >= self.minimum).count() as f32
                / (SIDE * SIDE) as f32
        })
    }

    pub fn paint(&self, painter: &egui::Painter, map: egui::Rect, plane: u32) {
        if !self.show {
            return;
        }
        let Some(grid) = self.planes.get(&plane) else {
            return;
        };
        let painter = painter.with_clip_rect(map);
        let cell = map.size() / SIDE as f32;
        // Merge horizontal runs instead of issuing a draw call per cell.
        for y in 0..SIDE {
            let mut x = 0;
            while x < SIDE {
                let level = grid[y * SIDE + x];
                let start = x;
                x += 1;
                while x < SIDE && grid[y * SIDE + x] == level {
                    x += 1;
                }
                if level < self.minimum {
                    continue;
                }
                let rect = egui::Rect::from_min_max(
                    map.min + egui::vec2(start as f32, y as f32) * cell,
                    map.min + egui::vec2(x as f32, (y + 1) as f32) * cell,
                );
                painter.rect_filled(rect, 0.0, color(level));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn half() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.5, 1.0))
    }

    #[test]
    fn records_only_settled_loaded_views_and_keeps_z_independent() {
        let now = Instant::now();
        let mut t = Tracking::default();
        assert!(!t.observe(0, half(), 1.0, false, now));
        assert_eq!(t.percent(0), 0.0);
        assert!(t.observe(0, half(), 1.0, true, now));
        assert_eq!(t.percent(0), 0.0);
        t.observe(0, half(), 1.0, true, now + DWELL);
        assert_eq!(t.percent(0), 50.0);
        assert_eq!(t.percent(1), 0.0);
        t.observe(0, half(), 0.25, true, now + DWELL);
        t.observe(0, half(), 0.25, true, now + DWELL * 2);
        t.minimum = 3;
        assert_eq!(t.percent(0), 50.0); // Revisits never reduce recorded detail.
        t.clear();
        assert_eq!(t.percent(0), 0.0);
    }

    #[test]
    fn overview_pause_motion_and_clipping() {
        let now = Instant::now();
        let mut t = Tracking::default();
        assert!(!t.observe(0, half(), 0.1, true, now));
        t.recording = false;
        assert!(!t.observe(0, half(), 1.0, true, now));
        t.recording = true;
        t.observe(0, half(), 0.5, true, now);
        let outside = egui::Rect::from_min_max(egui::pos2(0.5, -1.0), egui::pos2(2.0, 1.0));
        assert!(t.observe(0, outside, 0.5, true, now + DWELL));
        assert_eq!(t.percent(0), 0.0);
        t.observe(0, outside, 0.5, true, now + DWELL * 2);
        assert_eq!(t.percent(0), 50.0);
        t.minimum = 3;
        assert_eq!(t.percent(0), 0.0);
    }

    #[test]
    fn memory_limit_never_discards_existing_history() {
        let now = Instant::now();
        let mut t = Tracking::default();
        for plane in 0..=MAX_PLANES as u32 {
            t.observe(plane, half(), 1.0, true, now);
            t.observe(plane, half(), 1.0, true, now + DWELL);
        }
        assert_eq!(t.planes.len(), MAX_PLANES);
        assert!(t.limit_reached);
        assert_eq!(t.percent(0), 50.0);
        assert_eq!(t.percent(MAX_PLANES as u32), 0.0);
    }

    #[test]
    fn tiny_viewports_are_recorded_and_visibility_does_not_pause_recording() {
        let now = Instant::now();
        let mut t = Tracking {
            show: false,
            ..Default::default()
        };
        let tiny = egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.2501, 0.2501));
        t.observe(0, tiny, 2.0, true, now);
        assert!(t.observe(0, tiny, 2.0, true, now + DWELL)); // Update sidebar.
        assert!(t.percent(0) > 0.0);
        assert!(!t.observe(0, tiny, 2.0, true, now + DWELL * 2));
        // A loading interruption restarts dwell rather than counting stale pixels.
        t.clear();
        t.observe(0, half(), 1.0, true, now);
        t.observe(0, half(), 1.0, false, now + DWELL);
        assert!(t.observe(0, half(), 1.0, true, now + DWELL * 2));
        assert_eq!(t.percent(0), 0.0);
    }

    #[test]
    fn recorded_history_survives_reopening_and_clear_is_saved() {
        let root = std::env::temp_dir().join(format!("tracking-lifecycle-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut slide = crate::ndpa::test_slide();
        slide.path = root.join("sample.ndpi");
        std::fs::write(&slide.path, b"test image").unwrap();
        let now = Instant::now();
        let mut t = Tracking::default();
        t.load(&slide);
        for (z, zoom) in [(0, 0.5), (2, 2.0)] {
            t.observe(z, half(), zoom, true, now);
            t.observe(z, half(), zoom, true, now + DWELL);
        }
        assert!(t.dirty);
        t.flush();
        assert!(!t.dirty);
        let mut restored = Tracking::default();
        restored.load(&slide);
        assert_eq!(restored.percent(0), 50.0);
        assert_eq!(restored.percent(1), 0.0);
        assert_eq!(restored.percent(2), 50.0);
        restored.minimum = 4;
        assert_eq!(restored.percent(0), 0.0);
        assert_eq!(restored.percent(2), 50.0);
        restored.clear();
        restored.flush();
        let mut cleared = Tracking::default();
        cleared.load(&slide);
        assert_eq!(cleared.percent(0), 0.0);
        assert_eq!(cleared.percent(2), 0.0);
        std::fs::remove_file(root.join("sample.ndpi.ndpitrack")).unwrap();
        std::fs::remove_file(slide.path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
