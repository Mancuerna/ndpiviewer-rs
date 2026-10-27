use super::*;
pub(crate) const PRESETS: [f64; 10] = [1.25, 2.0, 2.5, 4.0, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0];

fn pixel_zoom(source: f64, requested: f64) -> Option<f32> {
    let zoom = requested / source;
    (source.is_finite() && source > 0.0 && requested.is_finite() && (0.0001..=8.0).contains(&zoom))
        .then_some(zoom as f32)
}

/// At most one focus step per input frame, never repeated by scroll smoothing.
pub(super) fn focus_scroll(events: &[egui::Event]) -> i32 {
    let delta: f32 = events
        .iter()
        .filter_map(|event| {
            if let egui::Event::MouseWheel {
                delta, modifiers, ..
            } = event
            {
                (modifiers.ctrl && !modifiers.alt && delta.y.is_finite()).then_some(delta.y)
            } else {
                None
            }
        })
        .sum();
    if delta > 0.0 {
        -1
    } else if delta < 0.0 {
        1
    } else {
        0
    }
}

/// Use raw events for discrete steps: smoothed deltas continue for several frames.
fn zoom_wheel_step(events: &[egui::Event]) -> i32 {
    let delta: f32 = events
        .iter()
        .filter_map(|event| match event {
            egui::Event::MouseWheel {
                delta, modifiers, ..
            } if modifiers.is_none() && delta.y.is_finite() => Some(delta.y),
            _ => None,
        })
        .sum();
    if delta > 0.0 {
        1
    } else if delta < 0.0 {
        -1
    } else {
        0
    }
}

fn next_magnification_zoom(
    source: f64,
    zoom: f32,
    direction: i32,
    enabled: &[bool; 10],
) -> Option<f32> {
    if !zoom.is_finite() || zoom <= 0.0 || direction == 0 {
        return None;
    }
    let candidates: Vec<_> = PRESETS
        .into_iter()
        .zip(enabled)
        .filter_map(|(value, &enabled)| enabled.then_some(value))
        .filter_map(|value| pixel_zoom(source, value))
        .collect();
    // Tolerate f32 round-off at an exact preset, but never skip intermediate zooms.
    if direction > 0 {
        candidates
            .into_iter()
            .find(|&next| next > zoom * (1.0 + 1e-5))
    } else {
        candidates
            .into_iter()
            .rev()
            .find(|&next| next < zoom * (1.0 - 1e-5))
    }
}

impl Viewer {
    pub(super) fn wheel_zoom(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        if self.chrome.settings_open
            || ctx.wants_keyboard_input()
            || ctx.memory(|m| m.any_popup_open())
        {
            return;
        }
        let direction = if self.preferences.invert_zoom_wheel {
            -1
        } else {
            1
        };
        let next = if self.preferences.wheel_magnification_steps {
            let step = ctx.input(|i| {
                if !i.modifiers.is_none() || focus_scroll(&i.events) != 0 {
                    0
                } else {
                    zoom_wheel_step(&i.events)
                }
            }) * direction;
            if step == 0 {
                return;
            }
            let source = self
                .slide
                .as_ref()
                .and_then(|s| s.ndpa.as_ref())
                .map(|g| g.lens);
            let Some(source) = source.filter(|s| s.is_finite() && *s > 0.0) else {
                self.status = "Stepped zoom unavailable: missing scan objective metadata. Choose continuous zoom in Settings.".into();
                return;
            };
            next_magnification_zoom(source, self.zoom, step, &self.preferences.wheel_presets)
        } else {
            let delta = ctx.input(|i| {
                if !i.modifiers.is_none() || focus_scroll(&i.events) != 0 {
                    0.0
                } else {
                    i.smooth_scroll_delta.y
                }
            }) * direction as f32;
            if !delta.is_finite() || delta == 0.0 {
                return;
            }
            Some((self.zoom * (delta * 0.003).exp()).clamp(0.0001, 8.0))
        };
        if let Some(next) = next.filter(|z| *z != self.zoom) {
            let pointer = ctx
                .input(|i| i.pointer.hover_pos())
                .unwrap_or(rect.center());
            let offset = self.inverse_transform(pointer - rect.center());
            let anchor = self.center + offset / self.zoom;
            self.features.cancel_gesture();
            self.zoom = next;
            self.center = anchor - offset / next;
            self.fit = false;
            self.changed();
        }
    }

