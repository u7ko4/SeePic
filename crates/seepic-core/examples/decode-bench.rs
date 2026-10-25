//! Times the decode path over a folder of real images.
//!
//! Run against the folder you actually browse:
//!
//! ```text
//! cargo run --release -p seepic-core --example decode-bench -- "C:\Users\mx\Pictures\Wallpaper" 1600
//! ```
//!
//! The argument is a longest-side target, matching what the viewer asks for
//! when it fits an image to its window. Each file is decoded twice: once by the
//! current path, and once by the sequence it replaced — full decode, expand to
//! RGBA, swap to BGRA, copy, swap back, resample, swap again. Both produce the
//! same output size, so the difference between the columns is the work that was
//! removed rather than a different amount of output.
//!
//! Read and decode are timed separately: on a warm file cache the read is
//! noise, so the decode figure is the one to compare.

use std::path::PathBuf;
use std::time::Instant;

use image::{DynamicImage, RgbaImage};

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = PathBuf::from(args.next().unwrap_or_else(|| ".".to_string()));
    let target: u32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1600);

    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .expect("read the folder")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    files.sort();

    println!("{} files in {}", files.len(), folder.display());
    println!("decode target: {target}px longest side\n");
    println!(
        "{:>30}  {:>10}  {:>10}  {:>8}  {:>9}  {:>9}",
        "file", "decoded", "of full", "speedup", "was", "now"
    );

    let mut was_total = 0f64;
    let mut now_total = 0f64;
    let mut pixels = 0u64;
    let mut full_pixels = 0u64;
    let mut decoded_count = 0f64;
    let mut failures = 0usize;

    for path in &files {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();

        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };

        let started = Instant::now();
        let Ok(image) = seepic_core::decode::decode_for_display(&bytes, true, Some(target)) else {
            failures += 1;
            println!("{:>30}  failed", shorten(&name, 30));
            continue;
        };
        let now_ms = started.elapsed().as_secs_f64() * 1000.;

        // The same output size, produced the way the viewer used to do it.
        let Some(was_ms) = old_path_ms(&bytes, image.width, image.height) else {
            failures += 1;
            continue;
        };

        let (source_w, source_h) = image.source_size();
        let decoded_px = (image.width as u64) * (image.height as u64);
        let full_px = (source_w as u64) * (source_h as u64);

        decoded_count += 1.;
        was_total += was_ms;
        now_total += now_ms;
        pixels += decoded_px;
        full_pixels += full_px;

        println!(
            "{:>30}  {:>5}x{:<4}  {:>8.0}%  {:>7.2}x  {was_ms:>7.1}ms  {now_ms:>7.1}ms",
            shorten(&name, 30),
            image.width,
            image.height,
            100. * decoded_px as f64 / full_px.max(1) as f64,
            was_ms / now_ms,
        );
    }

    let count = decoded_count.max(1.);
    println!();
    println!("files decoded     : {} ({} failed)", decoded_count as usize, failures);
    println!("mean, old path    : {:>7.1} ms", was_total / count);
    println!("mean, new path    : {:>7.1} ms", now_total / count);
    println!("speedup           : {:>7.2}x", was_total / now_total.max(1.));
    println!(
        "pixels decoded    : {:.0} M of {:.0} M full ({:.0}%)",
        pixels as f64 / 1e6,
        full_pixels as f64 / 1e6,
        100. * pixels as f64 / full_pixels.max(1) as f64
    );
    println!(
        "mean pixel buffer : {:.1} MB (full resolution would be {:.1} MB)",
        (pixels as f64 / count) * 4. / 1e6,
        (full_pixels as f64 / count) * 4. / 1e6
    );
}

/// Decode the way the viewer used to: everything at full size, then reduce.
///
/// Kept here rather than in the library so the comparison stays reproducible
/// without carrying the old code along.
fn old_path_ms(bytes: &[u8], out_width: u32, out_height: u32) -> Option<f64> {
    let started = Instant::now();
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .or_else(|_| image::load_from_memory(bytes))
        .ok()?;
    let (width, height) = (image.width(), image.height());

    // Expand to RGBA, then swap to BGRA: two full-size passes.
    let rgba = DynamicImage::from(image).into_rgba8();
    let bgra = seepic_core::decode::rgba_to_bgra(rgba.into_raw());

    // The old reduction cloned the BGRA buffer, swapped it back, resampled and
    // swapped the result — three more full-size passes.
    let source = RgbaImage::from_raw(width, height, seepic_core::decode::rgba_to_bgra(bgra))?;
    let scaled = image::imageops::resize(&source, out_width, out_height, image::imageops::FilterType::Triangle);
    let _ = seepic_core::decode::rgba_to_bgra(scaled.into_raw());

    Some(started.elapsed().as_secs_f64() * 1000.)
}

fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let tail: String = text
        .chars()
        .skip(text.chars().count() - (width - 1))
        .collect();
    format!("…{tail}")
}
