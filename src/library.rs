//! Folder browsing and a bounded, lazy thumbnail cache, independent of viewer readers.
use crate::{background::Task, channels, source::Source, Region};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

pub(crate) fn supported(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ndpi") || e.eq_ignore_ascii_case("ndpis"))
}

pub(crate) fn thumbnail(path: PathBuf) -> Result<egui::ColorImage, String> {
    let (mut source, slide) = Source::open(path)?;
    let level = slide.levels.last().ok_or("No pyramid level")?;
    let factor = (192.0 / level.width.max(level.height) as f64).min(1.0);
    let r = Region {
        sampling: 1,
        preview: true,
        series: level.series,
        interleaved: level.interleaved,
        plane: slide.reference_plane().unwrap_or(0),
        x: 0,
        y: 0,
        w: (level.width as f64 * factor).round().max(1.0) as u32,
        h: (level.height as f64 * factor).round().max(1.0) as u32,
        world: [0.0, 0.0, slide.width as f32, slide.height as f32],
    };
    source
        .read_display(&r, &channels::defaults(&slide.channels))
        .map(|(image, _)| image)
}

type Scan = Result<(PathBuf, Vec<PathBuf>), String>;
type Thumbnail = (PathBuf, Result<egui::ColorImage, String>);
#[derive(Default)]
pub(crate) struct Library {
    pub open: bool,
    selected: Option<PathBuf>,
    scroll_to_selection: bool,
    folder: Option<PathBuf>,
    paths: Vec<PathBuf>,
    filter: String,
    scan: Option<Task<Scan>>,
    thumbnails: Vec<(PathBuf, Task<Thumbnail>)>,
    pending_folder: Option<PathBuf>,
    cache: VecDeque<(PathBuf, Result<egui::TextureHandle, String>)>,
    error: Option<String>,
}

