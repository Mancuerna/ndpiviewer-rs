use super::*;

impl Viewer {
    pub(super) fn cache_bytes(&self) -> usize {
        self.cache
            .iter()
            .map(|e| e.region.w as usize * e.region.h as usize * 4)
            .sum()
    }

    pub(super) fn trim_cache(&mut self) {
        // A per-level/plane ceiling preserves room for earlier zoom levels.
        // All levels and planes also share one global LRU budget.
        loop {
            let overfull = self.cache.iter().find_map(|entry| {
                let r = &entry.region;
                let bytes: usize = self
                    .cache
                    .iter()
                    .filter(|e| e.region.series == r.series && e.region.plane == r.plane)
                    .map(|e| e.region.w as usize * e.region.h as usize * 4)
                    .sum();
                (bytes > self.cache_budget / 2).then_some((r.series, r.plane))
            });
            if self.cache_bytes() <= self.cache_budget && overfull.is_none() {
                break;
            }
            // Keep the current overview when possible, but never exceed the cap.
            let overview = self.overview(self.plane);
            let eligible = |e: &CachedRegion| {
                overfull.is_none_or(|(s, z)| e.region.series == s && e.region.plane == z)
            };
            let index = self
                .cache
                .iter()
                .position(|e| {
                    eligible(e) && !overview.as_ref().is_some_and(|r| same_region(r, &e.region))
                })
                .or_else(|| self.cache.iter().position(eligible))
                .unwrap_or(0);
            self.cache.remove(index);
        }
    }

    pub(super) fn visible_tiles(&self, size: egui::Vec2) -> Vec<Region> {
        let Some(slide) = &self.slide else {
            return vec![];
        };
        let size = self.world_view_size(size);
        let min = (self.center - size / (2.0 * self.zoom)).max(egui::Vec2::ZERO);
        let max = (self.center + size / (2.0 * self.zoom))
            .min(egui::vec2(slide.width as f32, slide.height as f32));
        if min.x >= max.x || min.y >= max.y {
            return vec![];
        }
        let chosen = slide
            .levels
            .iter()
            .rposition(|l| l.width as f32 / slide.width as f32 >= self.zoom.min(1.0))
            .unwrap_or(0);
        for level in &slide.levels[chosen..] {
            let sx = level.width as f32 / slide.width as f32;
            let sy = level.height as f32 / slide.height as f32;
            let x0 = (min.x * sx).floor() as u32 / TILE_SIZE;
            let y0 = (min.y * sy).floor() as u32 / TILE_SIZE;
            let x1 = ((max.x * sx).ceil() as u32)
                .min(level.width)
                .div_ceil(TILE_SIZE);
            let y1 = ((max.y * sy).ceil() as u32)
                .min(level.height)
                .div_ceil(TILE_SIZE);
            if ((x1 - x0) as usize) * ((y1 - y0) as usize) > MAX_VISIBLE_TILES {
                continue;
            }
            let mut tiles = Vec::new();
            for ty in y0..y1 {
                for tx in x0..x1 {
                    let x = tx * TILE_SIZE;
                    let y = ty * TILE_SIZE;
                    let w = TILE_SIZE.min(level.width - x);
                    let h = TILE_SIZE.min(level.height - y);
                    tiles.push(Region {
                        preview: false,
                        series: level.series,
                        interleaved: level.interleaved,
                        plane: self.plane,
                        x,
                        y,
                        w,
                        h,
                        world: [x as f32 / sx, y as f32 / sy, w as f32 / sx, h as f32 / sy],
                    });
                }
            }
            // Recompute priorities from the latest viewport after every read.
            // No obsolete pan/zoom work is allowed to accumulate in a queue.
            tiles.sort_by(|a, b| {
                let distance = |r: &Region| {
                    (egui::vec2(r.world[0] + r.world[2] * 0.5, r.world[1] + r.world[3] * 0.5)
                        - self.center)
                        .length_sq()
                };
                distance(a).total_cmp(&distance(b))
            });
            return tiles;
        }
        vec![] // The bounded overview handles very low zoom on huge slides.
    }

    pub(super) fn next_tile(&self, wanted: &[Region]) -> Option<Region> {
        let missing =
            |r: &Region| !self.cached(r) && !self.failed.iter().any(|f| same_region(f, r));
        self.overview(self.plane)
            .filter(missing)
            .or_else(|| wanted.iter().find(|r| missing(r)).cloned())
            .or_else(|| {
                // Prefetch only small overviews after foreground tiles finish.
                let slide = self.slide.as_ref()?;
                [self.plane.checked_add(1), self.plane.checked_sub(1)]
                    .into_iter()
                    .flatten()
                    .filter(|&z| z < slide.planes)
                    .filter_map(|z| self.overview(z))
                    .find(missing)
            })
    }

    pub(super) fn draw_tiles(&mut self, ui: &egui::Ui, rect: egui::Rect, wanted: &[Region]) {
        // Do not combine focus planes. Retain the old Z until a new overview is
        // available, then refine only with tiles belonging to that same plane.
        if self.overview(self.plane).is_some_and(|r| self.cached(&r)) {
            self.shown_plane = Some(self.plane);
            self.channels.fallback = None;
        }
        let Some(plane) = self.shown_plane else {
            if let Some(old) = &self.channels.fallback {
                self.paint_tile(ui, rect, old);
            }
            return;
        };
        let target_series = wanted.first().map(|r| r.series);
        let overview = self.overview(plane);
        let mut visible: Vec<usize> = self
            .cache
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let r = &e.region;
                let target = self.tile_rect(r.world, rect);
                (r.plane == plane
                    && target.intersects(rect)
                    && (overview.as_ref().is_some_and(|o| same_region(o, r))
                        || target_series.is_some_and(|s| r.series >= s)))
                .then_some(i)
            })
            .collect();
        visible.sort_by_key(|&i| {
            let r = &self.cache[i].region;
            (
                if overview.as_ref().is_some_and(|o| same_region(o, r)) {
                    0
                } else {
                    1
                },
                std::cmp::Reverse(r.series),
            )
        });
        for &i in &visible {
            let e = &self.cache[i];
            self.paint_tile(ui, rect, e);
        }
        // Touch visible entries; cached levels not drawn retain their LRU age.
        visible.sort_unstable();
        let mut touched = Vec::new();
        for i in visible.into_iter().rev() {
            touched.push(self.cache.remove(i));
        }
        self.cache.extend(touched);
    }
}
