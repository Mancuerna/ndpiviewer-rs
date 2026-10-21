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
