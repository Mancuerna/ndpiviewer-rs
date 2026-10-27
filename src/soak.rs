//! Opt-in lifecycle stress test using public samples; no patient data or screenshots.
#[cfg(test)]
mod tests {
    use crate::*;
    #[test]
    #[ignore = "Long-running public-sample test; NDPI_SOAK_SECONDS defaults to 120"]
    fn public_sample_lifecycle_soak() {
        let seconds = std::env::var("NDPI_SOAK_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120)
            .clamp(1, 86400);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images");
        let paths = [
            root.join("JNOC-GC2003.ndpi"),
            root.join("02B0669241M.ndpi"),
            root.join("Hamamatsu_fluorescence_Manuel_Stritt/test3.ndpis"),
        ];
        assert!(
            paths.iter().all(|p| p.is_file()),
            "Public test samples are required"
        );
        let ctx = egui::Context::default();
        let (tx, rx) = worker::spawn(&ctx);
        let started = Instant::now();
        let mut cycle = 0u64;
        let mut reads = 0u64;
        while started.elapsed().as_secs() < seconds {
            let path = paths[cycle as usize % paths.len()].clone();
            tx.send(Request::Open(cycle, path)).unwrap();
            let slide = match rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap() {
                Reply::Opened(_, Ok(s)) => s,
                _ => panic!("Open failed"),
            };
            for (index, level) in slide.levels.iter().enumerate() {
                let w = level.width.min(192);
                let h = level.height.min(192);
                for plane in [0, slide.planes - 1] {
                    let r = Region {
                        sampling: 1,
                        preview: false,
                        series: level.series,
                        interleaved: level.interleaved,
                        plane,
                        x: (level.width - w) / 2,
                        y: (level.height - h) / 2,
                        w,
                        h,
                        world: [0.0, 0.0, w as f32, h as f32],
                    };
                    let mut settings = channels::defaults(&slide.channels);
                    if !slide.rgb_components && index % 2 == 1 {
                        settings[0].enabled = false;
                    }
                    tx.send(Request::Read(cycle, r, settings)).unwrap();
                    let image = match rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap() {
                        Reply::Pixels(_, _, Ok((image, _))) => image,
                        _ => panic!("Read failed"),
                    };
                    assert_eq!(image.pixels.len(), w as usize * h as usize);
                    let bytes: Vec<_> = image
                        .pixels
                        .iter()
                        .flat_map(|p| p.to_srgba_unmultiplied())
                        .collect();
                    let mut crop = image::RgbaImage::from_raw(w, h, bytes).unwrap();
                    crate::export_overlay::paint(
                        &mut crop,
                        &slide,
                        [0, 0],
                        &[],
                        crate::export_overlay::Style {
                            scale: true,
                            labels: true,
                            annotation: [2.0, 18.0],
                        },
                        &std::sync::atomic::AtomicBool::new(false),
                    )
                    .unwrap();
                    let mut png = std::io::Cursor::new(Vec::new());
                    crop.write_to(&mut png, image::ImageFormat::Png).unwrap();
                    assert!(image::load_from_memory(png.get_ref()).is_ok());
                    reads += 1;
                }
            }
            tx.send(Request::Close).unwrap();
            assert!(matches!(
                rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
                Reply::Closed
            ));
            cycle += 1;
            eprintln!(
                "soak seconds={:.1} open_close_cycles={cycle} decoded_exported_tiles={reads}",
                started.elapsed().as_secs_f64()
            );
        }
        assert!(cycle > 0);
    }
}
