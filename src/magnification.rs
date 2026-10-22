use super::*;

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

impl Viewer {
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
            .map(|lens| format!("Magnification: {:.2}×", lens * f64::from(self.zoom)))
            .unwrap_or_else(|| "Magnification unavailable".into());
        ui.add_enabled_ui(source.is_some(), |ui| {
            ui.menu_button(label, |ui| {
                let source = source.unwrap_or(0.0);
                ui.small(format!("Source scan: {source}×"));
                ui.small("Scan-relative magnification; physical scale is shown by the µm bar.");
                for requested in [1.25, 2.0, 2.5, 4.0, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0] {
                    if let Some(zoom) = pixel_zoom(source, requested) {
                        if ui
                            .selectable_label(
                                (self.zoom - zoom).abs() < 0.0001,
                                [2.0, 4.0, 10.0, 20.0, 40.0]
                                    .iter()
                                    .position(|v| *v == requested)
                                    .map_or_else(
                                        || format!("{requested}×"),
                                        |index| format!("{requested}×  [{}]", index + 1),
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
