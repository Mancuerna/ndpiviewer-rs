pub(super) use crate::display::{defaults, render, rgb_channels, Channel, Setting};
use crate::{channel_settings, same_region, CachedRegion, Viewer};
use eframe::egui;
use std::time::Instant;

#[derive(Default)]
pub(super) struct State {
    pub persistence: channel_settings::Persistence,
    pub settings: Vec<Setting>,
    pub active: Vec<Setting>,
    pub dirty_at: Option<Instant>,
    pub fallback: Option<CachedRegion>,
    pub auto_pending: Option<usize>,
    pub auto_running: Option<(u64, usize, u32, Setting)>,
    pub auto_undo: Vec<Option<[f32; 3]>>,
    pub auto_message: Option<String>,
}

impl Viewer {
    pub(super) fn channel_panel(&mut self, ui: &mut egui::Ui) {
        let Some(slide) = self.slide.as_ref() else {
            return;
        };
        ui.separator();
        self.channels
            .auto_undo
            .resize(self.channels.settings.len(), None);
        let auto_busy =
            self.channels.auto_pending.is_some() || self.channels.auto_running.is_some();
        egui::CollapsingHeader::new("Channel display").default_open(true).show(ui, |ui| {
            if slide.rgb_components {
                ui.small("RGB components, not independent fluorescence markers.");
            } else {
                ui.small("Additive channel display. Colors are display pseudocolors.");
            }
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                if ui.button("Show all").clicked() {
                    changed |= crate::display::set_visibility(&mut self.channels.settings, None, true);
                }
                if ui.button("Hide all").clicked() {
                    changed |= crate::display::set_visibility(&mut self.channels.settings, None, false);
                }
            });
            let mut solo = None;
            for (index, (channel, setting)) in slide
                .channels
                .iter()
                .zip(self.channels.settings.iter_mut())
                .enumerate()
            {
                ui.push_id(index, |ui| {
                    ui.spacing_mut().slider_width =
                        (ui.available_width() - 185.0).clamp(50.0, 150.0);
                    ui.horizontal_wrapped(|ui| {
                        changed |= ui.checkbox(&mut setting.enabled, &channel.name).changed();
                        if ui.small_button("Solo").on_hover_text("Show only this channel; preserve all display adjustments.").clicked() {
                            solo = Some(index);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        changed |= ui.color_edit_button_srgb(&mut setting.color).on_hover_text("Channel color").changed();
                        if ui.small_button("Reset").on_hover_text("Reset only this channel's color, visibility and intensity adjustments.").clicked() {
                            *setting = defaults(std::slice::from_ref(channel)).remove(0);
                            changed = true;
                        }
                    if !slide.rgb_components {
                            if ui.add_enabled(!auto_busy, egui::Button::new("Auto")).on_hover_text("Estimate contrast from native samples across the current Z plane (1st–99.5th percentiles). Resets gamma to 1; preserves color and visibility.").clicked() {
                                self.channels.auto_pending = Some(index);
                                self.channels.auto_message = Some("Analyzing channel samples…".into());
                            }
                            if ui.add_enabled(!auto_busy && self.channels.auto_undo[index].is_some(), egui::Button::new("Undo Auto")).on_hover_text("Restore brightness, contrast and gamma from before the last Auto adjustment.").clicked() {
                                if let Some(values) = self.channels.auto_undo[index].take() {
                                    crate::auto_contrast::apply(setting, values);
                                    changed = true;
                                    self.channels.auto_message = Some("Previous intensity settings restored.".into());
                                }
                            }
                    }
                    });
                    changed |= ui
                        .add(egui::Slider::new(&mut setting.brightness, -1.0..=1.0).custom_formatter(|value, _| format!("{value:.2}")).text("Brightness"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut setting.contrast, 0.1..=32.0).logarithmic(true).custom_formatter(|value, _| format!("{value:.2}")).text("Contrast"))
                        .changed();
                    changed |= ui.add(egui::Slider::new(&mut setting.gamma, 0.1..=5.0).logarithmic(true).custom_formatter(|value, _| format!("{value:.2}")).text("Gamma")).changed();
                    ui.separator();
                });
            }
            if let Some(index) = solo {
                changed |= crate::display::set_visibility(&mut self.channels.settings, Some(index), true);
            }
            if ui.button("Reset channels").clicked() {
                self.channels.settings = defaults(&slide.channels);
                changed = true;
            }
            ui.small("Contrast multiplies intensity; brightness adds an offset. Gamma > 1 brightens midtones; 1 is neutral. Display only.");
            if changed {
                self.channels.persistence.mark_changed();
                self.channels.dirty_at = Some(Instant::now());
            }
            ui.small("Channel settings are saved automatically for this image.");
        });
        if let Some(message) = &self.channels.persistence.message {
            ui.colored_label(egui::Color32::LIGHT_RED, message);
        }
        if let Some(message) = &self.channels.auto_message {
            ui.label(message);
        }
    }

    pub(super) fn apply_channel_changes(&mut self, ctx: &egui::Context) {
        if let Some(at) = self.channels.dirty_at {
            if at.elapsed().as_millis() < 150 {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
                return;
            }
            self.channels.dirty_at = None;
            self.save_channel_settings();
            self.channels.active = self.channels.settings.clone();
            if let Some(overview) = self.shown_plane.and_then(|z| self.overview(z)) {
                if let Some(old) = self
                    .cache
                    .iter()
                    .find(|e| same_region(&e.region, &overview))
                {
                    self.channels.fallback = Some(CachedRegion {
                        region: old.region.clone(),
                        texture: old.texture.clone(),
                    });
                }
            }
            self.changed();
            self.slide_epoch = self.revision; // Reject reads rendered with older settings.
            self.cache.clear();
            self.failed.clear();
            self.shown_plane = None;
        }
    }

    pub(super) fn save_channel_settings(&mut self) {
        if let Some(slide) = &self.slide {
            self.channels
                .persistence
                .flush(slide, &self.channels.settings);
        }
    }

    pub(crate) fn auto_contrast_request(&mut self, index: usize) -> Option<crate::Request> {
        let slide = self.slide.as_ref()?;
        let setting = self.channels.settings.get(index)?.clone();
        let level = slide.levels.last()?.clone();
        self.channels.auto_running = Some((self.revision, index, self.plane, setting));
        Some(crate::Request::Auto(
            self.revision,
            index,
            self.plane,
            level,
        ))
    }

    pub(crate) fn finish_auto_contrast(&mut self, id: u64, result: Result<[f32; 3], String>) {
        let Some((request_id, index, plane, before)) = self.channels.auto_running.take() else {
            return;
        };
        if request_id != id
            || id < self.slide_epoch
            || plane != self.plane
            || self.channels.settings.get(index) != Some(&before)
        {
            self.channels.auto_message = Some(
                "Auto discarded because the image, plane or settings changed. Try again.".into(),
            );
            return;
        }
        match result {
            Ok(values) => {
                self.channels
                    .auto_undo
                    .resize(self.channels.settings.len(), None);
                self.channels.auto_undo[index] = Some(crate::auto_contrast::intensities(&before));
                crate::auto_contrast::apply(&mut self.channels.settings[index], values);
                self.channels.persistence.mark_changed();
                self.channels.dirty_at = Some(Instant::now());
                self.channels.auto_message = Some("Auto applied from sampled intensities on this Z plane. Undo Auto restores the previous adjustment.".into());
            }
            Err(error) => self.channels.auto_message = Some(error),
        }
    }
}
