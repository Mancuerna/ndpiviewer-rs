use std::path::{Path, PathBuf};

const LIMIT: usize = 20;

#[derive(Default)]
pub(super) struct Recent {
    pub paths: Vec<PathBuf>,
    pub error: Option<String>,
    path: Option<PathBuf>,
}

impl Recent {
    pub fn load() -> Self {
        let path = super::settings::path().map(|p| p.with_file_name("recent-images.json"));
        let paths = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .filter(|bytes| bytes.len() <= 256 * 1024)
            .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            paths: paths.into_iter().take(LIMIT).collect(),
            path,
            error: None,
        }
    }

    pub fn record(&mut self, path: &Path) {
        self.paths.retain(|existing| existing != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(LIMIT);
        self.save();
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.save();
    }

    fn save(&mut self) {
        let Some(path) = &self.path else { return };
        let result = (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = path.with_extension("tmp");
            std::fs::write(&temporary, serde_json::to_vec_pretty(&self.paths)?)?;
            std::fs::rename(temporary, path)
        })();
        self.error = result
            .err()
            .map(|error: std::io::Error| format!("Could not save recent images: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_paths_are_bounded_unique_and_clearable_without_touching_images() {
        let mut recent = Recent::default();
        for i in 0..30 {
            recent.record(Path::new(&format!("slide-{i}.ndpi")));
        }
        assert_eq!(recent.paths.len(), LIMIT);
        recent.record(Path::new("slide-15.ndpi"));
        assert_eq!(recent.paths[0], PathBuf::from("slide-15.ndpi"));
        assert_eq!(recent.paths.len(), LIMIT);
        recent.clear();
        assert!(recent.paths.is_empty());
    }
}
