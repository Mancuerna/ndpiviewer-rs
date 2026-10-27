//! Two-point similarity alignment, channel comparisons and paired viewport exports.
use super::*;
#[derive(Clone, Copy)]
struct Alignment {
    left: egui::Vec2,
    right: egui::Vec2,
    scale: f32,
    angle: f32,
}
impl Alignment {
    fn from_pairs(points: [[Option<egui::Vec2>; 2]; 2]) -> Option<Self> {
        let [a, b] = [points[0][0]?, points[0][1]?];
        let [c, d] = [points[1][0]?, points[1][1]?];
        let u = b - a;
        let v = d - c;
        if u.length() < 1.0 || v.length() < 1.0 {
            return None;
        }
        let scale = v.length() / u.length();
        let angle = v.y.atan2(v.x) - u.y.atan2(u.x);
        (scale.is_finite() && (0.001..=1000.0).contains(&scale)).then_some(Self {
            left: a,
            right: c,
            scale,
            angle,
        })
    }
    fn map(self, p: egui::Vec2, inverse: bool) -> egui::Vec2 {
        if inverse {
            self.left + orientation::turn(p - self.right, -self.angle) / self.scale
        } else {
            self.right + orientation::turn(p - self.left, self.angle) * self.scale
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    pub aligning: bool,
    points: [[Option<egui::Vec2>; 2]; 2],
    alignment: Option<Alignment>,
    paths: [Option<PathBuf>; 2],
    channel_copy: Option<(usize, PathBuf, Position, u32)>,
    pub export: Option<PathBuf>,
    pub export_names: bool,
    capture_sent: bool,
    ready_frames: usize,
    message: String,
}
impl Comparison {
    pub(super) fn comparison_tools_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Compare",|ui| {
            if ui.add_enabled(self.viewers[self.active].slide.is_some(),egui::Button::new("Compare channels of this image")).clicked() {
                let source=&self.viewers[self.active];let path=source.slide.as_ref().unwrap().path.clone();
                self.tools.channel_copy=Some((1-self.active,path.clone(),Position::of(source),source.plane));
                let peer=1-self.active;self.viewers[peer].open(path);self.split=true;self.linked=true;self.tools.alignment=None;ui.close_menu();
            }
            ui.small("Opens the same image in the other pane. Choose different channels in each pane; position, magnification and focus stay linked. The comparison pane does not overwrite saved channel settings.");
            ui.separator();
            let ready=self.split && self.viewers.iter().all(|v|v.slide.is_some());
            ui.add_enabled_ui(ready,|ui| {
                if ui.checkbox(&mut self.tools.aligning,"Align using two landmark pairs").changed() { self.linked=false; }
                if self.tools.aligning {
                    ui.small("Center the same landmark under each crosshair, then record point 1 in each image. Repeat with a distant landmark for point 2. Alignment is a visual similarity transform, not tissue registration.");
                    for side in 0..2 {
                        ui.horizontal(|ui| {
                            for point in 0..2 {
                                let set=self.tools.points[side][point].is_some();
                                if ui.button(format!("{} point {}{}",["Left","Right"][side],point+1,if set {" ✓"} else {""})).clicked() {
                                    self.tools.points[side][point]=Some(self.viewers[side].center);
                                }
                            }
                        });
                    }
                    if ui.button("Apply alignment").clicked() {
                        if let Some(alignment)=Alignment::from_pairs(self.tools.points) {
                            self.tools.alignment=Some(alignment);self.tools.paths=self.viewers.each_ref().map(|v|v.slide.as_ref().map(|s|s.path.clone()));self.linked=true;
                            self.viewers[0].features.rotation=0;self.viewers[1].features.rotation=0;
                            self.viewers[0].features.mirrored=[false;2];self.viewers[1].features.mirrored=[false;2];
                            self.viewers[0].features.angle=0.0;self.viewers[1].features.angle= -alignment.angle;
                            self.tools.aligning=false;self.synchronize_landmarks();
                            self.tools.message="Alignment applied. Tracking pauses on arbitrarily rotated views to avoid marking unseen corners.".into();
                        } else {self.tools.message="Record two distinct points in each image.".into();}
                    }
                }
                if ui.button("Clear alignment").clicked() {self.tools.alignment=None;self.tools.points=Default::default();for v in &mut self.viewers {v.features.angle=0.0;v.changed();}}
                ui.separator();
                ui.checkbox(&mut self.tools.export_names,"Include image names in comparison export");
                if ui.add_enabled(self.viewers.iter().all(|v|v.capture_ready()),egui::Button::new("Export comparison PNG…")).clicked() {
                    if let Some(path)=rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name("comparison.png").save_file() {self.tools.export=Some(path);self.tools.capture_sent=false;self.tools.ready_frames=0;ui.close_menu();ui.ctx().request_repaint();}
                }
                ui.small("Exports the two visible fields with their displayed scales and annotations. Wait for both images to finish loading.");
            });
            if !self.tools.message.is_empty(){ui.label(&self.tools.message);}
        });
    }
    pub(super) fn synchronize_landmarks(&mut self) -> bool {
        let Some(a) = self.tools.alignment else {
            return false;
        };
        if self.tools.paths
            != self
                .viewers
                .each_ref()
                .map(|v| v.slide.as_ref().map(|s| s.path.clone()))
        {
            self.tools.alignment = None;
            return false;
        }
        let source = &self.viewers[self.active];
        let center = a.map(source.center, self.active == 1);
        let zoom = if self.active == 0 {
            source.zoom / a.scale
        } else {
            source.zoom * a.scale
        };
        let angle = if self.active == 0 {
            source.features.angle - a.angle
        } else {
            source.features.angle + a.angle
        };
        let peer = &mut self.viewers[1 - self.active];
        let zoom = zoom.clamp(0.0001, 8.0);
        if peer.center != center || peer.zoom != zoom || peer.features.angle != angle {
            peer.center = center;
            peer.zoom = zoom;
            peer.features.angle = angle;
            peer.fit = false;
            peer.changed();
        }
        true
    }
    pub(super) fn poll_comparison_tools(&mut self, ctx: &egui::Context) {
        let paths = self
            .viewers
            .each_ref()
            .map(|v| v.slide.as_ref().map(|s| s.path.clone()));
        if paths != self.tools.paths {
            self.tools.paths = paths;
            self.tools.points = Default::default();
            self.tools.alignment = None;
            for viewer in &mut self.viewers {
                if viewer.features.angle != 0.0 {
                    viewer.features.angle = 0.0;
                    viewer.changed();
                }
            }
        }
        if let Some((peer, path, position, plane)) = self.tools.channel_copy.clone() {
            if self.viewers[peer]
                .slide
                .as_ref()
                .is_some_and(|s| s.path == path)
                && self.viewers[peer].pending_open.is_none()
            {
                let viewer = &mut self.viewers[peer];
                viewer.center = position.center;
                viewer.zoom = position.zoom;
                viewer.features.rotation = position.rotation;
                viewer.features.mirrored = position.mirrored;
                viewer.features.angle = position.angle;
                viewer.plane = plane;
                viewer.fit = false;
                viewer.channels.transient = true;
                viewer.prefer_channel_controls();
                viewer.changed();
                self.tools.channel_copy = None;
            }
        }
        if self.split
            && self.linked
            && self.viewers.iter().any(|v| v.channels.transient)
            && self.viewers[0]
                .slide
                .as_ref()
                .zip(self.viewers[1].slide.as_ref())
                .is_some_and(|(a, b)| a.path == b.path)
        {
            let plane = self.viewers[self.active].plane;
            let peer = &mut self.viewers[1 - self.active];
            if peer.plane != plane {
                peer.plane = plane;
                peer.changed();
            }
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let (Some(image), Some(path)) = (screenshot, self.tools.export.as_ref()) {
            let rect = self.pane_rects[0].union(self.pane_rects[1]);
            let result = crate::features::capture::crop_view(&image, rect, ctx.screen_rect())
                .and_then(|crop| {
                    crate::features::capture::save_view(&crop, path, image::ImageFormat::Png)
                });
            self.tools.message = result.map_or_else(
                |e| format!("Comparison export failed: {e}"),
                |_| "Comparison exported.".into(),
            );
            self.tools.export = None;
            self.tools.capture_sent = false;
        }
        if self.tools.export.is_some() {
            if self.viewers.iter().all(|v| v.capture_ready()) {
                self.tools.ready_frames += 1;
            } else {
                self.tools.ready_frames = 0;
            }
            ctx.request_repaint();
        }
        if self.tools.export.is_some() && !self.tools.capture_sent && self.tools.ready_frames >= 3 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.tools.capture_sent = true;
            ctx.request_repaint();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn landmarks_map_both_points_and_roundtrip_arbitrary_rotation() {
        let p = [
            [Some(egui::vec2(10., 20.)), Some(egui::vec2(20., 20.))],
            [Some(egui::vec2(30., 40.)), Some(egui::vec2(30., 60.))],
        ];
        let a = Alignment::from_pairs(p).unwrap();
        assert!((a.map(p[0][1].unwrap(), false) - p[1][1].unwrap()).length() < 0.001);
        let x = egui::vec2(200., -70.);
        assert!((a.map(a.map(x, false), true) - x).length() < 0.001);
        assert!(Alignment::from_pairs([[Some(x); 2]; 2]).is_none());
    }
}
