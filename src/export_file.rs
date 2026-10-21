//! Replace an export only after its encoder and filesystem writes succeed.
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_EXPORT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write(
    destination: &Path,
    encode: impl FnOnce(&mut BufWriter<File>) -> Result<(), String>,
) -> Result<(), String> {
    let mut name = destination.as_os_str().to_os_string();
    name.push(format!(
        ".{}.{}.export.tmp",
        std::process::id(),
        NEXT_EXPORT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = std::path::PathBuf::from(name);
    // Never truncate a stale or independently created temporary file.
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut output = BufWriter::new(file);
        encode(&mut output)?;
        output.flush().map_err(|e| e.to_string())?;
        output.get_ref().sync_all().map_err(|e| e.to_string())?;
        drop(output);
        std::fs::rename(&temporary, destination).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        // This call exclusively created this exact file; no other path is removed.
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_encoder_preserves_existing_export_and_success_replaces_it() {
        let folder = std::env::temp_dir().join(format!("viewer-export-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("report.csv");
        std::fs::write(&path, b"previous report").unwrap();
        let result = write(&path, |output| {
            output.write_all(b"partial result").unwrap();
            Err("Encoder failed".into())
        });
        assert_eq!(result.unwrap_err(), "Encoder failed");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous report");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        write(&path, |output| {
            output
                .write_all(b"complete report")
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete report");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
