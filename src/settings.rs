use std::path::{Path, PathBuf};

pub(super) fn path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root)
            .join("ndpiviewer-rs")
            .join("cache-mib.txt")
    })
}

pub(super) fn load(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .filter(|mib| [96, 256, 512].contains(mib))
        .unwrap_or(512)
        * 1048576
}

pub(super) fn save(path: &Path, bytes: usize) -> std::io::Result<()> {
    if ![96, 256, 512].iter().any(|mib| bytes == mib * 1048576) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Unsupported cache budget",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, format!("{}\n", bytes / 1048576))?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_setting_survives_reload_and_rejects_invalid_values() {
        let root =
            std::env::temp_dir().join(format!("ndpiviewer-settings-test-{}", std::process::id()));
        let path = root.join("cache-mib.txt");
        assert_eq!(load(&path), 512 * 1048576);
        for mib in [96, 256, 512] {
            save(&path, mib * 1048576).unwrap();
            assert_eq!(load(&path), mib * 1048576);
        }
        for invalid in ["", "0", "1024", "garbage", "18446744073709551615"] {
            std::fs::write(&path, invalid).unwrap();
            assert_eq!(load(&path), 512 * 1048576);
        }
        assert!(save(&path, 123).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

/// Application preferences, independent of the currently open slide.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub map_height: f32,
    pub arrow_percent: f32,
    pub auto_hide_sidebar: bool,
    pub prefetch_neighbors: bool,
    pub show_minimap: bool,
    pub sidebar_override: Option<bool>,
    pub ui_scale: f32,
    pub label_thumbnails: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            map_height: 150.0,
            arrow_percent: 25.0,
            auto_hide_sidebar: true,
            prefetch_neighbors: true,
            show_minimap: true,
            sidebar_override: None,
            ui_scale: 1.0,
            label_thumbnails: false,
        }
    }
}
impl Preferences {
    fn valid(&self) -> bool {
        self.ui_scale.is_finite()
            && (0.75..=2.0).contains(&self.ui_scale)
            && self.map_height.is_finite()
            && (100.0..=320.0).contains(&self.map_height)
            && self.arrow_percent.is_finite()
            && (1.0..=100.0).contains(&self.arrow_percent)
    }
    pub fn path() -> Option<PathBuf> {
        path().map(|p| p.with_file_name("navigation.json"))
    }
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(Self::valid)
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if !self.valid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid navigation preferences",
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self)?;
        crate::export_file::write(path, |file| {
            use std::io::Write;
            file.write_all(&bytes).map_err(|e| e.to_string())
        })
        .map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    #[test]
    fn old_pixel_preference_does_not_become_a_percentage() {
        let prefs: Preferences =
            serde_json::from_str(r#"{"map_height":275,"arrow_step":550}"#).unwrap();
        assert_eq!(prefs.map_height, 275.0);
        assert_eq!(prefs.arrow_percent, 25.0);
        assert!(prefs.auto_hide_sidebar && prefs.prefetch_neighbors);
    }
    #[test]
    fn navigation_preferences_survive_restart_and_invalid_files_are_safe() {
        let root = std::env::temp_dir().join(format!("ndpi-navigation-{}", std::process::id()));
        let path = root.join("navigation.json");
        assert_eq!(Preferences::load(&path), Preferences::default());
        let expected = Preferences {
            map_height: 275.0,
            arrow_percent: 35.0,
            show_minimap: false,
            sidebar_override: Some(false),
            ui_scale: 1.5,
            label_thumbnails: true,
            ..Default::default()
        };
        expected.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), expected);
        Preferences {
            map_height: 320.0,
            arrow_percent: 10.0,
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        assert_eq!(Preferences::load(&path).arrow_percent, 10.0);
        std::fs::write(&path, br#"{"map_height":9999,"arrow_percent":0}"#).unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        assert!(Preferences {
            arrow_percent: f32::NAN,
            ..expected
        }
        .save(&path)
        .is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
