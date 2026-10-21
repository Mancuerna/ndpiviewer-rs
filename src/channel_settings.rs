use super::*;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Identity {
    width: u32,
    height: u32,
    planes: u32,
    rgb_components: bool,
    channel_names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> (PathBuf, Slide) {
        let root = std::env::temp_dir().join(format!(
            "ndpi-channel-settings-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let slide = Slide {
            calibration: None,
            channels: channels::rgb_channels(),
            rgb_components: true,
            channel_band: None,
            registration: vec![],
            ndpa: None,
            path: root.join("image.ndpi"),
            width: 100,
            height: 80,
            planes: 3,
            levels: vec![],
        };
        (root, slide)
    }
    fn cleanup(root: PathBuf) {
        for entry in std::fs::read_dir(&root).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn restores_every_setting_and_keeps_images_independent() {
        let (root, a) = fixture("roundtrip");
        let mut b = a.clone();
        b.path = root.join("image.ndpis"); // Same stem must not share preferences.
        let mut saved = Persistence::default();
        let mut values = saved.load(&a);
        values[0].enabled = false;
        values[1].color = [17, 42, 230];
        values[1].brightness = -0.375;
        values[2].contrast = 12.5;
        values[1].gamma = 2.2;
        saved.mark_changed();
        saved.flush(&a, &values); // Also used on close/switch before debounce expires.
        assert!(saved.message.is_none());
        let mut reopened = Persistence::default();
        assert_eq!(reopened.load(&a), values);
        assert_eq!(reopened.load(&b), channels::defaults(&b.channels));
        let reset = channels::defaults(&a.channels);
        saved.mark_changed();
        saved.flush(&a, &reset); // Replaces an existing document on Windows.
        assert!(saved.message.is_none());
        assert_eq!(reopened.load(&a), reset);
        cleanup(root);
    }

    #[test]
    fn rejects_corrupt_mismatched_and_unsupported_documents_without_overwriting() {
        let (root, slide) = fixture("invalid");
        let base = Document {
            version: 1,
            image: Identity::of(&slide),
            settings: channels::defaults(&slide.channels),
        };
        let mut invalid = vec![b"broken json".to_vec(), vec![b' '; MAX_BYTES as usize + 1]];
        for kind in 0..7 {
            let mut doc: Document =
                serde_json::from_slice(&serde_json::to_vec(&base).unwrap()).unwrap();
            match kind {
                0 => doc.version = 2,
                1 => doc.image.channel_names.swap(0, 1),
                2 => doc.image.width += 1,
                3 => {
                    doc.settings.pop();
                }
                4 => doc.settings[0].brightness = 1.1,
                5 => doc.settings[0].contrast = 0.0,
                _ => doc.settings[0].gamma = 0.0,
            }
            invalid.push(serde_json::to_vec(&doc).unwrap());
        }
        for bytes in invalid {
            std::fs::write(path(&slide), &bytes).unwrap();
            let mut persistence = Persistence::default();
            let values = persistence.load(&slide);
            assert_eq!(values, channels::defaults(&slide.channels));
            assert!(persistence.message.is_some());
            persistence.mark_changed();
            persistence.flush(&slide, &values);
            assert_eq!(std::fs::read(path(&slide)).unwrap(), bytes);
        }
        let mut values = channels::defaults(&slide.channels);
        values[0].contrast = f32::NAN;
        assert!(!valid(&values, 3));
        cleanup(root);
    }

    #[test]
    fn failed_save_preserves_last_saved_values_and_reports_error() {
        let (root, slide) = fixture("write-failure");
        let mut persistence = Persistence::default();
        let values = persistence.load(&slide);
        persistence.mark_changed();
        persistence.flush(&slide, &values);
        let original = std::fs::read(path(&slide)).unwrap();
        let temporary = PathBuf::from(format!(
            "{}.{}.tmp",
            path(&slide).display(),
            std::process::id()
        ));
        std::fs::write(&temporary, b"another pending write").unwrap();
        let mut edited = values.clone();
        edited[0].enabled = false;
        persistence.mark_changed();
        persistence.flush(&slide, &edited);
        assert!(persistence.message.as_ref().unwrap().contains("not saved"));
        assert_eq!(std::fs::read(path(&slide)).unwrap(), original);
        assert_eq!(std::fs::read(&temporary).unwrap(), b"another pending write");
        cleanup(root);
    }

    #[test]
    fn folder_stacks_store_settings_inside_the_folder() {
        let (root, mut slide) = fixture("folder");
        slide.path = root.clone();
        let mut persistence = Persistence::default();
        let mut values = persistence.load(&slide);
        values[2].brightness = 0.4;
        persistence.mark_changed();
        persistence.flush(&slide, &values);
        assert_eq!(path(&slide), root.join("channel-settings.json"));
        assert_eq!(Persistence::default().load(&slide), values);
        cleanup(root);
    }
}

impl Identity {
    fn of(slide: &Slide) -> Self {
        Self {
            width: slide.width,
            height: slide.height,
            planes: slide.planes,
            rgb_components: slide.rgb_components,
            channel_names: slide.channels.iter().map(|c| c.name.clone()).collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    image: Identity,
    settings: Vec<channels::Setting>,
}

fn path(slide: &Slide) -> PathBuf {
    if slide.path.is_dir() {
        slide.path.join("channel-settings.json")
    } else {
        let mut name = slide.path.as_os_str().to_os_string();
        name.push(".channels.json");
        PathBuf::from(name)
    }
}

fn valid(settings: &[channels::Setting], count: usize) -> bool {
    settings.len() == count
        && count > 0
        && count <= 64
        && settings.iter().all(channels::Setting::is_valid)
}

#[derive(Default)]
pub(super) struct Persistence {
    pending: bool,
    blocked: bool,
    pub message: Option<String>,
}

impl Persistence {
    pub fn load(&mut self, slide: &Slide) -> Vec<channels::Setting> {
        *self = Self::default();
        let result = (|| -> Result<Option<Vec<channels::Setting>>, String> {
            let file = match std::fs::File::open(path(slide)) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.to_string()),
            };
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err("Settings file exceeds 64 KiB.".into());
            }
            let document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if document.version != 1
                || document.image != Identity::of(slide)
                || !valid(&document.settings, slide.channels.len())
            {
                return Err(
                    "Settings do not match this image or contain unsupported values.".into(),
                );
            }
            Ok(Some(document.settings))
        })();
        match result {
            Ok(Some(settings)) => settings,
            Ok(None) => channels::defaults(&slide.channels),
            Err(e) => {
                crate::diagnostics::record(crate::diagnostics::Event::SettingsLoadFailed);
                self.blocked = true;
                self.message = Some(format!("Channel settings not loaded: {e} Existing file preserved; changes are session-only."));
                channels::defaults(&slide.channels)
            }
        }
    }

    pub fn mark_changed(&mut self) {
        self.pending = true;
    }

    pub fn flush(&mut self, slide: &Slide, settings: &[channels::Setting]) {
        if !self.pending || self.blocked {
            return;
        }
        self.pending = false;
        let result = (|| -> Result<(), String> {
            if !valid(settings, slide.channels.len()) {
                return Err("Invalid channel settings.".into());
            }
            let doc = Document {
                version: 1,
                image: Identity::of(slide),
                settings: settings.to_vec(),
            };
            let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err("Settings exceed 64 KiB.".into());
            }
            let target = path(slide);
            let mut temporary = target.as_os_str().to_os_string();
            temporary.push(format!(".{}.tmp", std::process::id()));
            let temporary = PathBuf::from(temporary);
            // Exclusive creation avoids damaging another pending write.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            let write_result = (|| -> std::io::Result<()> {
                file.write_all(&bytes)?;
                file.sync_all()?;
                drop(file);
                std::fs::rename(&temporary, &target)
            })();
            if write_result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            write_result.map_err(|e| e.to_string())
        })();
        if result.is_err() {
            crate::diagnostics::record(crate::diagnostics::Event::SettingsSaveFailed);
        }
        self.message = result
            .err()
            .map(|e| format!("Channel settings not saved: {e}"));
    }
}
