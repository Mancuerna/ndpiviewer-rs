use super::*;

#[derive(Clone, Copy)]
struct Position {
    center: egui::Vec2,
    zoom: f32,
    rotation: u8,
    mirrored: [bool; 2],
}

impl Position {
    fn of(viewer: &Viewer) -> Self {
        Self {
            center: viewer.center,
            zoom: viewer.zoom,
            rotation: viewer.features.rotation,
            mirrored: viewer.features.mirrored,
        }
    }
}

/// Preserve the manually chosen correspondence between the two views.
/// Zoom is relative; pan distances are matched in screen pixels, accounting
/// for each image's independent rotation and current pixel zoom.
fn linked_position(before: Position, after: Position, target: Position) -> Position {
    let zoom = (target.zoom * (after.zoom / before.zoom)).clamp(0.0001, 8.0);
    let screen_delta = orientation::forward(
        (after.center - before.center) * after.zoom,
        after.rotation,
        after.mirrored,
    );
    Position {
        center: target.center
            + orientation::inverse(screen_delta, target.rotation, target.mirrored) / zoom,
        zoom,
        ..target
    }
}

pub(super) struct Comparison {
    preferences_save_at: Option<Instant>,
    updates: updates::Updates,
    library: crate::library::Library,
    tabs: [Vec<PathBuf>; 2],
    viewers: [Viewer; 2],
    split: bool,
    linked: bool,
    active: usize,
    total_budget: usize,
    pane_rects: [egui::Rect; 2],
    recent: recent::Recent,
    recorded_epochs: [u64; 2],
}

impl Comparison {
    pub(super) fn new(left: Viewer, right: Viewer) -> Self {
        Self {
            preferences_save_at: None,
            updates: Default::default(),
            library: Default::default(),
            tabs: Default::default(),
            total_budget: left.cache_budget,
            viewers: [left, right],
            split: false,
            linked: false,
            active: 0,
            pane_rects: [egui::Rect::NOTHING; 2],
            recent: recent::Recent::load(),
            recorded_epochs: [0; 2],
        }
    }

    fn save_preferences(&mut self) {
        if self.viewers[0].benchmark.is_none() {
            if let Some(path) = settings::Preferences::path() {
                if let Err(error) = self.viewers[self.active].preferences.save(&path) {
                    self.viewers[self.active].status =
                        format!("Could not save navigation preferences: {error}");
                    diagnostics::record(diagnostics::Event::SettingsSaveFailed);
                }
            }
        }
        self.preferences_save_at = None;
    }
    fn budgets(&mut self) {
        // Hidden comparisons retain their state and still count against the cap.
        let two =
            self.split || self.viewers[1].slide.is_some() || self.viewers[1].pending_open.is_some();
        let budget = if two {
            self.total_budget / 2
        } else {
            self.total_budget
        };
        for (index, viewer) in self.viewers.iter_mut().enumerate() {
            viewer.cache_budget = if index == 0 || two { budget } else { 0 };
            viewer.trim_cache();
        }
    }

    fn synchronize(&mut self, before: Position) {
        let source = Position::of(&self.viewers[self.active]);
        if source.center == before.center && source.zoom == before.zoom {
            return;
        }
        let peer = &mut self.viewers[1 - self.active];
        let Some(slide) = &peer.slide else { return };
        let target = linked_position(before, source, Position::of(peer));
        let center = target.center.clamp(
            egui::Vec2::ZERO,
            egui::vec2(slide.width as f32, slide.height as f32),
        );
        if center != peer.center || target.zoom != peer.zoom {
            peer.features.cancel_gesture();
            peer.center = center;
            peer.zoom = target.zoom;
            peer.fit = false;
            peer.changed();
        }
    }
}

