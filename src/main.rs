#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod annotation_history;
mod annotations;
mod associated;
mod auto_contrast;
mod background;
mod diagnostics;
mod export_overlay;
mod library;
mod region_export;
#[cfg(test)]
use bioformats::ImageReader;
#[cfg(test)]
use reader::decode;
#[cfg(test)]
use reader::inspect;
use slide::{Level, Region, Slide};
use worker::{Reply, Request};
mod reader;
mod slide;
mod worker;
use eframe::egui;
use eframe::glow::HasContext;
use std::{path::PathBuf, sync::mpsc, time::Instant};

mod benchmark;
mod calibration;
mod channel_settings;
mod channels;
mod comparison;
mod display;
mod export_file;
mod features;
mod magnification;
mod navigation;
mod ndpa;
mod ndpis;
mod orientation;
mod polygon;
mod recent;
mod settings;
mod slide_info;
mod source;
#[cfg(test)]
mod synthetic;
mod tiles;
mod tracking;
mod updates;

const MAX_REGION: u32 = 1536;
const TILE_SIZE: u32 = 512;
const MAX_VISIBLE_TILES: usize = 40;

const DEFAULT_CACHE_BUDGET: usize = 512 * 1024 * 1024;

fn same_region(a: &Region, b: &Region) -> bool {
    (a.series, a.plane, a.x, a.y, a.w, a.h, a.preview)
        == (b.series, b.plane, b.x, b.y, b.w, b.h, b.preview)
}

struct CachedRegion {
    region: Region,
    texture: egui::TextureHandle,
}

