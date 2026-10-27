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
            let visible = self.visible_tiles(self.viewport_size);
            let eligible = |e: &CachedRegion| {
                overfull.is_none_or(|(s, z)| e.region.series == s && e.region.plane == z)
            };
            let index = self
                .cache
                .iter()
                .position(|e| {
                    eligible(e)
                        && !overview.as_ref().is_some_and(|r| same_region(r, &e.region))
                        && !visible.iter().any(|r| same_region(r, &e.region))
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
        let desired = (self.zoom * self.pixel_ratio).min(1.0);
        let chosen = slide
            .levels
            .iter()
            .rposition(|l| {
                (l.width as f32 / slide.width as f32).min(l.height as f32 / slide.height as f32)
                    >= desired
            })
            .unwrap_or(0);
        let level = &slide.levels[chosen];
        let sx = level.width as f32 / slide.width as f32;
        let sy = level.height as f32 / slide.height as f32;
        // Fill gaps in sparse pyramids with filtered virtual levels, rather than
        // magnifying a tiny macro image. Power-of-two grids remain reusable on pan.
        let exponent = (sx.min(sy) / desired)
            .max(1.0)
            .log2()
            .floor()
            .clamp(0.0, 16.0) as u32;
        let mut sampling = 1_u32 << exponent;
        // Leave room for the overview even at the smallest split-view budget.
        let overview_bytes = self
            .overview(self.plane)
            .map_or(0, |r| r.w as usize * r.h as usize * 4);
        let tile_limit = ((self.cache_budget / 2).saturating_sub(overview_bytes)
            / (TILE_SIZE as usize * TILE_SIZE as usize * 4))
            .clamp(1, MAX_VISIBLE_TILES);
        loop {
            let span = TILE_SIZE * sampling;
            let x0 = (min.x * sx).floor() as u32 / span;
            let y0 = (min.y * sy).floor() as u32 / span;
            let x1 = ((max.x * sx).ceil() as u32).min(level.width).div_ceil(span);
            let y1 = ((max.y * sy).ceil() as u32)
                .min(level.height)
                .div_ceil(span);
            if ((x1 - x0) as usize) * ((y1 - y0) as usize) > tile_limit {
                sampling *= 2;
                continue;
            }
            let mut tiles = Vec::new();
            for ty in y0..y1 {
                for tx in x0..x1 {
                    let x = tx * span;
                    let y = ty * span;
                    let source_w = span.min(level.width - x);
                    let w = source_w.div_ceil(sampling);
                    let source_h = span.min(level.height - y);
                    let h = source_h.div_ceil(sampling);
                    tiles.push(Region {
                        sampling,
                        preview: false,
                        series: level.series,
                        interleaved: level.interleaved,
                        plane: self.plane,
                        x,
                        y,
                        w,
                        h,
                        world: [
                            x as f32 / sx,
                            y as f32 / sy,
                            source_w as f32 / sx,
                            source_h as f32 / sy,
                        ],
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
    }

    /// A bounded ring on the same source/sampling grid as the visible field.
    /// Budget the whole working set, including the overview, to avoid eviction loops.
    fn adjacent_tiles(&self, wanted: &[Region]) -> Vec<Region> {
        if !self.preferences.prefetch_neighbors {
            return vec![];
        }
        let Some(first) = wanted.first() else {
            return vec![];
        };
        let Some(slide) = &self.slide else {
            return vec![];
        };
        let Some(level) = slide.levels.iter().find(|l| l.series == first.series) else {
            return vec![];
        };
        let span = TILE_SIZE * first.sampling;
        let sx = level.width as f32 / slide.width as f32;
        let sy = level.height as f32 / slide.height as f32;
        let bytes = |r: &Region| r.w as usize * r.h as usize * 4;
        let used = wanted.iter().map(bytes).sum::<usize>()
            + self.overview(self.plane).as_ref().map_or(0, bytes);
        let mut remaining = (self.cache_budget / 2)
            .saturating_sub(used)
            .min(self.cache_budget / 4);
        let x0 = wanted
            .iter()
            .map(|r| r.x / span)
            .min()
            .unwrap()
            .saturating_sub(1);
        let y0 = wanted
            .iter()
            .map(|r| r.y / span)
            .min()
            .unwrap()
            .saturating_sub(1);
        let x1 =
            (wanted.iter().map(|r| r.x / span).max().unwrap() + 2).min(level.width.div_ceil(span));
        let y1 =
            (wanted.iter().map(|r| r.y / span).max().unwrap() + 2).min(level.height.div_ceil(span));
        let mut neighbors = Vec::new();
        for ty in y0..y1 {
            for tx in x0..x1 {
                let x = tx * span;
                let y = ty * span;
                if wanted.iter().any(|r| r.x == x && r.y == y) {
                    continue;
                }
                let width = span.min(level.width - x);
                let height = span.min(level.height - y);
                neighbors.push(Region {
                    x,
                    y,
                    w: width.div_ceil(first.sampling),
                    h: height.div_ceil(first.sampling),
                    world: [
                        x as f32 / sx,
                        y as f32 / sy,
                        width as f32 / sx,
                        height as f32 / sy,
                    ],
                    ..first.clone()
                });
            }
        }
        neighbors.sort_by(|a, b| {
            let distance = |r: &Region| {
                (egui::vec2(r.world[0] + r.world[2] / 2.0, r.world[1] + r.world[3] / 2.0)
                    - self.center)
                    .length_sq()
            };
            distance(a).total_cmp(&distance(b))
        });
        neighbors.truncate(16);
        neighbors.retain(|r| {
            let cost = bytes(r);
            if cost > remaining {
                false
            } else {
                remaining -= cost;
                true
            }
        });
        neighbors
    }

    pub(super) fn next_tile(&self, wanted: &[Region]) -> Option<Region> {
        let missing =
            |r: &Region| !self.cached(r) && !self.failed.iter().any(|f| same_region(f, r));
        self.overview(self.plane)
            .filter(missing)
            .or_else(|| wanted.iter().find(|r| missing(r)).cloned())
            .or_else(|| self.adjacent_tiles(wanted).into_iter().find(missing))
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
                std::cmp::Reverse(((r.world[2] / r.w as f32) * 1000.0) as u64),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neighbor_prefetch_is_bounded_converges_and_preserves_foreground() {
        for mib in [48, 96, 256, 512] {
            let (mut viewer, _) = crate::tests::tile_viewer();
            viewer.cache_budget = mib * 1048576;
            viewer.slide.as_mut().unwrap().planes = 5;
            viewer.viewport_size = egui::vec2(1600.0, 900.0);
            viewer.zoom = 0.5;
            let ctx = egui::Context::default();
            let texture = ctx.load_texture(
                "tile",
                egui::ColorImage::new([1, 1], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            for step in 0..5 {
                viewer.center.x += 200.0 * step as f32;
                let wanted = viewer.visible_tiles(viewer.viewport_size);
                let neighbors = viewer.adjacent_tiles(&wanted);
                assert!(!neighbors.is_empty());
                for r in &neighbors {
                    assert!(!wanted.iter().any(|w| same_region(w, r)));
                    assert_eq!(r.sampling, wanted[0].sampling);
                    assert_eq!(r.plane, viewer.plane);
                    assert!(r.x < viewer.slide.as_ref().unwrap().levels[r.series].width);
                }
                let mut reads = 0;
                while let Some(region) = viewer.next_tile(&wanted) {
                    // Foreground always wins over speculative work.
                    if wanted.iter().any(|r| !viewer.cached(r)) {
                        assert!(
                            same_region(&region, &viewer.overview(viewer.plane).unwrap())
                                || wanted.iter().any(|r| same_region(r, &region))
                        );
                    }
                    viewer.cache.push(CachedRegion {
                        region,
                        texture: texture.clone(),
                    });
                    viewer.trim_cache();
                    reads += 1;
                    assert!(reads < 100, "Prefetch must not churn at {mib} MiB");
                }
                assert!(wanted.iter().all(|r| viewer.cached(r)));
                assert!(neighbors.iter().all(|r| viewer.cached(r)));
                assert!(viewer.cache_bytes() <= viewer.cache_budget);
            }
            viewer.preferences.prefetch_neighbors = false;
            assert!(viewer
                .adjacent_tiles(&viewer.visible_tiles(viewer.viewport_size))
                .is_empty());
        }
    }

    #[test]
    fn sparse_pyramid_keeps_screen_detail_and_bounds_tile_memory() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let slide = viewer.slide.as_mut().unwrap();
        slide.levels.retain(|l| l.series == 0 || l.series == 5);
        viewer.zoom = 0.1;
        viewer.pixel_ratio = 2.0;
        let tiles = viewer.visible_tiles(egui::vec2(1000.0, 700.0));
        assert!(!tiles.is_empty() && tiles.len() <= MAX_VISIBLE_TILES);
        for tile in &tiles {
            assert_eq!(tile.series, 0);
            assert!(tile.sampling > 1);
            assert!(tile.w <= TILE_SIZE && tile.h <= TILE_SIZE);
            assert!(tile.world[2] / tile.w as f32 * viewer.zoom * viewer.pixel_ratio <= 1.01);
            let mut other_level = tile.clone();
            other_level.sampling *= 2;
            assert!(!same_region(tile, &other_level));
        }
        viewer.cache_budget = 48 * 1048576;
        let tiles = viewer.visible_tiles(egui::vec2(3840.0, 2160.0));
        let bytes: usize = tiles.iter().map(|r| r.w as usize * r.h as usize * 4).sum();
        let overview = viewer.overview(0).unwrap();
        assert!(bytes + overview.w as usize * overview.h as usize * 4 <= viewer.cache_budget / 2);
    }
}