impl eframe::App for Comparison {
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.save_preferences();
        for viewer in &mut self.viewers {
            viewer.finish();
        }
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let preferences_before = self.viewers[self.active].preferences;
        ctx.set_zoom_factor(preferences_before.ui_scale);
        self.library
            .set_label_mode(preferences_before.label_thumbnails);
        let previous_active = self.active;
        for viewer in &mut self.viewers {
            viewer.poll(ctx);
            viewer.handle_export(ctx);
        }
        if self.viewers[0].benchmark.is_none() {
            for (index, viewer) in self.viewers.iter().enumerate() {
                if viewer.slide_epoch != self.recorded_epochs[index] {
                    if let Some(slide) = &viewer.slide {
                        self.recent.record(&slide.path);
                        self.library.follow_slide(ctx, &slide.path);
                        if !self.tabs[index].contains(&slide.path) {
                            if self.tabs[index].len() >= 32 {
                                self.tabs[index].remove(0);
                            }
                            self.tabs[index].push(slide.path.clone());
                        }
                        self.recorded_epochs[index] = viewer.slide_epoch;
                    }
                }
            }
        }
        // Popup menus overlap the canvases. Selecting a menu item must keep
        // the intended target pane rather than activate the image underneath.
        if self.split && !ctx.memory(|memory| memory.any_popup_open()) {
            if let Some(pos) = ctx.input(|i| {
                (i.pointer.any_pressed()
                    || i.smooth_scroll_delta.y != 0.0
                    || i.events
                        .iter()
                        .any(|event| matches!(event, egui::Event::MouseWheel { .. })))
                .then(|| i.pointer.interact_pos())
                .flatten()
            }) {
                if ctx
                    .layer_id_at(pos)
                    .is_some_and(|layer| layer.order == egui::Order::Background)
                {
                    for index in 0..2 {
                        if self.pane_rects[index].contains(pos) {
                            if self.active != index {
                                self.viewers[self.active].features.cancel_gesture();
                            }
                            self.active = index;
                        }
                    }
                }
            }
        }