struct Viewer {
    region_export: region_export::Export,
    associated: associated::Associated,
    pending_close: bool,
    channels: channels::State,
    tx: mpsc::SyncSender<Request>,
    rx: mpsc::Receiver<Reply>,
    busy: bool,
    cache_budget: usize,
    features: features::Features,
    decoded_tiles: u64,
    benchmark: Option<benchmark::Benchmark>,
    pending_open: Option<PathBuf>,
    slide: Option<Slide>,
    revision: u64,
    dirty: bool,
    center: egui::Vec2,
    zoom: f32,
    plane: u32,
    fit: bool,
    shown_plane: Option<u32>,
    slide_epoch: u64,
    cache: Vec<CachedRegion>,
    failed: Vec<Region>,
    status: String,
    viewport_size: egui::Vec2,
    renderer: String,
}
impl Viewer {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (kind, size) in [
            (egui::TextStyle::Body, 17.0),
            (egui::TextStyle::Button, 17.0),
            (egui::TextStyle::Small, 15.0),
            (egui::TextStyle::Heading, 22.0),
        ] {
            style
                .text_styles
                .insert(kind, egui::FontId::proportional(size));
        }
        style.wrap_mode = Some(egui::TextWrapMode::Wrap);
        style.visuals.override_text_color = Some(egui::Color32::from_gray(230));
        style.spacing.interact_size.y = 30.0;
        style.spacing.button_padding = egui::vec2(8.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        cc.egui_ctx.set_style(style);
        let (tx, rx) = worker::spawn(&cc.egui_ctx);
        Self {
            region_export: Default::default(),
            associated: Default::default(),
            pending_close: false,
            tx,
            rx,
            busy: false,
            cache_budget: DEFAULT_CACHE_BUDGET,
            features: features::Features::default(),
            channels: channels::State::default(),
            decoded_tiles: 0,
            benchmark: None,
            pending_open: None,
            slide: None,
            revision: 0,
            dirty: false,
            center: egui::Vec2::ZERO,
            zoom: 1.0,
            plane: 0,
            fit: false,
            shown_plane: None,
            slide_epoch: 0,
            cache: Vec::new(),
            failed: Vec::new(),
            status: "Open an NDPI slide to begin.".into(),
            viewport_size: egui::Vec2::ZERO,
            // eframe makes the OpenGL context current during app creation.
            renderer: cc
                .gl
                .as_ref()
                .map(|gl| {
                    // SAFETY: eframe supplies a current GL context on this UI thread.
                    unsafe { gl.get_parameter_string(eframe::glow::RENDERER) }
                })
                .unwrap_or_else(|| "Unknown renderer".into()),
        }
    }
    fn open(&mut self, path: PathBuf) {
        self.region_export.reset_selection();
        self.pending_close = false;
        self.features.save_tracking();
        self.save_channel_settings();
        self.revision += 1;
        self.slide_epoch = self.revision;
        self.pending_open = Some(path);
        self.slide = None;
        self.features = features::Features::default();
        self.channels = channels::State::default();

        self.shown_plane = None;
        self.cache.clear();
        self.failed.clear();
        self.plane = 0;
        self.dirty = false;
        self.status = "Opening slide…".into();
    }
    fn changed(&mut self) {
        self.revision += 1;
        self.dirty = true;
    }
    fn close(&mut self) {
        self.open(PathBuf::new());
        self.pending_open = None;
        self.pending_close = true;
        self.status = "Slide closed.".into();
    }
    fn poll(&mut self, ctx: &egui::Context) {
        self.region_export.poll();
        while let Ok(reply) = self.rx.try_recv() {
            self.busy = false;
            match reply {
                Reply::Auto(id, result) => self.finish_auto_contrast(id, result),
                Reply::Opened(id, result) if id == self.revision => match result {
                    Ok(slide) => {
                        if slide.path.is_dir() {
                            self.plane = slide.planes / 2;
                        }
                        self.features.load_annotations(&slide);
                        self.features.load_tracking(&slide);
                        self.channels.settings = self.channels.persistence.load(&slide);
                        self.channels.active = self.channels.settings.clone();
                        if !slide.rgb_components && slide.channels.len() > 1 {
                            self.prefer_channel_controls();
                        }
                        self.slide = Some(slide);
                        self.fit = true;
                        self.status = "Slide opened.".into();
                    }
                    Err(e) => self.status = format!("Could not open slide: {e}"),
                },
                // A completed read is still useful after a pan or Z change.
                // Only responses from a different slide must be discarded.
                Reply::Pixels(id, region, result)
                    if id >= self.slide_epoch && self.slide.is_some() =>
                {
                    match result {
                        Ok((pixels, ms)) => {
                            self.decoded_tiles += 1;
                            self.status = format!(
                                "{} × {} region · {:.1} ms · series {}",
                                region.w, region.h, ms, region.series
                            );
                            self.cache.push(CachedRegion {
                                texture: ctx.load_texture(
                                    "slide-region",
                                    pixels,
                                    egui::TextureOptions::LINEAR,
                                ),
                                region,
                            });
                            self.trim_cache();
                        }
                        Err(e) => {
                            self.status = format!("Read failed: {e}");
                            self.failed.push(region);
                            if self.failed.len() > 256 {
                                self.failed.remove(0);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn cached(&self, region: &Region) -> bool {
        self.cache.iter().any(|e| same_region(&e.region, region))
    }
    fn overview(&self, plane: u32) -> Option<Region> {
        let slide = self.slide.as_ref()?;
        let level = slide.levels.last()?;
        let preview = level.width > MAX_REGION || level.height > MAX_REGION;
        let factor = (1024.0 / level.width.max(level.height) as f64).min(1.0);
        Some(Region {
            preview,
            series: level.series,
            interleaved: level.interleaved,
            plane,
            x: 0,
            y: 0,
            w: if preview {
                (level.width as f64 * factor).round().max(1.0) as u32
            } else {
                level.width
            },
            h: if preview {
                (level.height as f64 * factor).round().max(1.0) as u32
            } else {
                level.height
            },
            world: [0.0, 0.0, slide.width as f32, slide.height as f32],
        })
    }
}
impl Viewer {
    fn choose_slide(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Hamamatsu NDPI / NDPIS", &["ndpi", "ndpis"])
            .pick_file()
        {
            self.open(path);
        }
    }

    fn finish(&mut self) {
        self.features.save_tracking();
        self.save_channel_settings();
    }
    fn controls(&mut self, ctx: &egui::Context, total_budget: &mut usize) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.horizontal_wrapped(|ui| {
                ui.heading("NDPI Viewer");
                ui.separator();
                if ui.button("Open slide…").clicked() {
                    self.choose_slide();
                }
                if ui.button("Open Z stack…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.open(path);
                    }
                }
                if ui
                    .add_enabled(self.slide.is_some(), egui::Button::new("Fit slide"))
                    .clicked()
                {
                    self.fit = true;
                }
                let previous_budget = *total_budget;
                egui::ComboBox::from_id_salt("cache-budget")
                    .selected_text(format!("Cache: {} MiB", *total_budget / 1048576))
                    .show_ui(ui, |ui| {
                        for mib in [96, 256, 512] {
                            ui.selectable_value(total_budget, mib * 1048576, format!("{mib} MiB"));
                        }
                    });
                if *total_budget != previous_budget && self.benchmark.is_none() {
                    if let Some(path) = settings::path() {
                        if let Err(error) = settings::save(&path, *total_budget) {
                            diagnostics::record(diagnostics::Event::SettingsSaveFailed);
                            self.status = format!("Could not save cache setting: {error}");
                        }
                    }
                }
                ui.label(if self.zoom < 0.01 {
                    format!("Zoom {:.2}%", self.zoom * 100.0)
                } else {
                    format!("Zoom {:.0}%", self.zoom * 100.0)
                });
                if self.busy {
                    ui.spinner();
                }
            });
            ui.horizontal_wrapped(|ui| {
                self.navigation_buttons(ui);
                self.orientation_menu(ui);
                self.magnification_menu(ui);
                if ui
                    .add_enabled(
                        self.slide.is_some(),
                        egui::Button::new("Glass slide / label"),
                    )
                    .clicked()
                {
                    self.associated.open = true;
                }
                if ui
                    .add_enabled(self.slide.is_some(), egui::Button::new("Slide info"))
                    .clicked()
                {
                    self.features.info_open = true;
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.busy {
                    ui.spinner();
                }
                if self.channels.auto_pending.is_some() || self.channels.auto_running.is_some() {
                    ui.label("Analyzing channel samples…");
                } else if self.channels.dirty_at.is_some() || self.channels.fallback.is_some() {
                    ui.label("Updating channel display…");
                } else if self.slide.is_some()
                    && !self.channels.settings.iter().any(|channel| channel.enabled)
                {
                    ui.label(
                        "All channels are hidden. Use Channels > Show all to restore the image.",
                    );
                } else if self.status.contains(" region · ") {
                    ui.label(if self.busy {
                        "Loading image detail…"
                    } else {
                        "Ready"
                    });
                } else {
                    ui.label(&self.status);
                }
                ui.separator();
                ui.menu_button("Performance", |ui| {
                    ui.label(format!("OpenGL · {}", self.renderer));
                    ui.label(format!(
                        "Active cache: {:.1} / {} MiB · {} tiles",
                        self.cache_bytes() as f64 / 1048576.0,
                        self.cache_budget / 1048576,
                        self.cache.len()
                    ));
                    ui.label(&self.status);
                });
                ui.menu_button("Help", |ui| {
                    ui.label(concat!("NDPI Viewer ", env!("CARGO_PKG_VERSION")));
                    ui.label(if diagnostics::available() { "Local error logging is enabled." } else { "Local error logging is unavailable." });
                    ui.label("Logs contain event codes only, never image names, paths or annotations. Nothing is uploaded.");
                    if ui.button("Open error logs folder").clicked() {
                        if let Some(path) = diagnostics::directory() {
                            if let Err(error) = std::process::Command::new("explorer.exe").arg(path).spawn() {
                                self.status = format!("Could not open log folder: {error}");
                            }
                        }
                    }
                    ui.separator();
                    ui.label("Scroll: zoom · Drag: pan · F: fit · Escape: Pan / select");
                    ui.label("Page Up / Down or Ctrl+wheel: focus Z");
                    ui.label("Ctrl+Z / Ctrl+Y: undo / redo annotations");
                    ui.label("W: overview / return · F11: full screen");
                    ui.label("Right-drag: zoom to region · B: scale bar");
                });
            });
        });
        egui::SidePanel::left("slide-info")
            .default_width(350.0)
            .min_width(320.0)
            .show(ctx, |ui| {
                self.inspector_panel(ui);
            });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let size = ui.available_size();
        let canvas = egui::Rect::from_min_size(ui.cursor().min, size);
        let over_navigator = self.navigator_rect(canvas).is_some_and(|map| {
            ctx.input(|i| {
                i.pointer.hover_pos().is_some_and(|p| map.contains(p))
                    || i.pointer.press_origin().is_some_and(|p| map.contains(p))
            })
        });
        let (rect, response) = ui.allocate_exact_size(
            size,
            if over_navigator {
                egui::Sense::hover()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        if rect.size() != self.viewport_size {
            self.viewport_size = rect.size();
            if self.slide.is_some() {
                self.changed();
            }
        }
        let temporary_pan = self.temporary_pan(&ctx);
        if temporary_pan {
            self.features.cancel_gesture();
        }
        let picked = if !temporary_pan
            && response.double_clicked()
            && self.features.tool == features::Tool::Pan
        {
            response
                .interact_pointer_pos()
                .and_then(|p| self.pick_annotation(p, rect))
        } else {
            None
        };
        if let Some(index) = picked {
            self.visit_annotation(index, true);
        }
        let selecting_region = !temporary_pan && self.select_export_region(ui, rect, &response);
        let editing_geometry = if selecting_region || temporary_pan {
            false
        } else {
            self.edit_geometry(ui, rect, &response)
        };
        if let Some(slide) = self.slide.as_ref() {
            let dimensions = egui::vec2(slide.width as f32, slide.height as f32);
            if response.hovered() && !over_navigator {
                let step = ctx.input(|i| magnification::focus_scroll(&i.events));
                let plane = (i64::from(self.plane) + i64::from(step))
                    .clamp(0, i64::from(slide.planes.saturating_sub(1)))
                    as u32;
                if plane != self.plane {
                    self.features.cancel_gesture();
                    self.plane = plane;
                    self.changed();
                }
            }
            if self.fit
                || (response.double_clicked()
                    && picked.is_none()
                    && self.features.tool == features::Tool::Pan)
            {
                self.center = dimensions * 0.5;
                self.zoom = (self.world_view_size(rect.size()).x / dimensions.x)
                    .min(self.world_view_size(rect.size()).y / dimensions.y)
                    * 0.95;
                self.fit = false;
                self.changed();
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && (self.features.tool == features::Tool::Pan || temporary_pan)
                && !editing_geometry
                && !selecting_region
            {
                self.center -= self.inverse_transform(ctx.input(|i| i.pointer.delta())) / self.zoom;
                self.changed();
            }
            if response.hovered() && !over_navigator {
                let scroll = ctx.input(|i| {
                    if i.modifiers.ctrl || magnification::focus_scroll(&i.events) != 0 {
                        0.0
                    } else {
                        i.smooth_scroll_delta.y
                    }
                });
                if scroll != 0.0 {
                    let pointer = ctx
                        .input(|i| i.pointer.hover_pos())
                        .unwrap_or(rect.center());
                    let offset = self.inverse_transform(pointer - rect.center());
                    let anchor = self.center + offset / self.zoom;
                    self.zoom = (self.zoom * (scroll * 0.003).exp()).clamp(0.0001, 8.0);
                    self.center = anchor - offset / self.zoom;
                    self.changed();
                }
            }
            self.prepare_benchmark();
            self.center = self.center.clamp(egui::Vec2::ZERO, dimensions);
            let wanted = self.visible_tiles(rect.size());
            self.draw_tiles(ui, rect, &wanted);
            self.track_displayed_view(&ctx, rect, &wanted);
            if !selecting_region {
                self.canvas_tools(ui, rect, &response);
            }
            self.paint_export_region(ui, rect);
            self.navigator_overlay(ui, rect);
            self.advance_benchmark(&ctx, &wanted);
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Open a slide to begin",
                egui::FontId::proportional(22.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        if !self.busy {
            let request = if std::mem::take(&mut self.pending_close) {
                Some(Request::Close)
            } else if let Some(path) = self.pending_open.take() {
                Some(Request::Open(self.revision, path))
            } else if let Some(index) = self.channels.auto_pending.take() {
                self.auto_contrast_request(index)
            } else if self.slide.is_some() {
                self.dirty = false;
                self.next_tile(&self.visible_tiles(rect.size()))
                    .map(|r| Request::Read(self.revision, r, self.channels.active.clone()))
            } else {
                None
            };
            if let Some(request) = request {
                if self.tx.send(request).is_ok() {
                    self.busy = true;
                } else {
                    diagnostics::record(diagnostics::Event::ReaderStopped);
                    self.status = "The slide reader stopped. Restart the application.".into();
                }
            }
        }
    }
}
fn main() -> eframe::Result {
    diagnostics::init();
    let result = eframe::run_native(
        "NDPI Viewer",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_icon(
                    eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
                        .expect("Embedded application icon must be a valid PNG"),
                )
                .with_inner_size([1200.0, 800.0])
                .with_min_inner_size([700.0, 600.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let mut viewer = Viewer::new(cc);
            if let Some(path) = settings::path() {
                viewer.cache_budget = settings::load(&path);
            }
            if let Some((benchmark, path, budget)) = benchmark::Benchmark::from_args() {
                viewer.cache_budget = budget;
                viewer.benchmark = Some(benchmark);
                viewer.open(path);
            }
            Ok(Box::new(comparison::Comparison::new(
                viewer,
                Viewer::new(cc),
            )))
        }),
    );
    diagnostics::record(if result.is_ok() {
        diagnostics::Event::Closed
    } else {
        diagnostics::Event::StartupFailed
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn tile_viewer() -> (Viewer, mpsc::Sender<Reply>) {
        let (tx, _) = mpsc::sync_channel(1);
        let (reply_tx, rx) = mpsc::channel();
        (
            Viewer {
                region_export: Default::default(),
                associated: Default::default(),
                pending_close: false,
                tx,
                rx,
                busy: false,
                cache_budget: DEFAULT_CACHE_BUDGET,
                features: features::Features::default(),
                channels: channels::State::default(),
                decoded_tiles: 0,
                benchmark: None,
                pending_open: None,
                slide: Some(Slide {
                    calibration: None,
                    channels: channels::rgb_channels(),
                    rgb_components: true,
                    channel_band: None,
                    registration: vec![],
                    ndpa: None,
                    path: PathBuf::new(),
                    width: 20000,
                    height: 16000,
                    planes: 1,
                    levels: [1, 2, 4, 8, 16, 32]
                        .iter()
                        .enumerate()
                        .map(|(series, d)| Level {
                            series,
                            interleaved: true,
                            width: 20000 / d,
                            height: 16000 / d,
                        })
                        .collect(),
                }),
                revision: 0,
                dirty: false,
                center: egui::vec2(10000.0, 8000.0),
                zoom: 1.0,
                plane: 0,
                fit: false,
                shown_plane: None,
                slide_epoch: 0,
                cache: vec![],
                failed: vec![],
                status: String::new(),
                viewport_size: egui::Vec2::ZERO,
                renderer: "Test".into(),
            },
            reply_tx,
        )
    }

    #[test]
    fn tile_grid_reuses_pan_overlap_and_covers_viewport() {
        let (mut viewer, _) = tile_viewer();
        let size = egui::vec2(1000.0, 700.0);
        let initial = viewer.visible_tiles(size);
        viewer.center.x += 20.0;
        let moved = viewer.visible_tiles(size);
        assert!(initial
            .iter()
            .all(|r| moved.iter().any(|s| same_region(r, s))));
        for zoom in [0.01, 0.04, 0.125, 0.5, 1.0, 8.0] {
            viewer.zoom = zoom;
            let tiles = viewer.visible_tiles(size);
            assert!(!tiles.is_empty() && tiles.len() <= MAX_VISIBLE_TILES);
            // Every sampled viewport point inside the image must be covered.
            for y in 0..8 {
                for x in 0..11 {
                    let p = viewer.center
                        + (egui::vec2(x as f32 / 10.0, y as f32 / 7.0) - egui::vec2(0.5, 0.5))
                            * (size - egui::Vec2::splat(1.0))
                            / zoom;
                    if p.x >= 0.0 && p.y >= 0.0 && p.x < 20000.0 && p.y < 16000.0 {
                        assert!(tiles.iter().any(|r| p.x >= r.world[0]
                            && p.y >= r.world[1]
                            && p.x < r.world[0] + r.world[2]
                            && p.y < r.world[1] + r.world[3]));
                    }
                }
            }
        }
    }

    #[test]
    fn cache_limits_each_level_and_all_levels_together() {
        let (mut viewer, _) = tile_viewer();
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "test",
            egui::ColorImage::new([1, 1], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let overview = viewer.overview(0).unwrap();
        viewer.cache.push(CachedRegion {
            region: overview.clone(),
            texture: texture.clone(),
        });
        for mib in [96, 256, 512, 96] {
            viewer.cache_budget = mib * 1048576;
            viewer.trim_cache();
            assert!(
                viewer.cache_bytes() <= viewer.cache_budget,
                "Shrinking applies immediately"
            );
            for series in 0..4 {
                for x in 0..300 {
                    viewer.cache.push(CachedRegion {
                        region: Region {
                            preview: false,
                            series,
                            interleaved: true,
                            plane: 0,
                            x: x * TILE_SIZE,
                            y: 0,
                            w: TILE_SIZE,
                            h: TILE_SIZE,
                            world: [0.0; 4],
                        },
                        texture: texture.clone(),
                    });
                    viewer.trim_cache();
                    assert!(viewer.cache_bytes() <= viewer.cache_budget);
                    let bytes: usize = viewer
                        .cache
                        .iter()
                        .filter(|e| e.region.series == series)
                        .map(|e| e.region.w as usize * e.region.h as usize * 4)
                        .sum();
                    assert!(bytes <= viewer.cache_budget / 2);
                }
            }
            assert!(
                viewer.cached(&overview),
                "Current overview must survive deep exploration"
            );
        }
    }

    #[test]
    #[ignore = "Requires the local large cytology sample"]
    fn cytology_zoom_round_trip_reuses_cached_tiles() {
        let (mut viewer, replies) = tile_viewer();
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images/02B0669241M.ndpi");
        let mut reader = ImageReader::open(&path).unwrap();
        let slide = inspect(&mut reader, path).unwrap();
        viewer.center = egui::vec2(slide.width as f32 * 0.5, slide.height as f32 * 0.5);
        viewer.slide = Some(slide);
        let ctx = egui::Context::default();
        let size = egui::vec2(1000.0, 700.0);
        for (step, zoom) in [0.005, 0.04, 0.125, 0.5, 1.0, 0.5, 0.125, 0.04, 0.005]
            .into_iter()
            .enumerate()
        {
            viewer.zoom = zoom;
            let wanted = viewer.visible_tiles(size);
            let mut reads = 0;
            while let Some(region) = viewer.next_tile(&wanted) {
                let result = decode(&mut reader, &region).unwrap();
                replies.send(Reply::Pixels(0, region, Ok(result))).unwrap();
                viewer.poll(&ctx);
                reads += 1;
                assert!(reads <= MAX_VISIBLE_TILES + 1, "Scheduler must converge");
            }
            eprintln!(
                "zoom={zoom} reads={reads} cache_mib={:.1}",
                viewer.cache_bytes() as f64 / 1048576.0
            );
            if step >= 5 {
                assert_eq!(reads, 0, "Returning to a cached zoom must not decode again");
            }
            assert!(viewer.cache_bytes() <= viewer.cache_budget);
        }
    }

    #[test]
    fn viewport_requests_are_bounded_and_inside_the_slide() {
        let (tx, _) = mpsc::sync_channel(1);
        let (reply_tx, rx) = mpsc::channel();
        let slide = Slide {
            calibration: None,
            channels: channels::rgb_channels(),
            rgb_components: true,
            channel_band: None,
            registration: vec![],
            ndpa: None,
            path: PathBuf::new(),
            width: 20000,
            height: 16000,
            planes: 5,
            levels: [1, 2, 4, 8, 16, 32]
                .iter()
                .enumerate()
                .map(|(series, d)| Level {
                    series,
                    interleaved: true,
                    width: 20000 / d,
                    height: 16000 / d,
                })
                .collect(),
        };
        let mut viewer = Viewer {
            region_export: Default::default(),
            associated: Default::default(),
            pending_close: false,
            tx,
            rx,
            busy: false,
            cache_budget: DEFAULT_CACHE_BUDGET,
            features: features::Features::default(),
            channels: channels::State::default(),
            decoded_tiles: 0,
            benchmark: None,
            pending_open: None,
            slide: Some(slide),
            revision: 0,
            dirty: false,
            center: egui::vec2(10000.0, 8000.0),
            zoom: 1.0,
            plane: 4,
            fit: false,
            shown_plane: None,
            slide_epoch: 0,
            cache: Vec::new(),
            failed: Vec::new(),
            status: String::new(),
            viewport_size: egui::Vec2::ZERO,
            renderer: "Test".into(),
        };
        for zoom in [0.02, 0.05, 0.3, 1.0, 8.0] {
            viewer.zoom = zoom;
            for center in [
                egui::Vec2::ZERO,
                egui::vec2(10000.0, 8000.0),
                egui::vec2(20000.0, 16000.0),
            ] {
                viewer.center = center;
                for r in viewer.visible_tiles(egui::vec2(1920.0, 1080.0)) {
                    let l = &viewer.slide.as_ref().unwrap().levels[r.series];
                    assert!(r.w <= MAX_REGION && r.h <= MAX_REGION);
                    assert!(r.x + r.w <= l.width && r.y + r.h <= l.height);
                    assert_eq!(r.plane, 4);
                    assert_eq!(r.x % TILE_SIZE, 0);
                    assert_eq!(r.y % TILE_SIZE, 0);
                }
            }
        }
        let ctx = egui::Context::default();
        let wanted = viewer.visible_tiles(egui::vec2(1920.0, 1080.0))[0].clone();
        viewer.revision = 10;
        viewer.slide_epoch = 1;
        let pixels = egui::ColorImage::from_rgb([1, 1], &[10, 20, 30]);
        reply_tx
            .send(Reply::Pixels(2, wanted.clone(), Ok((pixels.clone(), 1.0))))
            .unwrap();
        viewer.poll(&ctx);
        assert!(
            viewer.cached(&wanted),
            "Same-slide replies survive viewport changes"
        );
        viewer.shown_plane = Some(4);
        viewer.plane = 3;
        viewer.changed();
        assert_eq!(viewer.shown_plane, Some(4));
        viewer.channels.settings = channels::defaults(&channels::rgb_channels());
        viewer.channels.settings[0].enabled = false;
        viewer.channels.dirty_at = Some(Instant::now() - std::time::Duration::from_millis(200));
        viewer.apply_channel_changes(&ctx);
        assert!(!viewer.channels.active[0].enabled);
        assert!(viewer.cache.is_empty());
        reply_tx
            .send(Reply::Pixels(2, wanted.clone(), Ok((pixels.clone(), 1.0))))
            .unwrap();
        viewer.poll(&ctx);
        assert!(
            viewer.cache.is_empty(),
            "Old channel composites must be discarded"
        );
        reply_tx
            .send(Reply::Pixels(
                viewer.revision,
                wanted.clone(),
                Ok((pixels.clone(), 1.0)),
            ))
            .unwrap();
        viewer.poll(&ctx);
        assert!(
            viewer.cached(&wanted),
            "Current channel composites should be accepted"
        );
        viewer.open(PathBuf::from("another.ndpi"));
        reply_tx
            .send(Reply::Pixels(2, wanted, Ok((pixels, 1.0))))
            .unwrap();
        viewer.poll(&ctx);
        assert!(
            viewer.cache.is_empty(),
            "Old-slide replies must not populate the new cache"
        );
        assert!(viewer.shown_plane.is_none());
    }

    #[test]
    #[ignore = "Requires the local large cytology sample"]
    fn large_overview_samples_across_tile_boundaries() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images/02B0669241M.ndpi");
        let mut reader = ImageReader::open(&path).unwrap();
        let slide = inspect(&mut reader, path).unwrap();
        let level = slide.levels.last().unwrap();
        assert!(level.width > MAX_REGION && level.height > MAX_REGION);
        let region = Region {
            preview: true,
            series: level.series,
            interleaved: level.interleaved,
            plane: 0,
            x: 0,
            y: 0,
            w: 1024,
            h: 520,
            world: [0.0; 4],
        };
        let (overview, _) = decode(&mut reader, &region).unwrap();
        assert_eq!(overview.size, [1024, 520]);
        // Check edges and both sides of source-tile boundaries against direct reads.
        for oy in [0, 143, 144, 287, 288, 519] {
            for ox in [0, 143, 144, 287, 288, 1023] {
                let pixel = Region {
                    preview: false,
                    x: ox * level.width / region.w,
                    y: oy * level.height / region.h,
                    w: 1,
                    h: 1,
                    ..region.clone()
                };
                let (expected, _) = decode(&mut reader, &pixel).unwrap();
                assert_eq!(
                    overview.pixels[(oy * region.w + ox) as usize],
                    expected.pixels[0]
                );
            }
        }
    }

    #[test]
    #[ignore = "Requires local public NDPI samples"]
    fn public_samples_decode_all_viewer_levels_and_z_planes() {
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images");
        let mut checked = 0;
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ndpi"))
            {
                continue;
            }
            let mut reader = ImageReader::open(&path).unwrap();
            let slide = inspect(&mut reader, path.clone()).unwrap();
            use bioformats::FormatReader;
            let mut reference = bioformats::tiff::TiffReader::new();
            reference.set_ndpi_64bit(std::fs::metadata(&path).unwrap().len() >= (1u64 << 32));
            reference.set_id(&path).unwrap();
            let mut plane_sums = Vec::new();
            for level in &slide.levels {
                for plane in 0..slide.planes {
                    let w = 128.min(level.width);
                    let h = 128.min(level.height);
                    let r = Region {
                        preview: false,
                        series: level.series,
                        interleaved: level.interleaved,
                        plane,
                        x: (level.width - w) / 2,
                        y: (level.height - h) / 2,
                        w,
                        h,
                        world: [0.0; 4],
                    };
                    let (image, _) = decode(&mut reader, &r).unwrap();
                    assert_eq!(image.pixels.len(), w as usize * h as usize);
                    if plane == 0 {
                        // Bypass the NDPI wrapper to catch its layout/metadata mismatch.
                        // This is a layout regression check, not an independent decoder.
                        let index = (0..reference.ifd_count())
                            .find(|&i| {
                                let ifd = reference.ifd(i).unwrap();
                                ifd.image_width() == Some(level.width)
                                    && ifd.image_length() == Some(level.height)
                            })
                            .unwrap();
                        let planar = reference
                            .read_physical_ifd_region(index, r.x, r.y, w, h)
                            .unwrap();
                        let count = w as usize * h as usize;
                        assert_eq!(planar.len(), count * 3);
                        for (i, pixel) in image.pixels.iter().enumerate() {
                            assert_eq!(
                                [pixel.r(), pixel.g(), pixel.b()],
                                [planar[i], planar[count + i], planar[2 * count + i]]
                            );
                        }
                    }
                    if level.series == 0 {
                        plane_sums.push(
                            image
                                .pixels
                                .iter()
                                .map(|p| p.r() as u64 + p.g() as u64 + p.b() as u64)
                                .sum::<u64>(),
                        );
                    }
                    checked += 1;
                }
            }
            if slide.planes > 1 {
                assert!(
                    plane_sums.windows(2).any(|w| w[0] != w[1]),
                    "Z planes should decode different sample pixels"
                );
            }
        }
        assert!(checked >= 120);
    }
}
