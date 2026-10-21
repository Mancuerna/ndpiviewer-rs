use bioformats::ImageReader;
use std::{error::Error, path::PathBuf, time::Instant};

fn number(value: &std::ffi::OsStr) -> Result<u32, Box<dyn Error>> {
    Ok(value.to_str().ok_or("Invalid numeric argument")?.parse()?)
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("Inspect: ndpi-probe <file.ndpi>");
        println!("Region:  ndpi-probe <file.ndpi> <series> <plane> <x> <y> <width> <height>");
        println!("Indices are zero-based. Plane is the reader linear index, not necessarily Z.");
        return Ok(());
    }
    if args.len() != 1 && args.len() != 7 {
        return Err("Use --help to see supported arguments".into());
    }
    let path = PathBuf::from(&args[0]);
    if !path.is_file() {
        return Err("The file does not exist or cannot be accessed".into());
    }
    let opened = Instant::now();
    let mut reader = ImageReader::open(&path)?;
    println!("Open: {:.1} ms", opened.elapsed().as_secs_f64() * 1000.0);
    println!("Series: {}", reader.series_count());
    for series in 0..reader.series_count() {
        reader.set_series(series)?;
        let m = reader.metadata();
        println!(
            "Series {series}: {}x{}; Z={} C={} T={}; planes={}; {:?}; order={:?}; RGB={}; interleaved={}; resolutions={}",
            m.size_x, m.size_y, m.size_z, m.size_c, m.size_t,
            m.image_count, m.pixel_type, m.dimension_order, m.is_rgb,
            m.is_interleaved, reader.resolution_count()
        );
    }
    if args.len() == 7 {
        let series = number(&args[1])? as usize;
        let plane = number(&args[2])?;
        let x = number(&args[3])?;
        let y = number(&args[4])?;
        let w = number(&args[5])?;
        let h = number(&args[6])?;
        if series >= reader.series_count() {
            return Err("Series index is out of range".into());
        }
        reader.set_series(series)?;
        let m = reader.metadata();
        if plane >= m.image_count || w == 0 || h == 0 || w > 1024 || h > 1024 {
            return Err(
                "Invalid plane or region dimensions outside the 1..1024 limit per side".into(),
            );
        }
        if x.checked_add(w).is_none_or(|end| end > m.size_x)
            || y.checked_add(h).is_none_or(|end| end > m.size_y)
        {
            return Err("The requested region is outside the image bounds".into());
        }
        // A small region bounds our request, not the decoder internal allocations.
        // Measure process memory using real NDPI files.
        let started = Instant::now();
        let bytes = reader.open_bytes_region(plane, x, y, w, h)?;
        println!(
            "Region {w}x{h}: {} bytes; {:.1} ms",
            bytes.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        // Simple reproducible non-cryptographic summary. Does not print pixels.
        let checksum = bytes
            .iter()
            .fold(0u64, |sum, &b| sum.wrapping_add(b as u64));
        println!("Byte sum (does not validate image fidelity): {checksum}");
    }
    reader.close()?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
