//! Opt-in deterministic navigation through the real desktop renderer.
use super::*;
use std::io::Write;

pub(super) struct Benchmark {
    output: std::fs::File,
    views: Vec<(f32, f32, f32)>,
    index: usize,
    started: Option<Instant>,
    reads: u64,
    ready_frames: usize,
}

impl Benchmark {
    #[cfg(not(feature = "development-tools"))]
    pub(super) fn from_args() -> Option<(Self, PathBuf, usize)> {
        None
    }

    #[cfg(feature = "development-tools")]
    pub(super) fn from_args() -> Option<(Self, PathBuf, usize)> {
        let args: Vec<_> = std::env::args_os().collect();
        if args.get(1).is_none_or(|a| a != "--cache-benchmark") {
            return None;
        }
        assert_eq!(
            args.len(),
            5,
            "Usage: --cache-benchmark SLIDE CACHE_MIB OUTPUT.csv"
        );
        let mib: usize = args[3]
            .to_string_lossy()
            .parse()
            .expect("Invalid cache size");
        assert!(
            [96, 256, 512].contains(&mib),
            "Cache must be 96, 256 or 512 MiB"
        );
        let mut output = std::fs::File::create(&args[4]).expect("Cannot create benchmark output");
        writeln!(output, "stage,phase,zoom,elapsed_ms,reads,cache_bytes").unwrap();
        let mut views = vec![];
        // 24 separated locations at three resolutions, followed by an exact
        // reverse traversal. The working set deliberately exceeds small caches.
        for zoom in [0.125, 0.5, 1.0] {
            for y in 0..4 {
                for x in 0..6 {
                    views.push((0.15 + x as f32 * 0.14, 0.2 + y as f32 * 0.2, zoom));
                }
            }
        }
        let reverse: Vec<_> = views.iter().rev().copied().collect();
        views.extend(reverse);
        Some((
            Self {
                output,
                views,
                index: 0,
                started: None,
                reads: 0,
                ready_frames: 0,
            },
            PathBuf::from(&args[2]),
            mib * 1048576,
        ))
    }
}

impl Viewer {
    pub(super) fn prepare_benchmark(&mut self) {
        let Some(b) = &mut self.benchmark else {
            return;
        };
        let Some(slide) = &self.slide else {
            return;
        };
        if b.index >= b.views.len() {
            return;
        }
        if b.started.is_none() {
            b.started = Some(Instant::now());
            b.reads = self.decoded_tiles;
            b.ready_frames = 0;
        }
        let (x, y, zoom) = b.views[b.index];
        self.center = egui::vec2(x * slide.width as f32, y * slide.height as f32);
        self.zoom = zoom;
    }

    pub(super) fn advance_benchmark(&mut self, ctx: &egui::Context, wanted: &[Region]) {
        if self.benchmark.is_none() {
            return;
        }
        let ready = !self.busy
            && self.overview(self.plane).is_some_and(|r| self.cached(&r))
            && wanted.iter().all(|r| self.cached(r));
        let bytes = self.cache_bytes();
        let b = self.benchmark.as_mut().unwrap();
        if b.index >= b.views.len() {
            return;
        }
        if !self.failed.is_empty() || b.started.is_some_and(|s| s.elapsed().as_secs() > 60) {
            writeln!(b.output, "ERROR,read failure or stage timeout").unwrap();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ready {
            // A second frame lets the renderer consume newly uploaded textures.
            b.ready_frames += 1;
            if b.ready_frames >= 2 {
                writeln!(
                    b.output,
                    "{},{},{},{:.3},{},{}",
                    b.index,
                    if b.index < b.views.len() / 2 {
                        "outbound"
                    } else {
                        "return"
                    },
                    b.views[b.index].2,
                    b.started.unwrap().elapsed().as_secs_f64() * 1000.0,
                    self.decoded_tiles - b.reads,
                    bytes
                )
                .unwrap();
                b.output.flush().unwrap();
                b.index += 1;
                b.started = None;
                if b.index == b.views.len() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        ctx.request_repaint();
    }
}
