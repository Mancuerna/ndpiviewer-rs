use super::*;
#[test]
fn native_export_matches_source_across_tiles_and_cancel_preserves_file() {
    let root = std::env::temp_dir().join(format!("native-region-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let stack = root.join("stack");
    std::fs::create_dir_all(&stack).unwrap();
    let original = image::RgbImage::from_fn(1100, 800, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
    });
    original.save(stack.join("0.png")).unwrap();
    let (_, slide) = Source::open(stack).unwrap();
    let destination = root.join("crop.png");
    let job = || Job {
        slide: slide.clone(),
        area: [10, 20, 700, 600],
        plane: 0,
        settings: channels::defaults(&slide.channels),
        annotations: vec![],
        scale: false,
        labels: false,
        annotation_style: [2.0, 18.0],
        path: destination.clone(),
    };
    let cancelled = AtomicBool::new(false);
    let progress = AtomicU32::new(0);
    export(job(), &cancelled, &progress).unwrap();
    let result = image::open(&destination).unwrap().to_rgb8();
    assert_eq!(result.dimensions(), (700, 600));
    for (x, y, pixel) in result.enumerate_pixels() {
        assert_eq!(pixel, original.get_pixel(x + 10, y + 20));
    }
    let bytes = std::fs::read(&destination).unwrap();
    cancelled.store(true, Ordering::Relaxed);
    assert!(export(job(), &cancelled, &progress)
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(bytes, std::fs::read(&destination).unwrap());
    let mut invalid = job();
    invalid.area = [0, 0, u32::MAX, 10];
    assert!(export(invalid, &AtomicBool::new(false), &progress).is_err());
    assert_eq!(
        bounds(
            egui::Rect::from_min_max(egui::pos2(-5.0, -2.0), egui::pos2(10.3, 20.2)),
            &slide
        ),
        Some([0, 0, 11, 21])
    );
}

#[test]
fn exported_overlays_include_scale_text_and_source_geometry() {
    let mut slide = crate::ndpa::test_slide();
    slide.calibration = Some(crate::calibration::Calibration { x: 0.25, y: 0.25 });
    let mut image = image::RgbaImage::from_pixel(640, 480, image::Rgba([40, 40, 40, 255]));
    let annotation = crate::annotations::Annotation {
        appearance: Default::default(),
        tool: crate::annotations::Tool::Line,
        points: vec![[110.0, 220.0], [410.0, 220.0]],
        plane: 0,
        note: "Test".into(),
        origin: None,
    };
    crate::export_overlay::paint(
        &mut image,
        &slide,
        [10, 20],
        &[annotation],
        crate::export_overlay::Style {
            scale: true,
            labels: true,
            annotation: [2.0, 18.0],
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_ne!(image.get_pixel(250, 200).0, [40, 40, 40, 255]);
    let white_scale_pixels = (408..470)
        .flat_map(|y| (16..220).map(move |x| (x, y)))
        .filter(|&(x, y)| image.get_pixel(x, y)[0] > 200)
        .count();
    assert!(
        white_scale_pixels > 100,
        "Scale line and glyphs should be visible"
    );
    assert_eq!(image.get_pixel(600, 50).0, [40, 40, 40, 255]);
    let path = std::env::temp_dir().join("native-export-overlay-check.png");
    image.save(path).unwrap();
}

#[test]
#[ignore = "Requires public NDPI and NDPIS samples"]
fn public_slides_export_original_resolution_and_embedded_images() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images");
    for name in [
        "JNOC-GC2003.ndpi",
        "Hamamatsu_fluorescence_Manuel_Stritt/test3.ndpis",
    ] {
        let path = root.join(name);
        let (mut source, slide) = Source::open(path.clone()).unwrap();
        let area = [slide.width / 2, slide.height / 2, 600, 550];
        let plane = slide.planes - 1;
        let settings = channels::defaults(&slide.channels);
        let destination = std::env::temp_dir().join(format!(
            "native-{}.png",
            path.file_stem().unwrap().to_string_lossy()
        ));
        export(
            Job {
                slide: slide.clone(),
                area,
                plane,
                settings: settings.clone(),
                annotations: vec![],
                scale: false,
                labels: false,
                annotation_style: [2.0, 18.0],
                path: destination.clone(),
            },
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )
        .unwrap();
        let expected = source
            .read_display(
                &Region {
                    sampling: 1,
                    preview: false,
                    series: slide.levels[0].series,
                    interleaved: slide.levels[0].interleaved,
                    plane,
                    x: area[0],
                    y: area[1],
                    w: area[2],
                    h: area[3],
                    world: [0.0; 4],
                },
                &settings,
            )
            .unwrap()
            .0;
        let actual = image::open(destination).unwrap().to_rgba8();
        for (pixel, expected) in actual.pixels().zip(expected.pixels) {
            assert_eq!(pixel.0, expected.to_array());
        }
        let thumbnail = crate::library::thumbnail(path.clone()).unwrap();
        assert!(thumbnail.size[0] <= 192 && thumbnail.size[1] <= 192);
        let associated = crate::associated::load(path).unwrap();
        assert!(!associated.is_empty());
        for (title, image) in associated {
            println!("{name}: {title}, preview {:?}", image.size);
            assert!(image.size[0] <= 1024 && image.size[1] <= 1024);
        }
    }
}
