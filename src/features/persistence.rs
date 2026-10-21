//! Bounded NDPA persistence and protection against external sidecar edits.
use super::{circle, polygon, Annotation, Features, Tool};
use crate::{ndpa, Slide};
use std::path::PathBuf;

pub(super) fn annotation_path(slide: &Slide) -> PathBuf {
    if slide.path.is_dir() {
        slide.path.join("annotations.ndpa")
    } else {
        let mut name = slide.path.as_os_str().to_os_string();
        name.push(".ndpa");
        PathBuf::from(name)
    }
}

fn read_annotations(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Annotation file exceeds 16 MiB.".into());
    }
    Ok(bytes)
}

pub(super) fn validate(annotations: &[Annotation], slide: &Slide) -> bool {
    annotations.len() <= 10000
        && annotations.iter().all(|a| {
            a.plane < slide.planes
                && a.tool != Tool::Pan
                && match a.tool {
                    Tool::Polygon => polygon::valid(&a.points),
                    Tool::Circle => circle::valid(&a.points, slide.calibration),
                    Tool::Line | Tool::Pointer | Tool::Rectangle => a.points.len() == 2,
                    Tool::Pan | Tool::Freehand => false,
                }
                && a.note.len() <= 40000
                && a.appearance.title.len() <= 4000
                && a.points.iter().all(|p| {
                    p[0].is_finite()
                        && p[1].is_finite()
                        && p[0].abs() <= 1_000_000_000.0
                        && p[1].abs() <= 1_000_000_000.0
                })
        })
}

impl Features {
    pub(crate) fn load_annotations(&mut self, slide: &Slide) {
        let path = annotation_path(slide);
        if path.exists() {
            let result = (|| {
                let bytes = read_annotations(&path)?;
                let xml = String::from_utf8(bytes.clone()).map_err(|e| e.to_string())?;
                let (archive, annotations) = ndpa::Archive::read(xml, slide)?;
                self.last_saved = Some(bytes);
                self.status = Some(if archive.unsupported == 0 {
                    "Annotations: Hamamatsu NDPA".into()
                } else {
                    format!(
                        "Annotations: NDPA ({} unsupported entries preserved, not displayed).",
                        archive.unsupported
                    )
                });
                self.archive = Some(archive);
                self.annotations = annotations;
                Ok::<_, String>(())
            })();
            if let Err(e) = result {
                crate::diagnostics::record(crate::diagnostics::Event::AnnotationLoadFailed);
                self.writable = false;
                self.error = Some(format!(
                    "NDPA not loaded: {e} Existing file preserved; changes are session-only."
                ));
            }
            return;
        }
        self.writable = ndpa::available(slide);
        self.status = Some(if self.writable {
            "Annotations: Hamamatsu NDPA".into()
        } else {
            "Annotations are session-only: NDPA saving requires physical calibration, slide-centre offsets and Z metadata.".into()
        });
    }
    pub(super) fn save(&mut self, slide: &Slide) {
        if !self.writable {
            return;
        }
        let result = (|| -> Result<(), String> {
            if !ndpa::available(slide) {
                return Err("NDPA saving requires physical calibration, slide-centre offsets and Z metadata. Annotations are session-only.".into());
            }
            if !validate(&self.annotations, slide) {
                return Err("Invalid annotation coordinates.".into());
            }
            let xml = if let Some(archive) = &self.archive {
                archive.write(&self.annotations, slide)?
            } else {
                ndpa::write_new(&self.annotations, slide)?
            };
            let path = annotation_path(slide);
            let bytes = xml.into_bytes();
            // Detect edits by another viewer before replacing the sidecar.
            let current = match read_annotations(&path) {
                Ok(bytes) => Some(bytes),
                Err(_) if !path.exists() => None,
                Err(e) => return Err(e),
            };
            if current != self.last_saved {
                self.writable = false;
                return Err("The sidecar changed outside this viewer. Reopen the image before editing; changes are session-only.".into());
            }
            use std::io::Write;
            let mut temp_name = path.as_os_str().to_os_string();
            temp_name.push(format!(".{}.tmp", std::process::id()));
            let temp = PathBuf::from(temp_name);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|e| e.to_string())?;
            let written = file.write_all(&bytes).and_then(|_| file.sync_all());
            drop(file);
            let result = written.and_then(|_| std::fs::rename(&temp, &path));
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result.map_err(|e| e.to_string())?;
            self.last_saved = Some(bytes);
            Ok(())
        })();
        if result.is_err() {
            crate::diagnostics::record(crate::diagnostics::Event::AnnotationSaveFailed);
        }
        self.error = result.err().map(|e| format!("Annotations not saved: {e}"));
    }
}