    pub(super) fn set_magnification(&mut self, requested: f64) {
        let zoom = self
            .slide
            .as_ref()
            .and_then(|s| s.ndpa.as_ref())
            .and_then(|g| pixel_zoom(g.lens, requested));
        if let Some(zoom) = zoom {
            self.features.cancel_gesture();
            self.zoom = zoom;
            self.fit = false;
            self.changed();
        } else {
            self.status =
                "Magnification unavailable: missing or unsupported scan objective metadata.".into();
        }
    }

    pub(super) fn magnification_menu(&mut self, ui: &mut egui::Ui) {
        let source = self
            .slide
            .as_ref()
            .and_then(|slide| slide.ndpa.as_ref())
            .map(|g| g.lens);
        let label = source
            .filter(|lens| lens.is_finite() && *lens > 0.0)
            .map(|lens| format!("{:.2}×", lens * f64::from(self.zoom)))
            .unwrap_or_else(|| "Zoom".into());
        ui.add_enabled_ui(source.is_some(), |ui| {
            ui.menu_button(label, |ui| {
                let source = source.unwrap_or(0.0);
                ui.small(format!("Source scan: {source}×"));
                ui.small("Scan-relative magnification; physical scale is shown by the µm bar.");
                for requested in PRESETS {
                    if let Some(zoom) = pixel_zoom(source, requested) {
                        if ui
                            .selectable_label(
                                (self.zoom - zoom).abs() < 0.0001,
                                self.preferences
                                    .magnifications
                                    .iter()
                                    .position(|v| *v == requested)
                                    .map_or_else(
                                        || format!("{requested}×"),
                                        |index| {
                                            format!(
                                                "{requested}×  [{}]",
                                                self.preferences
                                                    .bindings
                                                    .get(crate::keybindings::MAG_ACTIONS[index])
                                                    .label()
                                            )
                                        },
                                    ),
                            )
                            .clicked()
                        {
                            self.features.cancel_gesture();
                            self.zoom = zoom;
                            self.fit = false;
                            self.changed();
                            ui.close_menu();
                        }
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wheel(y: f32, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, y),
            modifiers,
        }
    }

    #[test]
    fn custom_wheel_presets_skip_disabled_magnifications() {
        let enabled = [
            true, false, true, false, true, false, false, false, true, false,
        ];
        assert_eq!(
            next_magnification_zoom(40.0, 0.04, 1, &enabled),
            Some(0.0625)
        );
        assert_eq!(
            next_magnification_zoom(40.0, 0.0625, 1, &enabled),
            Some(0.125)
        );
        assert_eq!(
            next_magnification_zoom(40.0, 0.125, -1, &enabled),
            Some(0.0625)
        );
        assert_eq!(next_magnification_zoom(40.0, 0.125, 1, &enabled), Some(2.0));
        assert_eq!(next_magnification_zoom(40.0, 0.03125, -1, &enabled), None);
    }

    #[test]
    fn presets_snap_in_both_directions_and_stop_at_the_limits() {
        for (current, direction, expected) in [
            (0.01, 1, Some(0.05)),
            (0.05, 1, Some(0.1)),
            (0.125, 1, Some(0.25)),
            (0.125, -1, Some(0.1)),
            (0.25, -1, Some(0.1)),
            (1.0, 1, None),
            (0.05, -1, None),
            (2.0, -1, Some(1.0)),
        ] {
            assert_eq!(
                next_magnification_zoom(
                    40.0,
                    current,
                    direction,
                    &settings::Preferences::default().wheel_presets
                ),
                expected
            );
        }
        assert_eq!(
            next_magnification_zoom(
                20.0,
                1.0,
                1,
                &settings::Preferences::default().wheel_presets
            ),
            Some(2.0)
        );
        assert_eq!(
            next_magnification_zoom(
                f64::NAN,
                1.0,
                1,
                &settings::Preferences::default().wheel_presets
            ),
            None
        );
        assert_eq!(zoom_wheel_step(&[wheel(3.0, egui::Modifiers::CTRL)]), 0);
        assert_eq!(zoom_wheel_step(&[wheel(3.0, egui::Modifiers::SHIFT)]), 0);
        assert_eq!(
            zoom_wheel_step(&[wheel(f32::NAN, egui::Modifiers::NONE)]),
            0
        );
        assert_eq!(zoom_wheel_step(&[]), 0);
    }

    #[test]
    fn wheel_modes_preserve_pointer_anchor_and_do_not_repeat_smoothed_steps() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        viewer.slide.as_mut().unwrap().ndpa = Some(crate::ndpa::Geometry {
            lens: 40.0,
            offset: [0.0; 2],
            z: vec![0.0],
        });
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        let pointer = egui::pos2(650.0, 450.0);
        for rotation in 0..4 {
            for inverted in [false, true] {
                viewer.features.rotation = rotation;
                viewer.preferences.wheel_magnification_steps = true;
                viewer.preferences.invert_zoom_wheel = inverted;
                viewer.zoom = 0.25;
                let anchor =
                    viewer.center + viewer.inverse_transform(pointer - rect.center()) / viewer.zoom;
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![
                            egui::Event::PointerMoved(pointer),
                            wheel(3.0, egui::Modifiers::NONE),
                        ],
                        ..Default::default()
                    },
                    |ctx| viewer.wheel_zoom(ctx, rect),
                );
                assert_eq!(viewer.zoom, if inverted { 0.1 } else { 0.5 });
                let after =
                    viewer.center + viewer.inverse_transform(pointer - rect.center()) / viewer.zoom;
                assert!((after - anchor).length() < 0.01);
                let zoom = viewer.zoom;
                let _ = ctx.run(Default::default(), |ctx| {
                    ctx.input_mut(|i| i.smooth_scroll_delta.y = 20.0);
                    viewer.wheel_zoom(ctx, rect);
                });
                assert_eq!(viewer.zoom, zoom, "No extra preset from smoothing");
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![wheel(3.0, egui::Modifiers::CTRL)],
                        ..Default::default()
                    },
                    |ctx| viewer.wheel_zoom(ctx, rect),
                );
                assert_eq!(viewer.zoom, zoom, "Ctrl+wheel belongs to focus Z");
                viewer.preferences.wheel_magnification_steps = false;
                viewer.zoom = 0.25;
                let _ = ctx.run(Default::default(), |ctx| {
                    ctx.input_mut(|i| i.smooth_scroll_delta.y = 20.0);
                    viewer.wheel_zoom(ctx, rect);
                });
                assert_eq!(viewer.zoom > 0.25, !inverted);
            }
        }
        viewer.slide.as_mut().unwrap().ndpa = None;
        viewer.preferences.wheel_magnification_steps = true;
        let zoom = viewer.zoom;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![wheel(3.0, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| viewer.wheel_zoom(ctx, rect),
        );
        assert_eq!(viewer.zoom, zoom);
        assert!(viewer.status.contains("missing scan objective"));
    }

    #[test]
    fn scan_magnification_controls_pixel_zoom_without_inventing_calibration() {
        assert_eq!(pixel_zoom(40.0, 10.0), Some(0.25));
        assert_eq!(pixel_zoom(20.0, 40.0), Some(2.0));
        assert_eq!(pixel_zoom(20.0, 160.0), Some(8.0));
        for (source, requested) in [
            (0.0, 20.0),
            (f64::NAN, 20.0),
            (20.0, f64::INFINITY),
            (20.0, 200.0),
        ] {
            assert!(pixel_zoom(source, requested).is_none());
        }
    }
    #[test]
    fn focus_wheel_requires_control_and_ignores_horizontal_motion() {
        let wheel = |ctrl, y| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, y),
            modifiers: egui::Modifiers {
                ctrl,
                ..Default::default()
            },
        };
        assert_eq!(focus_scroll(&[wheel(false, 120.0)]), 0);
        assert_eq!(focus_scroll(&[wheel(true, 120.0)]), -1);
        assert_eq!(focus_scroll(&[wheel(true, -120.0), wheel(true, -80.0)]), 1);
        assert_eq!(focus_scroll(&[wheel(true, 0.0)]), 0);
        assert_eq!(focus_scroll(&[wheel(true, f32::NAN)]), 0);
    }
}