        if self.library.open {
            let viewer = &self.viewers[self.active];
            let current = viewer
                .pending_open
                .as_deref()
                .or_else(|| viewer.slide.as_ref().map(|s| s.path.as_path()));
            if let Some(path) = self.library.cycle_shortcut(ctx, current) {
                self.viewers[self.active].open(path);
            }
        } else {
            self.viewers[self.active].handle_overview_shortcut(ctx);
        }
        let benchmark = self.viewers[0].benchmark.is_some();
        if !benchmark {
            self.updates.show(ctx);
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            if !dropped.is_empty() {
                if dropped.len() == 1 {
                    if let Some(path) = &dropped[0].path {
                        self.viewers[self.active].open(path.clone());
                    }
                } else {
                    self.viewers[self.active].status =
                        "Drop one slide or stack folder at a time into the selected image.".into();
                }
            }
            let mut fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11)) {
                fullscreen = !fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
            }
            if fullscreen
                && !ctx.wants_keyboard_input()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            egui::TopBottomPanel::top("comparison-controls").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Slide library…").clicked() { self.library.open = true; }
                    ui.menu_button("Recent images", |ui| {
                        let mut selected = None;
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for path in &self.recent.paths {
                                if ui.button(path.display().to_string()).clicked() {
                                    selected = Some(path.clone());
                                }
                            }
                        });
                        if self.recent.paths.is_empty() { ui.label("No recent images"); }
                        if let Some(path) = selected {
                            self.viewers[self.active].open(path);
                            ui.close_menu();
                        }
                        if ui.add_enabled(!self.recent.paths.is_empty(), egui::Button::new("Clear recent list")).clicked() {
                            self.recent.clear();
                            ui.close_menu();
                        }
                        if let Some(error) = &self.recent.error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                    });
                    ui.menu_button("View", |ui| {
                    if ui.checkbox(&mut fullscreen, "Full screen").on_hover_text("F11 to toggle; Escape to leave full screen.").changed() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
                    }
                    if ui.checkbox(&mut self.split, "Side-by-side").changed() {
                        if !self.split {
                            self.active = 0;
                            self.viewers[1].finish();
                        } else if self.viewers[0].slide.is_some() && self.viewers[1].slide.is_none() {
                            self.active = 1;
                        }
                    }
                    });
                    self.viewers[self.active].toolbar(ui);
                    if self.split {
                        ui.selectable_value(&mut self.active, 0, "Left image");
                        ui.selectable_value(&mut self.active, 1, "Right image");
                        ui.checkbox(&mut self.linked, "Sync zoom and pan")
                            .on_hover_text("Align each image with sync off, then enable it. Relative zoom and screen movement are linked; Z, rotation and channels remain independent.");
                        ui.strong(if self.active == 0 { "Controls: LEFT image" } else { "Controls: RIGHT image" });
                    }

                });
            });
            egui::TopBottomPanel::top("slide-tabs").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(if self.active == 0 { "Left:" } else { "Right:" });
                    let mut selected = None;
                    let mut closed = None;
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (index, path) in self.tabs[self.active].iter().enumerate() {
                                let active = self.viewers[self.active]
                                    .slide
                                    .as_ref()
                                    .is_some_and(|s| &s.path == path);
                                ui.push_id(index, |ui| {
                                    if ui
                                        .selectable_label(
                                            active,
                                            path.file_name().unwrap_or_default().to_string_lossy(),
                                        )
                                        .on_hover_text(path.display().to_string())
                                        .clicked()
                                        && !active
                                    {
                                        selected = Some(path.clone());
                                    }
                                    if ui.small_button("×").on_hover_text("Close tab").clicked() {
                                        closed = Some(index);
                                    }
                                });
                            }
                        });
                    });
                    if let Some(index) = closed {
                        let path = self.tabs[self.active].remove(index);
                        if self.viewers[self.active]
                            .slide
                            .as_ref()
                            .is_some_and(|s| s.path == path)
                        {
                            selected = self.tabs[self.active].last().cloned();
                            if selected.is_none() {
                                self.viewers[self.active].close();
                            }
                        }
                    }
                    if let Some(path) = selected {
                        self.viewers[self.active].open(path);
                    }
                });
            });
            if let Some(path) = self.library.show(ctx) {
                self.viewers[self.active].open(path);
            }
        }
        if self.active != previous_active {
            self.viewers[previous_active].features.cancel_gesture();
        }
        let before = Position::of(&self.viewers[self.active]);
        let epoch = self.viewers[self.active].slide_epoch;
        let can_sync =
            self.split && self.linked && self.viewers.iter().all(|v| v.slide.is_some() && !v.fit);
        if !self.library.open {
            self.viewers[self.active].handle_shortcuts(ctx);
        }
        for viewer in &mut self.viewers {
            viewer.apply_channel_changes(ctx);
        }
        self.viewers[self.active].controls(ctx, &mut self.total_budget);
        self.budgets();

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(25, 29, 34)))
            .show(ctx, |ui| {
                if self.split {
                    ui.columns(2, |columns| {
                        // Render the driving pane first so the other pane follows
                        // in the same frame, regardless of which side is selected.
                        for index in [self.active, 1 - self.active] {
                            let pane = &mut columns[index];
                            self.pane_rects[index] = pane.max_rect();
                            pane.set_clip_rect(pane.max_rect());
                            pane.push_id(index, |ui| {
                                let side = if index == 0 { "Left" } else { "Right" };
                                let name = self.viewers[index]
                                    .slide
                                    .as_ref()
                                    .and_then(|s| s.path.file_name())
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "No image".into());
                                let color = if self.active == index {
                                    egui::Color32::LIGHT_BLUE
                                } else {
                                    egui::Color32::LIGHT_GRAY
                                };
                                ui.horizontal_wrapped(|ui| {
                                    let state = if self.active == index {
                                        " · ACTIVE"
                                    } else {
                                        ""
                                    };
                                    ui.colored_label(color, format!("{side}{state} · {name}"));
                                    if ui
                                        .button("Open here…")
                                        .on_hover_text("Open an image in this pane.")
                                        .clicked()
                                    {
                                        self.viewers[self.active].features.cancel_gesture();
                                        self.active = index;
                                        self.viewers[index].choose_slide();
                                    }
                                });
                                if let Some(slide) = &self.viewers[index].slide {
                                    ui.small(format!(
                                        "Zoom {:.1}% · Z {} / {}",
                                        self.viewers[index].zoom * 100.0,
                                        self.viewers[index].plane + 1,
                                        slide.planes,
                                    ));
                                }
                                self.viewers[index].canvas(ui);
                            });
                            pane.painter().rect_stroke(
                                self.pane_rects[index].shrink(0.5),
                                0.0,
                                egui::Stroke::new(
                                    if self.active == index { 2.0_f32 } else { 1.0 },
                                    if self.active == index {
                                        egui::Color32::LIGHT_BLUE
                                    } else {
                                        egui::Color32::from_gray(70)
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            if index == self.active
                                && can_sync
                                && epoch == self.viewers[index].slide_epoch
                            {
                                self.synchronize(before);
                            }
                        }
                    });
                } else {
                    self.pane_rects = [egui::Rect::NOTHING; 2];
                    self.viewers[0].canvas(ui);
                }
            });
        for viewer in &mut self.viewers {
            viewer.observe_navigation(ctx);
        }
        let preferences = self.viewers[self.active].preferences;
        if preferences != preferences_before {
            for viewer in &mut self.viewers {
                viewer.preferences = preferences;
            }
            self.preferences_save_at = Some(Instant::now() + std::time::Duration::from_secs(1));
        }
        if let Some(deadline) = self.preferences_save_at {
            if Instant::now() >= deadline {
                self.save_preferences();
            } else {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        self.viewers[self.active].slide_info(ctx);
        let viewer = &mut self.viewers[self.active];
        viewer
            .associated
            .show(ctx, viewer.slide.as_ref().map(|s| &s.path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_caches_share_the_budget_even_when_comparison_is_hidden() {
        let (left, _) = crate::tests::tile_viewer();
        let (right, _) = crate::tests::tile_viewer();
        let mut app = Comparison::new(left, right);
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "budget-test",
            egui::ColorImage::new([1, 1], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        for mib in [512, 256, 96] {
            app.total_budget = mib * 1048576;
            app.split = mib != 96;
            for viewer in &mut app.viewers {
                for x in 0..300 {
                    viewer.cache.push(CachedRegion {
                        texture: texture.clone(),
                        region: Region {
                            sampling: 1,
                            preview: false,
                            series: x as usize % 4,
                            interleaved: true,
                            plane: 0,
                            x: x * TILE_SIZE,
                            y: 0,
                            w: TILE_SIZE,
                            h: TILE_SIZE,
                            world: [0.0; 4],
                        },
                    });
                }
            }
            app.budgets();
            assert!(app.viewers.iter().map(Viewer::cache_bytes).sum::<usize>() <= app.total_budget);
            assert!(app
                .viewers
                .iter()
                .all(|v| v.cache_budget == app.total_budget / 2));
        }
    }

    #[test]
    fn right_pane_drives_left_without_changing_z_channels_or_rotation() {
        let (left, _) = crate::tests::tile_viewer();
        let (right, _) = crate::tests::tile_viewer();
        let mut app = Comparison::new(left, right);
        app.active = 1;
        app.viewers[0].plane = 0;
        app.viewers[0].features.rotation = 1;
        let channels = app.viewers[0].channels.settings.clone();
        let before = Position::of(&app.viewers[1]);
        app.viewers[1].center.x += 100.0;
        app.viewers[1].zoom *= 2.0;
        app.synchronize(before);
        assert_eq!(app.viewers[0].zoom, 2.0);
        assert_eq!(app.viewers[0].center, egui::vec2(10000.0, 7900.0));
        assert_eq!(app.viewers[0].plane, 0);
        assert_eq!(app.viewers[0].features.rotation, 1);
        assert_eq!(app.viewers[0].channels.settings, channels);
        assert!(app.viewers[0].dirty);
    }

    fn position(x: f32, y: f32, zoom: f32, rotation: u8) -> Position {
        Position {
            center: egui::vec2(x, y),
            zoom,
            rotation,
            mirrored: [false; 2],
        }
    }

    #[test]
    fn preserves_alignment_and_relative_zoom_between_different_images() {
        let a = position(100.0, 200.0, 1.0, 0);
        let b = position(900.0, 800.0, 0.5, 0);
        let next = linked_position(a, position(120.0, 190.0, 2.0, 0), b);
        assert_eq!(next.zoom, 1.0);
        assert_eq!(next.center, egui::vec2(940.0, 780.0));
        let reverse = linked_position(position(120.0, 190.0, 2.0, 0), a, next);
        assert_eq!(reverse.center, b.center);
        assert_eq!(reverse.zoom, b.zoom);
    }

    #[test]
    fn mirrored_panes_keep_the_same_screen_pan_without_changing_orientation() {
        for rotation in 0..4 {
            for source_flags in [[false, false], [true, false], [false, true], [true, true]] {
                for target_flags in [[false, false], [true, false], [false, true], [true, true]] {
                    let a = Position {
                        mirrored: source_flags,
                        ..position(10.0, 20.0, 1.0, rotation)
                    };
                    let after = Position {
                        center: a.center + egui::vec2(30.0, -20.0),
                        ..a
                    };
                    let b = Position {
                        mirrored: target_flags,
                        ..position(900.0, 800.0, 0.5, (rotation + 1) % 4)
                    };
                    let next = linked_position(a, after, b);
                    assert_eq!(
                        orientation::forward(
                            (after.center - a.center) * after.zoom,
                            a.rotation,
                            a.mirrored
                        ),
                        orientation::forward(
                            (next.center - b.center) * next.zoom,
                            b.rotation,
                            b.mirrored
                        ),
                    );
                    assert_eq!(next.mirrored, b.mirrored);
                }
            }
        }
    }

    #[test]
    fn independent_rotations_produce_the_same_screen_displacement() {
        for source_rotation in 0..4 {
            for target_rotation in 0..4 {
                let a = position(100.0, 200.0, 2.0, source_rotation);
                let next_a = position(120.0, 190.0, 2.0, source_rotation);
                let b = position(900.0, 800.0, 0.5, target_rotation);
                let next_b = linked_position(a, next_a, b);
                let source_delta =
                    features::rotate((next_a.center - a.center) * a.zoom, source_rotation);
                let target_delta =
                    features::rotate((next_b.center - b.center) * b.zoom, target_rotation);
                assert_eq!(source_delta, target_delta);
                assert_eq!(next_b.rotation, b.rotation);
            }
        }
    }
}
