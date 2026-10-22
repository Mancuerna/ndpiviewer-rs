use super::*;

fn reflect(v: egui::Vec2, mirrored: [bool; 2]) -> egui::Vec2 {
    egui::vec2(
        if mirrored[0] { -v.x } else { v.x },
        if mirrored[1] { -v.y } else { v.y },
    )
}

/// Mirrors operate on the displayed horizontal and vertical axes after rotation.
pub(super) fn forward(v: egui::Vec2, rotation: u8, mirrored: [bool; 2]) -> egui::Vec2 {
    reflect(features::rotate(v, rotation), mirrored)
}

pub(super) fn inverse(v: egui::Vec2, rotation: u8, mirrored: [bool; 2]) -> egui::Vec2 {
    features::rotate(reflect(v, mirrored), (4 - rotation % 4) % 4)
}

impl Viewer {
    pub(super) fn toggle_overview(&mut self) {
        if self.slide.is_none() {
            return;
        }
        self.features.cancel_gesture();
        if let Some((center, zoom)) = self.features.overview_return.take() {
            self.center = center;
            self.zoom = zoom;
            self.fit = false;
            self.changed();
        } else {
            self.features.overview_return = Some((self.center, self.zoom));
            self.fit = true;
        }
    }

    pub(super) fn toggle_mirror(&mut self, axis: usize) {
        self.features.cancel_gesture();
        self.features.mirrored[axis] = !self.features.mirrored[axis];
        self.changed();
    }

    pub(super) fn orientation_menu(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.slide.is_some(), |ui| {
            if ui.button(if self.features.overview_return.is_some() { "Return to detail [Tab]" } else { "Overview [Tab]" })
                .on_hover_text("Temporary whole-slide view; press Tab again to restore the previous position and zoom.")
                .clicked()
            {
                self.toggle_overview();
            }
            ui.menu_button("Orientation", |ui| {
                if ui.button("Rotate clockwise 90°    R").clicked() {
                    self.features.cancel_gesture();
                    self.features.rotation = (self.features.rotation + 1) % 4;
                    self.changed();
                }
                if ui.selectable_label(self.features.mirrored[0], "Mirror horizontally    Alt+M").clicked() {
                    self.toggle_mirror(0);
                }
                if ui.selectable_label(self.features.mirrored[1], "Flip vertically    Alt+F").clicked() {
                    self.toggle_mirror(1);
                }
                if ui.button("Reset orientation").clicked() {
                    self.features.cancel_gesture();
                    self.features.rotation = 0;
                    self.features.mirrored = [false; 2];
                    self.changed();
                }
                ui.small("Applies to the selected image. Source pixels and annotation coordinates stay unchanged.");
            });
            if self.features.mirrored[0] || self.features.mirrored[1] {
                ui.label(match self.features.mirrored {
                    [true, true] => "Mirrored H + V",
                    [true, false] => "Mirrored H",
                    _ => "Flipped V",
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_overview_restores_detail_without_resetting_z_or_orientation() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let original = (viewer.center, viewer.zoom);
        viewer.toggle_overview();
        assert!(viewer.fit);
        viewer.center = egui::vec2(100.0, 300.0);
        viewer.zoom = 0.02;
        viewer.features.mirrored = [true, false];
        viewer.toggle_overview();
        assert!(!viewer.fit);
        assert_eq!((viewer.center, viewer.zoom), original);
        assert_eq!(viewer.features.mirrored, [true, false]);
        assert!(viewer.features.overview_return.is_none());
    }

    #[test]
    fn every_orientation_round_trips_annotation_and_pointer_coordinates() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let rect = egui::Rect::from_min_size(egui::pos2(340.0, 80.0), egui::vec2(700.0, 600.0));
        for rotation in 0..4 {
            for mirrored in [[false, false], [true, false], [false, true], [true, true]] {
                viewer.features.rotation = rotation;
                viewer.features.mirrored = mirrored;
                for zoom in [0.04, 0.5, 2.0] {
                    viewer.zoom = zoom;
                    for point in [viewer.center, viewer.center + egui::vec2(40.0, -70.0)] {
                        let screen = viewer.screen_point(point, rect);
                        let restored =
                            viewer.center + viewer.inverse_transform(screen - rect.center()) / zoom;
                        assert!((point - restored).length() < 0.01);
                    }
                    let region = [9900.0, 7900.0, 200.0, 200.0];
                    let tile = viewer.tile_rect(region, rect);
                    assert!((tile.width() - 200.0 * zoom).abs() < 0.01);
                    assert!(tile.contains(viewer.screen_point(viewer.center, rect)));
                }
            }
        }
    }

    #[test]
    fn reflection_preserves_length_and_is_screen_relative_after_rotation() {
        let v = egui::vec2(3.0, 4.0);
        assert_eq!(forward(v, 1, [true, false]), egui::vec2(4.0, 3.0));
        assert_eq!(forward(v, 1, [false, true]), egui::vec2(-4.0, -3.0));
        for rotation in 0..4 {
            for flags in [[false, false], [true, false], [false, true], [true, true]] {
                assert_eq!(forward(v, rotation, flags).length_sq(), v.length_sq());
                assert_eq!(inverse(forward(v, rotation, flags), rotation, flags), v);
            }
        }
    }
}
