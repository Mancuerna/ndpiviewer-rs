//! Versioned binary navigation sidecar; independent of NDPA annotations.
use super::{MAX_PLANES, SIDE};
use crate::Slide;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"NDPITRK1";
const MAX_ID: usize = 128 * 1024;
const MAX_BYTES: usize = MAX_ID + MAX_PLANES * (SIDE * SIDE + 4) + 64;

#[derive(Default)]
pub(super) struct Storage {
    target: Option<PathBuf>,
    identity: Vec<u8>,
    planes: u32,
    previous: Option<u64>,
    blocked: bool,
    pub message: Option<String>,
}

fn path(slide: &Slide) -> PathBuf {
    if slide.path.is_dir() {
        slide.path.join("navigation.ndpitrack")
    } else {
        let mut name = slide.path.as_os_str().to_os_string();
        name.push(".ndpitrack");
        PathBuf::from(name)
    }
}

fn identity(slide: &Slide) -> Result<Vec<u8>, String> {
    let mut id = format!(
        "{}:{}:{}:{:?}:{:?}\n",
        slide.width, slide.height, slide.planes, slide.calibration, slide.registration
    );
    for channel in &slide.channels {
        id.push_str(&format!("{:?}\n", channel.name));
    }
    let files = if slide.path.is_dir() {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&slide.path).map_err(|e| e.to_string())? {
            let file = entry.map_err(|e| e.to_string())?.path();
            if file.extension().is_some_and(|ext| {
                ["jpg", "jpeg", "png"]
                    .iter()
                    .any(|s| ext.eq_ignore_ascii_case(s))
            }) {
                files.push(file);
            }
        }
        files.sort();
        files
    } else {
        let mut files = vec![slide.path.clone()];
        if slide
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ndpis"))
        {
            files.extend(crate::ndpis::files(&slide.path)?);
        }
        files
    };
    for file in files {
        let meta = std::fs::metadata(&file).map_err(|e| e.to_string())?;
        let modified = meta
            .modified()
            .map_err(|e| e.to_string())?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        id.push_str(&format!(
            "{:?}:{}:{modified}\n",
            file.file_name(),
            meta.len()
        ));
        if id.len() > MAX_ID {
            return Err("Tracking image identity exceeds its size limit.".into());
        }
    }
    if id.len() > MAX_ID {
        return Err("Tracking image identity exceeds its size limit.".into());
    }
    Ok(id.into_bytes())
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Tracking file exceeds its size limit.".into());
    }
    Ok(Some(bytes))
}

impl Storage {
    pub fn writable(&self) -> bool {
        self.target.is_some() && !self.blocked
    }

