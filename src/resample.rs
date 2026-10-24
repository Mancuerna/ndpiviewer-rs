//! Area-filtered virtual pyramid tiles with bounded source reads.
use crate::{Region, MAX_REGION};
use eframe::egui;

pub(crate) fn tile(
    region: &Region,
    dimensions: [u32; 2],
    mut read: impl FnMut(u32, u32, u32, u32) -> Result<egui::ColorImage, String>,
) -> Result<egui::ColorImage, String> {
    let r = region;
    if r.sampling == 0
        || r.w == 0
        || r.h == 0
        || r.w > MAX_REGION
        || r.h > MAX_REGION
        || r.x >= dimensions[0]
        || r.y >= dimensions[1]
    {
        return Err("Invalid sampled tile.".into());
    }
    let width = r.w.saturating_mul(r.sampling).min(dimensions[0] - r.x);
    let height = r.h.saturating_mul(r.sampling).min(dimensions[1] - r.y);
    if width.div_ceil(r.sampling) != r.w || height.div_ceil(r.sampling) != r.h {
        return Err("Sampled tile extends outside the image.".into());
    }
    let mut sums = vec![[0_u64; 4]; r.w as usize * r.h as usize];
    for y in (0..height).step_by(MAX_REGION as usize) {
        for x in (0..width).step_by(MAX_REGION as usize) {
            let w = MAX_REGION.min(width - x);
            let h = MAX_REGION.min(height - y);
            let image = read(r.x + x, r.y + y, w, h)?;
            if image.size != [w as usize, h as usize] {
                return Err("Unexpected sampled source dimensions.".into());
            }
            for py in 0..h {
                let oy = (y + py) / r.sampling;
                for px in 0..w {
                    let ox = (x + px) / r.sampling;
                    let sum = &mut sums[(oy * r.w + ox) as usize];
                    let p = image.pixels[(py * w + px) as usize];
                    sum[0] += u64::from(p.r());
                    sum[1] += u64::from(p.g());
                    sum[2] += u64::from(p.b());
                    sum[3] += 1;
                }
            }
        }
    }
    let pixels = sums
        .into_iter()
        .map(|s| {
            let channel = |i| ((s[i] + s[3] / 2) / s[3]) as u8;
            egui::Color32::from_rgb(channel(0), channel(1), channel(2))
        })
        .collect();
    Ok(egui::ColorImage {
        size: [r.w as usize, r.h as usize],
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn area_filter_preserves_detail_across_source_chunks_and_partial_edges() {
        let region = Region {
            sampling: 4,
            preview: false,
            series: 0,
            interleaved: true,
            plane: 0,
            x: 10,
            y: 20,
            w: 400,
            h: 2,
            world: [0.0; 4],
        };
        let mut calls = 0;
        let image = tile(&region, [1609, 27], |x, y, w, h| {
            calls += 1;
            assert!(w <= MAX_REGION && h <= MAX_REGION);
            let pixels = (y..y + h)
                .flat_map(|py| {
                    (x..x + w).map(move |px| {
                        if (px + py) % 2 == 0 {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::BLACK
                        }
                    })
                })
                .collect();
            Ok(egui::ColorImage {
                size: [w as usize, h as usize],
                pixels,
            })
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(image.size, [400, 2]);
        for pixel in &image.pixels[..399] {
            assert_eq!(*pixel, egui::Color32::from_gray(128));
        }
        assert_eq!(image.pixels[799], egui::Color32::from_gray(142));
        assert!(tile(&region, [5, 5], |_, _, _, _| unreachable!()).is_err());
        assert!(tile(&region, [1610, 28], |_, _, _, _| Err("read failed".into())).is_err());
    }
}
