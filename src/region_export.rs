//! Full-resolution region export with bounded output, tile reads and cancellation.
use crate::{background::Task, channels, source::Source, Region, Slide, Viewer};
use eframe::egui;
use image::ImageEncoder;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
};

const MAX_PIXELS: u64 = 16_777_216;
#[cfg(test)]
#[path = "region_export_tests.rs"]
mod tests;

pub(crate) fn bounds(rect: egui::Rect, slide: &Slide) -> Option<[u32; 4]> {
    if !rect.is_finite() {
        return None;
    }
    let x = rect.min.x.floor().max(0.0) as u32;
    let y = rect.min.y.floor().max(0.0) as u32;
    let right = (rect.max.x.ceil().max(0.0) as u32).min(slide.width);
    let bottom = (rect.max.y.ceil().max(0.0) as u32).min(slide.height);
    (right > x && bottom > y).then(|| [x, y, right - x, bottom - y])
}

pub(crate) struct Job {
    pub slide: Slide,
    pub area: [u32; 4],
    pub plane: u32,
    pub settings: Vec<channels::Setting>,
    pub annotations: Vec<crate::annotations::Annotation>,
    pub scale: bool,
    pub labels: bool,
    pub annotation_style: [f32; 2],
    pub path: PathBuf,
}

pub(crate) fn export(job: Job, cancel: &AtomicBool, progress: &AtomicU32) -> Result<(), String> {
    let [x, y, w, h] = job.area;
    if w == 0
        || h == 0
        || u64::from(w) * u64::from(h) > MAX_PIXELS
        || x.checked_add(w).is_none_or(|v| v > job.slide.width)
        || y.checked_add(h).is_none_or(|v| v > job.slide.height)
        || job.plane >= job.slide.planes
    {
        return Err(
            "Select a valid region of at most 16 megapixels (64 MiB output buffer).".into(),
        );
    }
    if job.path == job.slide.path
        || !job
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err("Choose a separate .png output file.".into());
    }
    let (mut source, current) = Source::open(job.slide.path.clone())?;
    if current.width != job.slide.width
        || current.height != job.slide.height
        || current.planes != job.slide.planes
        || current.channels.len() != job.settings.len()
    {
        return Err("Source metadata changed. Reopen the slide before exporting.".into());
    }
    let level = current.levels.first().ok_or("No native-resolution level")?;
    if level.width != current.width || level.height != current.height {
        return Err("Native-resolution level unavailable.".into());
    }
    let mut output = image::RgbaImage::new(w, h);
    let total = w.div_ceil(512) * h.div_ceil(512);
    let mut completed = 0;
    for ty in (0..h).step_by(512) {
        for tx in (0..w).step_by(512) {
            if cancel.load(Ordering::Relaxed) {
                return Err("Export cancelled.".into());
            }
            let region = Region {
                sampling: 1,
                preview: false,
                series: level.series,
                interleaved: level.interleaved,
                plane: job.plane,
                x: x + tx,
                y: y + ty,
                w: 512.min(w - tx),
                h: 512.min(h - ty),
                world: [0.0; 4],
            };
            let (tile, _) = source.read_display(&region, &job.settings)?;
            for py in 0..region.h {
                for px in 0..region.w {
                    output.put_pixel(
                        tx + px,
                        ty + py,
                        image::Rgba(tile.pixels[(py * region.w + px) as usize].to_array()),
                    );
                }
            }
            completed += 1;
            progress.store(completed * 90 / total, Ordering::Relaxed);
        }
    }
    drop(source);
    if job.scale || !job.annotations.is_empty() {
        crate::export_overlay::paint(
            &mut output,
            &job.slide,
            [x, y],
            &job.annotations,
            crate::export_overlay::Style {
                scale: job.scale,
                labels: job.labels,
                annotation: job.annotation_style,
            },
            cancel,
        )?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled.".into());
    }
    progress.store(95, Ordering::Relaxed);
    crate::export_file::write(&job.path, |writer| {
        image::codecs::png::PngEncoder::new(writer)
            .write_image(output.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        Ok(())
    })?;
    progress.store(100, Ordering::Relaxed);
    Ok(())
}

pub(crate) struct Export {
    pub selecting: bool,
    start: Option<egui::Vec2>,
    pub area: Option<[u32; 4]>,
    pub scale: bool,
    pub annotations: bool,
    pub labels: bool,
    pub task: Option<Task<Result<PathBuf, String>>>,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<AtomicU32>,
    pub message: Option<String>,
}
impl Default for Export {
    fn default() -> Self {
        Self {
            selecting: false,
            start: None,
            area: None,
            scale: true,
            annotations: true,
            labels: true,
            task: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicU32::new(0)),
            message: None,
        }
    }
}
impl Export {
    pub fn poll(&mut self) {
        if let Some(result) = self.task.as_ref().and_then(Task::take) {
            self.task = None;
            self.message = Some(match result {
                Ok(path) => format!("Region saved: {}", path.display()),
                Err(e) => {
                    if !self.cancel.load(Ordering::Relaxed) {
                        crate::diagnostics::record(crate::diagnostics::Event::RegionExportFailed);
                    }
                    e
                }
            });
        }
    }
    pub fn reset_selection(&mut self) {
        self.selecting = false;
        self.start = None;
        self.area = None;
    }
    pub fn start(&mut self, ctx: &egui::Context, job: Job) {
        self.cancel = Arc::new(AtomicBool::new(false));
        self.progress = Arc::new(AtomicU32::new(0));
        self.message = None;
        let cancel = self.cancel.clone();
        let progress = self.progress.clone();
        self.task = Some(Task::spawn(ctx, move || {
            let path = job.path.clone();
            export(job, &cancel, &progress).map(|()| path)
        }));
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Viewer {
    pub(crate) fn select_export_region(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        let selecting = self.region_export.selecting;
        if selecting {
            if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                self.region_export.selecting = false;
                self.region_export.start = None;
                return true;
            }
            if self.shown_plane != Some(self.plane) {
                return true;
            }
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.region_export.start = ui
                    .ctx()
                    .input(|i| i.pointer.press_origin())
                    .map(|p| self.world_point(p, rect));
            }
            if let (Some(start), Some(end), Some(slide)) = (
                self.region_export.start,
                response.interact_pointer_pos(),
                self.slide.as_ref(),
            ) {
                let end = self.world_point(end, rect);
                self.region_export.area = bounds(
                    egui::Rect::from_two_pos(start.to_pos2(), end.to_pos2()),
                    slide,
                );
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                self.region_export.start = None;
                self.region_export.selecting = false;
            }
        }
        selecting
    }
    pub(crate) fn paint_export_region(&self, ui: &egui::Ui, rect: egui::Rect) {
        if let Some([x, y, w, h]) = self.region_export.area {
            if self.export_region_visible() {
                let selected = self.tile_rect([x as f32, y as f32, w as f32, h as f32], rect);
                ui.painter().with_clip_rect(rect).rect_stroke(
                    selected,
                    0.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}