impl Library {
    /// Tab cycles matching slides in this library; text fields retain normal Tab behavior.
    pub fn cycle_shortcut(
        &mut self,
        ctx: &egui::Context,
        current: Option<&Path>,
    ) -> Option<PathBuf> {
        if !self.open
            || self.scan.is_some()
            || ctx.memory(|m| m.any_popup_open() || m.has_focus(egui::Id::new("library-filter")))
        {
            return None;
        }
        // Read modifiers from the key event: Shift may already be released by this frame.
        let modifiers = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if *modifiers == egui::Modifiers::NONE
                    || *modifiers == egui::Modifiers::SHIFT =>
                {
                    Some(*modifiers)
                }
                _ => None,
            })
        })?;
        let backward = modifiers.shift;
        if !ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::Tab)) {
            return None;
        }
        ctx.memory_mut(|m| {
            let id = egui::Id::new("library-cycle-focus");
            m.interested_in_focus(id, egui::LayerId::background());
            m.surrender_focus(id);
        });
        let query = self.filter.to_lowercase();
        let paths: Vec<_> = self
            .paths
            .iter()
            .filter(|p| {
                supported(p)
                    && !p.is_dir()
                    && p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect();
        let selected = cycle_path(&paths, current.or(self.selected.as_deref()), backward)?;
        self.selected = Some(selected.clone());
        self.scroll_to_selection = true;
        Some(selected)
    }

    pub fn follow_slide(&mut self, ctx: &egui::Context, path: &Path) {
        if !supported(path) {
            return;
        }
        self.selected = Some(path.to_path_buf());
        self.scroll_to_selection = true;
        if let Some(folder) = path.parent() {
            if self.folder.as_deref() != Some(folder) {
                self.scan(ctx, folder.to_path_buf());
            } else {
                self.pending_folder = None;
            }
        }
    }

    fn scan(&mut self, ctx: &egui::Context, folder: PathBuf) {
        if self.scan.is_some() {
            self.pending_folder = Some(folder);
            return;
        }
        if self.folder.as_ref() != Some(&folder) {
            self.paths.clear();
            self.filter.clear();
        }
        self.folder = Some(folder.clone());
        self.error = None;
        self.scan = Some(Task::spawn(ctx, move || {
            let entries = std::fs::read_dir(&folder).map_err(|e| e.to_string())?;
            let mut paths = Vec::new();
            for (count, entry) in entries.enumerate() {
                if count >= 20000 {
                    return Err("Folder contains too many entries. Choose a smaller folder.".into());
                }
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if entry.file_type().map_err(|e| e.to_string())?.is_dir() || supported(&path) {
                    paths.push(path);
                }
                if paths.len() > 4096 {
                    return Err("Choose a folder with at most 4096 slides and subfolders.".into());
                }
            }
            paths.sort_by_cached_key(|p| {
                (
                    !p.is_dir(),
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
            });
            Ok((folder, paths))
        }));
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        if let Some(result) = self.scan.as_ref().and_then(Task::take) {
            self.scan = None;
            match result {
                Ok((folder, paths)) => {
                    self.folder = Some(folder);
                    self.paths = paths;
                    self.filter.clear();
                    if self.pending_folder.is_none()
                        && !self.paths.iter().any(|p| Some(p) == self.selected.as_ref())
                    {
                        self.selected = None;
                    }
                }
                Err(e) => {
                    crate::diagnostics::record(crate::diagnostics::Event::LibraryReadFailed);
                    self.error = Some(e);
                }
            }
        }
        if self.scan.is_none() {
            if let Some(folder) = self.pending_folder.take() {
                self.scan(ctx, folder);
            }
        }
        for index in (0..self.thumbnails.len()).rev() {
            let Some((path, result)) = self.thumbnails[index].1.take() else {
                continue;
            };
            self.thumbnails.swap_remove(index);
            if self.paths.contains(&path) {
                let texture = result.map(|image| {
                    ctx.load_texture(
                        path.display().to_string(),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                });
                self.cache.push_back((path, texture));
                while self.cache.len() > 48 {
                    self.cache.pop_front();
                }
            }
        }
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut folder = None;
        let mut selected = None;
        let mut next_thumbnails = Vec::new();
        egui::Window::new("Slide library")
            .open(&mut open)
            .default_size([660.0, 620.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.scan.is_none(), egui::Button::new("Choose folder…"))
                        .clicked()
                    {
                        folder = rfd::FileDialog::new().pick_folder();
                    }
                    if ui
                        .add_enabled(
                            self.scan.is_none()
                                && self.folder.as_ref().and_then(|p| p.parent()).is_some(),
                            egui::Button::new("Up"),
                        )
                        .clicked()
                    {
                        folder = self
                            .folder
                            .as_ref()
                            .and_then(|p| p.parent())
                            .map(Path::to_path_buf);
                    }
                    if ui
                        .add_enabled(
                            self.scan.is_none() && self.folder.is_some(),
                            egui::Button::new("Refresh"),
                        )
                        .clicked()
                    {
                        self.cache.clear();
                        folder = self.folder.clone();
                    }
                });
                if let Some(path) = &self.folder {
                    ui.label(path.display().to_string());
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter).id(egui::Id::new("library-filter")).hint_text("Filter by filename"),
                );
                ui.small("Tab / Shift+Tab: next / previous slide in this library. Thumbnails load as you browse.");
                if self.scan.is_some() {
                    ui.label("Reading folder…");
                }
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                let query = self.filter.to_lowercase();
                let filtered: Vec<_> = self
                    .paths
                    .iter()
                    .filter(|p| {
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&query)
                    })
                    .collect();
                if filtered.is_empty() && self.scan.is_none() {
                    ui.label("No matching slides or folders.");
                }
                let mut scroll = egui::ScrollArea::vertical();
                if self.scroll_to_selection {
                    if let Some(index) = filtered.iter().position(|p| Some(p.as_path()) == self.selected.as_deref()) {
                        scroll = scroll.vertical_scroll_offset(index as f32 * (98.0 + ui.spacing().item_spacing.y));
                    }
                    self.scroll_to_selection = false;
                }
                scroll.show_rows(ui, 98.0, filtered.len(), |ui, range| {
                    for row in range {
                        let path = filtered[row];
                        ui.push_id(path, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(110.0, 90.0),
                                    egui::Sense::hover(),
                                );
                                if path.is_dir() {
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "Folder",
                                        egui::FontId::proportional(16.0),
                                        egui::Color32::LIGHT_BLUE,
                                    );
                                } else if let Some(index) =
                                    self.cache.iter().position(|(p, _)| p == path)
                                {
                                    let entry = self.cache.remove(index).unwrap();
                                    match &entry.1 {
                                        Ok(texture) => {
                                            let size = texture.size_vec2();
                                            let fit =
                                                (rect.width() / size.x).min(rect.height() / size.y);
                                            ui.painter().image(
                                                texture.id(),
                                                egui::Rect::from_center_size(
                                                    rect.center(),
                                                    size * fit,
                                                ),
                                                egui::Rect::from_min_max(
                                                    egui::Pos2::ZERO,
                                                    egui::pos2(1.0, 1.0),
                                                ),
                                                egui::Color32::WHITE,
                                            );
                                        }
                                        Err(e) => {
                                            crate::diagnostics::record(
                                                crate::diagnostics::Event::LibraryReadFailed,
                                            );
                                            ui.label("Preview unavailable").on_hover_text(e);
                                        }
                                    }
                                    self.cache.push_back(entry);
                                } else {
                                    if !self.thumbnails.iter().any(|(p, _)| p == path) {
                                        next_thumbnails.push(path.clone());
                                    }
                                }
                                ui.vertical(|ui| {
                                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                                    if self.selected.as_ref() == Some(path) { ui.colored_label(egui::Color32::LIGHT_BLUE, name); }
                                    else { ui.label(name); }
                                    if path.is_dir() {
                                        if ui
                                            .add_enabled(
                                                self.scan.is_none(),
                                                egui::Button::new("Browse folder"),
                                            )
                                            .clicked()
                                        {
                                            folder = Some(path.clone());
                                        }
                                        if ui.small_button("Open as Z stack").clicked() {
                                            selected = Some(path.clone());
                                        }
                                    } else if ui.button("Open slide").clicked() {
                                        selected = Some(path.clone());
                                    }
                                });
                            });
                        });
                    }
                });
            });
        if let Some(path) = &selected {
            self.selected = Some(path.clone());
        }
        self.open = open && selected.is_none();
        if let Some(folder) = folder {
            self.scan(ctx, folder);
        }
        if self.scan.is_none() {
            for path in next_thumbnails
                .into_iter()
                .take(2_usize.saturating_sub(self.thumbnails.len()))
            {
                self.thumbnails.push((
                    path.clone(),
                    Task::spawn(ctx, move || {
                        let image = thumbnail(path.clone());
                        (path, image)
                    }),
                ));
            }
        }
        selected
    }
}