    pub fn load(&mut self, slide: &Slide) -> BTreeMap<u32, Vec<u8>> {
        *self = Self {
            target: Some(path(slide)),
            planes: slide.planes,
            ..Self::default()
        };
        let result = (|| {
            self.identity = identity(slide)?;
            let Some(bytes) = read(self.target.as_ref().unwrap())? else {
                return Ok(BTreeMap::new());
            };
            let grids = self.decode(&bytes)?;
            self.previous = Some(checksum(&bytes));
            Ok::<_, String>(grids)
        })();
        match result {
            Ok(grids) => grids,
            Err(e) => {
                crate::diagnostics::record(crate::diagnostics::Event::TrackingLoadFailed);
                self.blocked = true;
                self.message = Some(format!("Tracking not loaded: {e} Existing file preserved; new tracking is session-only."));
                BTreeMap::new()
            }
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<BTreeMap<u32, Vec<u8>>, String> {
        if bytes.len() < 24 || &bytes[..8] != MAGIC {
            return Err("Unsupported tracking format.".into());
        }
        let end = bytes.len() - 8;
        if checksum(&bytes[..end]) != u64::from_le_bytes(bytes[end..].try_into().unwrap()) {
            return Err("Tracking checksum mismatch.".into());
        }
        let id_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if id_len > MAX_ID || 16 + id_len > end || bytes[12..12 + id_len] != self.identity {
            return Err("Tracking does not match this image.".into());
        }
        let count =
            u32::from_le_bytes(bytes[12 + id_len..16 + id_len].try_into().unwrap()) as usize;
        if count > MAX_PLANES || 16 + id_len + count * (4 + SIDE * SIDE) != end {
            return Err("Invalid tracking grid count or size.".into());
        }
        let mut grids = BTreeMap::new();
        for chunk in bytes[16 + id_len..end].as_chunks::<{ 4 + SIDE * SIDE }>().0 {
            let z = u32::from_le_bytes(chunk[..4].try_into().unwrap());
            if z >= self.planes || grids.contains_key(&z) || chunk[4..].iter().any(|&v| v > 4) {
                return Err("Invalid tracking plane or zoom level.".into());
            }
            grids.insert(z, chunk[4..].to_vec());
        }
        Ok(grids)
    }

    pub fn save(&mut self, grids: &BTreeMap<u32, Vec<u8>>) -> bool {
        if self.blocked || self.target.is_none() {
            return false;
        }
        let result = (|| {
            if grids.len() > MAX_PLANES
                || grids.iter().any(|(&z, grid)| {
                    z >= self.planes || grid.len() != SIDE * SIDE || grid.iter().any(|&v| v > 4)
                })
            {
                return Err("Invalid tracking data.".to_string());
            }
            let target = self.target.as_ref().unwrap();
            if read(target)?.as_deref().map(checksum) != self.previous {
                self.blocked = true;
                return Err("The tracking file changed in another application. Reopen the image before saving.".into());
            }
            let mut bytes = MAGIC.to_vec();
            bytes.extend_from_slice(&(self.identity.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&self.identity);
            bytes.extend_from_slice(&(grids.len() as u32).to_le_bytes());
            for (&z, grid) in grids {
                bytes.extend_from_slice(&z.to_le_bytes());
                bytes.extend_from_slice(grid);
            }
            bytes.extend_from_slice(&checksum(&bytes).to_le_bytes());
            let mut temporary = target.as_os_str().to_os_string();
            temporary.push(format!(".{}.tmp", std::process::id()));
            let temporary = PathBuf::from(temporary);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            let result = file.write_all(&bytes).and_then(|_| file.sync_all());
            drop(file);
            let result = result.and_then(|_| std::fs::rename(&temporary, target));
            if result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            result.map_err(|e| e.to_string())?;
            self.previous = Some(checksum(&bytes));
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = None;
                true
            }
            Err(e) => {
                crate::diagnostics::record(crate::diagnostics::Event::TrackingSaveFailed);
                self.message = Some(format!("Tracking not saved: {e}"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, Slide) {
        let root = std::env::temp_dir().join(format!("ndpitrack-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut slide = crate::ndpa::test_slide();
        slide.path = root.join("sample.ndpi");
        std::fs::write(&slide.path, b"source fixture").unwrap();
        (root, slide)
    }

    fn cleanup(root: PathBuf, slide: &Slide) {
        let _ = std::fs::remove_file(path(slide));
        std::fs::remove_file(&slide.path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn restores_each_z_and_zoom_and_persists_clear() {
        let (root, slide) = fixture("roundtrip");
        let mut storage = Storage::default();
        assert!(storage.load(&slide).is_empty());
        let grids = BTreeMap::from([(0, vec![1; SIDE * SIDE]), (2, vec![4; SIDE * SIDE])]);
        assert!(storage.save(&grids));
        let mut reopened = Storage::default();
        assert_eq!(reopened.load(&slide), grids);
        assert!(reopened.save(&BTreeMap::new()));
        assert!(Storage::default().load(&slide).is_empty());
        cleanup(root, &slide);
    }

    #[test]
    fn preserves_corrupt_mismatched_and_externally_changed_files() {
        let (root, slide) = fixture("protection");
        let grids = BTreeMap::from([(0, vec![1; SIDE * SIDE])]);
        let mut storage = Storage::default();
        storage.load(&slide);
        assert!(storage.save(&grids));
        let original = std::fs::read(path(&slide)).unwrap();
        let mut changed_slide = slide.clone();
        changed_slide.width += 1;
        let mut mismatch = Storage::default();
        assert!(mismatch.load(&changed_slide).is_empty());
        assert!(!mismatch.save(&grids));
        assert_eq!(std::fs::read(path(&slide)).unwrap(), original);
        let mut corrupt = original.clone();
        corrupt[20] ^= 1;
        std::fs::write(path(&slide), &corrupt).unwrap();
        let mut bad = Storage::default();
        assert!(bad.load(&slide).is_empty());
        assert!(!bad.save(&grids));
        assert!(!storage.save(&grids)); // Changed after original load.
        assert_eq!(std::fs::read(path(&slide)).unwrap(), corrupt);
        std::fs::write(path(&slide), &original).unwrap();
        std::fs::write(&slide.path, b"replacement image of different size").unwrap();
        let mut replaced = Storage::default();
        assert!(replaced.load(&slide).is_empty());
        assert!(replaced.message.is_some());
        assert!(!replaced.save(&grids));
        cleanup(root, &slide);
    }

    #[test]
    fn failed_write_can_retry_without_clobbering_a_temporary_file() {
        let (root, slide) = fixture("retry");
        let mut storage = Storage::default();
        storage.load(&slide);
        let grids = BTreeMap::from([(1, vec![3; SIDE * SIDE])]);
        let mut temporary = path(&slide).as_os_str().to_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, b"owned by another write").unwrap();
        assert!(!storage.save(&grids));
        assert_eq!(
            std::fs::read(&temporary).unwrap(),
            b"owned by another write"
        );
        std::fs::remove_file(temporary).unwrap();
        assert!(storage.save(&grids));
        assert_eq!(Storage::default().load(&slide), grids);
        cleanup(root, &slide);
    }

    #[test]
    fn folder_identity_ignores_sidecar_writes_and_images_stay_independent() {
        let root = std::env::temp_dir().join(format!("ndpitrack-folder-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("0.jpg"), b"image").unwrap();
        let mut slide = crate::ndpa::test_slide();
        slide.path = root.clone();
        let mut storage = Storage::default();
        storage.load(&slide);
        let grids = BTreeMap::from([(0, vec![2; SIDE * SIDE])]);
        assert!(storage.save(&grids));
        assert_eq!(Storage::default().load(&slide), grids);
        let separate = root.join("other.ndpi");
        std::fs::write(&separate, b"other image").unwrap();
        slide.path = separate.clone();
        assert!(Storage::default().load(&slide).is_empty());
        std::fs::remove_file(separate).unwrap();
        std::fs::remove_file(root.join("0.jpg")).unwrap();
        std::fs::remove_file(root.join("navigation.ndpitrack")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

// Stable FNV-1a checksum for accidental corruption and concurrent sidecar edits.
