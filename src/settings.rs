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
    pub arrow_step: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            map_height: 150.0,
            arrow_step: 100.0,
        }
    }
}
impl Preferences {
    fn valid(&self) -> bool {
        self.map_height.is_finite()
            && (100.0..=320.0).contains(&self.map_height)
            && self.arrow_step.is_finite()
            && (10.0..=1000.0).contains(&self.arrow_step)
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
    fn navigation_preferences_survive_restart_and_invalid_files_are_safe() {
        let root = std::env::temp_dir().join(format!("ndpi-navigation-{}", std::process::id()));
        let path = root.join("navigation.json");
        assert_eq!(Preferences::load(&path), Preferences::default());
        let expected = Preferences {
            map_height: 275.0,
            arrow_step: 350.0,
        };
        expected.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), expected);
        Preferences {
            map_height: 320.0,
            arrow_step: 10.0,
        }
        .save(&path)
        .unwrap();
        assert_eq!(Preferences::load(&path).arrow_step, 10.0);
        std::fs::write(&path, br#"{"map_height":9999,"arrow_step":0}"#).unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        assert!(Preferences {
            arrow_step: f32::NAN,
            ..expected
        }
        .save(&path)
        .is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