fn cycle_path(paths: &[PathBuf], current: Option<&Path>, backward: bool) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    let index = paths.iter().position(|p| Some(p.as_path()) == current);
    let next = match (index, backward) {
        (Some(i), false) => (i + 1) % paths.len(),
        (Some(i), true) => (i + paths.len() - 1) % paths.len(),
        (None, false) => 0,
        (None, true) => paths.len() - 1,
    };
    Some(paths[next].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn following_slides_scans_the_folder_and_keeps_latest_request() {
        let ctx = egui::Context::default();
        let root =
            std::env::temp_dir().join(format!("viewer-library-follow-{}", std::process::id()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let a = root.join("a/one.ndpi");
        let b = root.join("b/two.ndpis");
        for p in [
            &a,
            &b,
            &root.join("a/other.ndpi"),
            &root.join("a/notes.txt"),
        ] {
            std::fs::write(p, []).unwrap();
        }
        let mut library = Library::default();
        library.follow_slide(&ctx, &a);
        library.follow_slide(&ctx, &b);
        library.follow_slide(&ctx, &a); // A newer request cancels the queued folder.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while library.scan.is_some() || library.pending_folder.is_some() {
            library.show(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(library.folder.as_deref(), a.parent());
        assert_eq!(library.selected, Some(a.clone()));
        assert_eq!(library.paths.len(), 2);
        library.filter = "one".into();
        library.follow_slide(&ctx, &root.join("a/other.ndpi"));
        assert!(library.scan.is_none());
        assert_eq!(library.filter, "one");
        library.follow_slide(&ctx, &b);
        while library.scan.is_some() {
            library.show(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(library.paths, vec![b.clone()]);
        assert_eq!(library.selected, Some(b));
        assert!(library.filter.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Requires downloaded public slides"]
    fn public_preview_timings_and_embedded_images() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example_images");
        for name in [
            "02B0669241M.ndpi",
            "JNOC-GC2003.ndpi",
            "JARE59-CD.ndpi",
            "Hamamatsu_fluorescence_Manuel_Stritt/test3.ndpis",
        ] {
            let path = root.join(name);
            let start = std::time::Instant::now();
            let preview = thumbnail(path.clone()).unwrap();
            println!(
                "{name}: thumbnail {:?}, {:?}",
                preview.size,
                start.elapsed()
            );
            let start = std::time::Instant::now();
            let embedded = crate::associated::load(path.clone()).unwrap();
            println!(
                "{name}: {} embedded images, {:?}",
                embedded.len(),
                start.elapsed()
            );
            assert!(!embedded.is_empty());
            for (index, (_, image)) in embedded.iter().enumerate() {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    std::env::temp_dir().join(format!(
                        "viewer-label-{}-{index}.png",
                        path.file_stem().unwrap().to_string_lossy()
                    )),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
    }
    #[test]
    fn library_cycle_wraps_in_both_directions() {
        let paths: Vec<_> = ["a.ndpi", "b.ndpi", "c.ndpis"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(cycle_path(&paths, None, false), Some(paths[0].clone()));
        assert_eq!(
            cycle_path(&paths, Some(&paths[2]), false),
            Some(paths[0].clone())
        );
        assert_eq!(
            cycle_path(&paths, Some(&paths[0]), true),
            Some(paths[2].clone())
        );
        assert_eq!(
            cycle_path(&paths[..1], Some(&paths[0]), false),
            Some(paths[0].clone())
        );
        assert!(cycle_path(&[], None, false).is_none());
    }
    #[test]
    fn library_tab_filters_slides_and_respects_text_focus_and_modifiers() {
        let ctx = egui::Context::default();
        let mut library = Library {
            open: true,
            paths: vec!["a.ndpi".into(), "b.ndpi".into(), "notes.txt".into()],
            ..Default::default()
        };
        for (modifiers, text_focus, expected) in [
            (egui::Modifiers::CTRL, false, None),
            (egui::Modifiers::NONE, true, None),
            (egui::Modifiers::NONE, false, Some(PathBuf::from("a.ndpi"))),
            (egui::Modifiers::SHIFT, false, Some(PathBuf::from("b.ndpi"))),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    modifiers: egui::Modifiers::NONE,
                    ..Default::default()
                },
                |ctx| {
                    let id = egui::Id::new("library-filter");
                    ctx.memory_mut(|m| {
                        if text_focus {
                            m.request_focus(id);
                        } else {
                            m.surrender_focus(id);
                        }
                    });
                    assert_eq!(library.cycle_shortcut(ctx, None), expected);
                },
            );
        }
        library.filter = "b".into();
        assert_eq!(
            cycle_path(&[PathBuf::from("b.ndpi")], None, false),
            Some("b.ndpi".into())
        );
    }
}
