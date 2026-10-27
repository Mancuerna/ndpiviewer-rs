//! Compact viewer controls and an overlay inspector that never resizes the image.
use super::*;

#[derive(Default)]
pub(super) struct Chrome {
    pub(super) settings_open: bool,
    pub(crate) clean_view: bool,
    binding_error: Option<String>,
    settings_section: usize,
    settings_search: String,
    sidebar_open: bool,
    last_hover: f64,
}

impl Viewer {
    pub(crate) fn sidebar_shown(&self) -> bool {
        self.preferences
            .sidebar_override
            .unwrap_or(!self.preferences.auto_hide_sidebar || self.chrome.sidebar_open)
    }

    pub(crate) fn toggle_sidebar(&mut self) {
        self.preferences.sidebar_override = Some(!self.sidebar_shown());
    }

    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let tools = ui
            .button(format!(
                "Tools [{}]",
                self.preferences
                    .bindings
                    .get(keybindings::Action::Sidebar)
                    .label()
            ))
            .on_hover_text("Show annotation, channel, tracking and export controls");
        if tools.clicked() {
            self.toggle_sidebar();
        }
        if tools.hovered() {
            self.chrome.last_hover = ui.input(|i| i.time);
        }
        ui.menu_button("Open", |ui| {
            if ui.button("Open slide…").clicked() {
                self.choose_slide();
                ui.close_menu();
            }
            if ui.button("Open Z stack…").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.open(path);
                }
                ui.close_menu();
            }
        });
        if ui
            .add_enabled(
                self.slide.is_some(),
                egui::Button::new(format!(
                    "Fit [{}]",
                    self.preferences
                        .bindings
                        .get(keybindings::Action::Fit)
                        .label()
                )),
            )
            .clicked()
        {
            self.fit = true;
        }
        self.navigation_buttons(ui);
        self.magnification_menu(ui);
        ui.menu_button("Image", |ui| {
            self.orientation_menu(ui);
            if ui
                .add_enabled(
                    self.slide.is_some(),
                    egui::Button::new("Glass slide / label"),
                )
                .clicked()
            {
                self.associated.open = true;
                ui.close_menu();
            }
            if ui
                .add_enabled(self.slide.is_some(), egui::Button::new("Slide info"))
                .clicked()
            {
                self.features.info_open = true;
                ui.close_menu();
            }
        });
        if ui
            .button(format!(
                "Hide tools [{}]",
                self.preferences
                    .bindings
                    .get(keybindings::Action::CleanView)
                    .label()
            ))
            .clicked()
        {
            self.chrome.clean_view = true;
            self.features.info_open = false;
            self.associated.open = false;
            self.features.cancel_gesture();
            ui.ctx().request_repaint();
        }
        if ui.button("Settings…").clicked() {
            self.chrome.settings_open = true;
        }
    }

    pub(super) fn controls(&mut self, ctx: &egui::Context, total_budget: &mut usize) {
        if self.chrome.clean_view {
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("inspector-overlay-rect"), egui::Rect::NOTHING)
            });
            let reveal = ctx.input(|i| {
                i.pointer
                    .hover_pos()
                    .is_some_and(|p| p.x < 200.0 && p.y < 60.0)
            });
            if reveal {
                egui::Area::new(egui::Id::new("exit-clean-view"))
                    .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
                    .show(ctx, |ui| {
                        if ui
                            .small_button(format!(
                                "Show tools [{}]",
                                self.preferences
                                    .bindings
                                    .get(keybindings::Action::CleanView)
                                    .label()
                            ))
                            .on_hover_text("Escape also restores the interface")
                            .clicked()
                        {
                            self.chrome.clean_view = false;
                        }
                    });
            }
            return;
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.busy { ui.spinner(); }
                if self.pending_open.is_some() || (self.busy && self.slide.is_none()) { ui.label("Opening slide…"); }
                else if self.slide.is_some() {
                    let wanted=self.visible_tiles(self.viewport_size);
                    let loaded=wanted.iter().filter(|r|self.cached(r)).count();
                    if self.shown_plane!=Some(self.plane) { ui.label("Loading overview…"); }
                    else if loaded<wanted.len() { ui.label(format!("Refining view: {loaded}/{} tiles",wanted.len())); }
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
                    ui.label("Scroll: zoom · Drag: pan · Escape: Pan / select");
                    ui.label("Page Up / Down or Ctrl+wheel: focus Z");
                    ui.label("Ctrl+Z / Ctrl+Y: undo / redo annotations");
                    ui.label("F11: full screen · Hold Space: temporary pan");
                    ui.label("Right-drag: zoom to region");
                    for action in keybindings::ACTIONS { ui.label(format!("{}: {}", self.preferences.bindings.get(action).label(), self.preferences.action_label(action))); }
                });
            });
        });

        if self.capture_pending() {
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("inspector-overlay-rect"), egui::Rect::NOTHING)
            });
            return;
        }
        let mut open = self.chrome.settings_open;
        egui::Window::new("Settings")
            .vscroll(true)
            .max_height((ctx.screen_rect().height() - 120.0).max(160.0))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(410.0)
            .show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.chrome.settings_search).hint_text("Find settings…"));
                ui.horizontal_wrapped(|ui| { for (index,label) in ["Navigation","Appearance","Annotations","Shortcuts","Library","Performance"].into_iter().enumerate() { ui.selectable_value(&mut self.chrome.settings_section,index,label); } });
                ui.separator();
                let query=self.chrome.settings_search.trim().to_lowercase();
                if (query.is_empty() && self.chrome.settings_section == 0) || (!query.is_empty() && "navigation wheel magnification keys overlap arrows movement zoom focus".contains(&query)) {
                ui.heading("Navigation");
                ui.collapsing("Magnification keys", |ui| {
                    for index in 0..5 {
                        ui.horizontal(|ui| {
                            ui.label(self.preferences.bindings.get(keybindings::MAG_ACTIONS[index]).label());
                            egui::ComboBox::from_id_salt(("mag-slot",index)).selected_text(format!("{}×",self.preferences.magnifications[index])).show_ui(ui, |ui| {
                                for value in magnification::PRESETS { ui.selectable_value(&mut self.preferences.magnifications[index],value,format!("{value}×")); }
                            });
                        });
                    }
                });
                ui.add(egui::Slider::new(&mut self.preferences.field_overlap, 0.0..=50.0).suffix("%").text("Field overlap"));
                ui.small("N / P: next / previous field. T: hold to hide marks. L: hold to magnify. ?: shortcut sheet.");
                ui.checkbox(&mut self.preferences.wheel_magnification_steps, "Wheel: microscope magnification steps");
                ui.small("Requires scan objective metadata. Select at least two wheel magnifications:");
                ui.horizontal_wrapped(|ui| {
                    for (index, value) in magnification::PRESETS.iter().enumerate() {
                        let mut enabled = self.preferences.wheel_presets[index];
                        let can_change = !enabled || self.preferences.wheel_presets.iter().filter(|v| **v).count() > 2;
                        if ui.add_enabled(can_change, egui::Checkbox::new(&mut enabled, format!("{value}×"))).on_hover_text("Keep at least two magnifications selected").changed() {
                            self.preferences.wheel_presets[index] = enabled;
                        }
                    }
                });
                ui.checkbox(&mut self.preferences.invert_zoom_wheel, "Invert zoom wheel direction");
                ui.small("Applies to continuous and stepped zoom. Ctrl+wheel still changes focus Z.");
                ui.separator();
                ui.label("Arrow-key movement (% of the visible field)");
                ui.add(
                    egui::Slider::new(&mut self.preferences.arrow_percent, 1.0..=100.0).suffix("%"),
                );
                ui.small("Horizontal arrows use the field width; vertical arrows use its height.");
                ui.separator();

                }
                if (query.is_empty() && self.chrome.settings_section == 1) || (!query.is_empty() && "appearance interface scale sidebar minimap distraction tools".contains(&query)) {
                ui.heading("Interface");
                if ui.checkbox(&mut self.preferences.auto_hide_sidebar, "Auto-hide left sidebar").changed() {
                    self.preferences.sidebar_override = None;
                }
                if ui.button("Resume automatic sidebar visibility").clicked() {
                    self.preferences.sidebar_override = None;
                }
                ui.small("Automatic mode reveals the sidebar at the left edge. Shortcuts can be customized below.");
                ui.checkbox(&mut self.preferences.show_minimap, format!("Show minimap [{}]", self.preferences.bindings.get(keybindings::Action::Minimap).label()));
                ui.label("Interface scale");
                let mut percent = (self.preferences.ui_scale * 100.0).round();
                if ui.add(egui::Slider::new(&mut percent, 75.0..=200.0).step_by(5.0).suffix("%")).changed() {
                    self.preferences.ui_scale = percent / 100.0;
                }
                if ui.small_button("Reset scale to 100%").clicked() { self.preferences.ui_scale = 1.0; }
                ui.separator();
                ui.collapsing("Distraction-free view", |ui| {
                    ui.checkbox(&mut self.preferences.clean_show_minimap, "Keep minimap visible");
                    ui.checkbox(&mut self.preferences.clean_show_scale, "Keep scale bar visible");
                    ui.label(format!("Toggle with {}. Escape restores the interface.", self.preferences.bindings.get(keybindings::Action::CleanView).label()));
                });

                }
                if (query.is_empty() && self.chrome.settings_section == 2) || (!query.is_empty() && "annotations color width text size".contains(&query)) {
                ui.collapsing("Annotation defaults", |ui| {
                    ui.horizontal(|ui| { ui.label("New annotation color"); ui.color_edit_button_srgb(&mut self.preferences.annotation_color); });
                    ui.add(egui::Slider::new(&mut self.preferences.annotation_width, 1.0..=6.0).text("Line width"));
                    ui.add(egui::Slider::new(&mut self.preferences.annotation_text_size, 12.0..=32.0).text("Text size"));
                    ui.small("Line and text sizes apply to all displayed annotations and exports. Existing annotation colors are preserved.");
                    if ui.button("Restore annotation defaults").clicked() {
                        let defaults = settings::Preferences::default();
                        self.preferences.annotation_color = defaults.annotation_color;
                        self.preferences.annotation_width = defaults.annotation_width;
                        self.preferences.annotation_text_size = defaults.annotation_text_size;
                    }
                });

                }
                if (query.is_empty() && self.chrome.settings_section == 3) || (!query.is_empty() && "shortcuts keyboard keys bindings".contains(&query)) {
                ui.collapsing("Keyboard shortcuts", |ui| {
                    ui.small("Editing shortcuts, arrows, Page Up/Down, Space, Enter, Escape and F11 remain reserved.");
                    if ui.button("Restore default shortcuts").clicked() { self.preferences.bindings = Default::default(); self.chrome.binding_error = None; }
                    if let Some(error) = &self.chrome.binding_error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
                    for action in keybindings::ACTIONS {
                        ui.push_id(action as usize, |ui| {
                            let old = self.preferences.bindings.get(action);
                            let mut binding = old;
                            ui.horizontal_wrapped(|ui| {
                                ui.label(self.preferences.action_label(action));
                                egui::ComboBox::from_id_salt("key").selected_text(binding.key.name()).show_ui(ui, |ui| {
                                    for key in egui::Key::ALL { ui.selectable_value(&mut binding.key, *key, key.name()); }
                                });
                                ui.checkbox(&mut binding.ctrl, "Ctrl"); ui.checkbox(&mut binding.alt, "Alt"); ui.checkbox(&mut binding.shift, "Shift");
                            });
                            if binding != old {
                                let mut proposed = self.preferences.bindings;
                                proposed.0[action as usize] = binding;
                                self.chrome.binding_error = proposed.error();
                                if self.chrome.binding_error.is_none() { self.preferences.bindings = proposed; }
                            }
                        });
                    }
                });

                }
                if (query.is_empty() && self.chrome.settings_section == 4) || (!query.is_empty() && "library labels thumbnails".contains(&query)) {
                ui.heading("Library");
                ui.checkbox(&mut self.preferences.label_thumbnails, "Use glass-slide / label thumbnails");
                ui.small("Uses the captured label or macro image. Falls back to the slide preview when unavailable.");
                ui.separator();

                }
                if (query.is_empty() && self.chrome.settings_section == 5) || (!query.is_empty() && "performance cache memory prefetch diagnostic".contains(&query)) {
                ui.heading("Performance");
                ui.checkbox(
                    &mut self.preferences.prefetch_neighbors,
                    "Prefetch adjacent image tiles",
                );
                ui.small(
                    "Loads nearby detail when the visible field is ready, within the cache limit.",
                );
                ui.checkbox(&mut self.preferences.prefetch_z, "Prefetch neighboring Z detail");
                self.diagnostic_export_button(ui);
                let previous = *total_budget;
                egui::ComboBox::from_id_salt("cache-budget")
                    .selected_text(format!("Cache: {} MiB", *total_budget / 1048576))
                    .show_ui(ui, |ui| {
                        for mib in [96, 256, 512] {
                            ui.selectable_value(total_budget, mib * 1048576, format!("{mib} MiB"));
                        }
                    });
                if previous != *total_budget && self.benchmark.is_none() {
                    if let Some(path) = settings::path() {
                        if let Err(error) = settings::save(&path, *total_budget) {
                            diagnostics::record(diagnostics::Event::SettingsSaveFailed);
                            self.status = format!("Could not save cache setting: {error}");
                        }
                    }
                }

                }
            });
        self.chrome.settings_open = open;
        if open {
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("inspector-overlay-rect"), egui::Rect::NOTHING)
            });
        } else {
            self.sidebar(ctx);
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let available = ctx.available_rect();
        let width = 350.0_f32.min(available.width());
        let panel = egui::Rect::from_min_size(available.min, egui::vec2(width, available.height()));
        let edge = egui::Rect::from_min_size(available.min, egui::vec2(22.0, available.height()));
        let (pointer, time, down) =
            ctx.input(|i| (i.pointer.hover_pos(), i.time, i.pointer.any_down()));
        let hover_edge = pointer.is_some_and(|p| edge.contains(p));
        if hover_edge && !down {
            self.chrome.sidebar_open = true;
        }
        if pointer.is_some_and(|p| panel.contains(p)) && self.chrome.sidebar_open {
            self.chrome.last_hover = time;
        }
        let interacting = down || ctx.wants_keyboard_input() || ctx.memory(|m| m.any_popup_open());
        if self.preferences.auto_hide_sidebar
            && self.chrome.sidebar_open
            && !interacting
            && !hover_edge
        {
            if time - self.chrome.last_hover > 0.65 {
                self.chrome.sidebar_open = false;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        let shown = self.sidebar_shown();
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("inspector-overlay-rect"),
                if shown { panel } else { edge },
            )
        });
        egui::Area::new(egui::Id::new("inspector-overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(available.min)
            .movable(false)
            .show(ctx, |ui| {
                ui.set_min_size(if shown { panel.size() } else { edge.size() });
                ui.set_max_size(if shown { panel.size() } else { edge.size() });
                egui::Frame::side_top_panel(ui.style())
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_gray(85)))
                    .show(ui, |ui| {
                        if shown {
                            ui.set_width((width - 16.0).max(1.0));
                            ui.set_height((available.height() - 16.0).max(1.0));
                            egui::ScrollArea::vertical()
                                .id_salt("whole-inspector")
                                .show(ui, |ui| {
                                    self.inspector_panel(ui);
                                });
                        } else {
                            ui.set_min_size(egui::vec2(4.0, (available.height() - 16.0).max(1.0)));
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_preserves_viewport() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let ctx = egui::Context::default();
        let mut budget = DEFAULT_CACHE_BUDGET;
        let mut sizes = Vec::new();
        for shown in [false, true, false] {
            viewer.preferences.auto_hide_sidebar = !shown;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                viewer.controls(ctx, &mut budget);
                egui::CentralPanel::default().show(ctx, |ui| {
                    sizes.push(ui.available_size());
                });
            });
        }
        assert_eq!(sizes[0], sizes[1]);
        assert_eq!(sizes[1], sizes[2]);
    }
}
