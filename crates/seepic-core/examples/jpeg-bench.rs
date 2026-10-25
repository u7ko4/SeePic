//! Compares the two JPEG decoders on the same files, at forced DCT scales.
//!
//! The viewer can decode a JPEG either through `image`'s backend (`zune-jpeg`,
//! full resolution only) or through `jpeg-decoder`, which can inverse-transform
//! the DCT at 1/2, 1/4 or 1/8 resolution. Reduced resolution decodes far fewer
//! pixels, but the entropy decode still reads the whole file, so the question
//! this answers is where — if anywhere — the saving overtakes the fact that the
//! two decoders' entropy stages are not equally fast.
//!
//! Each measurement is the fastest of several runs, which is the only figure
//! that is stable on a machine that is doing anything else.
//!
//! ```text
//! cargo run --release -p seepic-core --example jpeg-bench -- "C:\Users\mx\Pictures\Wallpaper"
//! ```

use std::path::PathBuf;
use std::time::Instant;

/// Runs per measurement.
const RUNS: usize = 5;

fn main() {
    let folder = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .expect("read the folder")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg")
                })
        })
        .collect();
    files.sort();

    println!("{} JPEGs in {}\n", files.len(), folder.display());
    println!(
        "{:>30} {:>11} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "file", "size", "zune(img)", "zune dir", "jpegdec", "1/2", "1/4"
    );

    let mut totals = [0f64; 4];
    let mut ratios = [0f64; 3];
    let mut ratios_full = [0f64; 1];
    let mut count = 0f64;

    for path in &files {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };

        let zune = fastest(|| {
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
                .ok()
                .map(|_| ())
        });
        let Some(zune_ms) = zune else { continue };
        let Some(direct_ms) = fastest(|| zune_direct(&bytes).map(|_| ())) else {
            continue;
        };

        let Some((source_w, source_h)) = source_size(&bytes) else {
            continue;
        };
        let longest = source_w.max(source_h);

        let full = fastest(|| decode_at(&bytes, longest));
        let half = fastest(|| decode_at(&bytes, longest / 2));
        let quarter = fastest(|| decode_at(&bytes, longest / 4));
        let (Some(full_ms), Some(half_ms), Some(quarter_ms)) = (full, half, quarter) else {
            continue;
        };

        count += 1.;
        totals[0] += zune_ms;
        totals[1] += full_ms;
        totals[2] += half_ms;
        totals[3] += quarter_ms;
        ratios[0] += full_ms / zune_ms;
        ratios[1] += half_ms / zune_ms;
        ratios[2] += quarter_ms / zune_ms;
        ratios_full[0] += direct_ms / zune_ms;

        println!(
            "{:>30} {:>5}x{:<5} {:>8.1}ms {:>8.1}ms {:>8.1}ms {:>8.1}ms {:>8.1}ms",
            shorten(&name, 30),
            source_w,
            source_h,
            zune_ms,
            direct_ms,
            full_ms,
            half_ms,
            quarter_ms
        );
    }

    println!();
    println!("mean zune(img) full: {:>7.1} ms   (baseline)", totals[0] / count);
    println!(
        "mean zune direct   : {:>7.1} ms   {:.2}x",
        totals[0] / count * ratios_full[0] / count,
        ratios_full[0] / count
    );
    println!("mean jpegdec full  : {:>7.1} ms   {:.2}x", totals[1] / count, ratios[0] / count);
    println!("mean jpegdec 1/2   : {:>7.1} ms   {:.2}x", totals[2] / count, ratios[1] / count);
    println!("mean jpegdec 1/4   : {:>7.1} ms   {:.2}x", totals[3] / count, ratios[2] / count);
    println!();
    println!("below 1.00x means the DCT scale beats a full decode by the faster decoder");
}

/// The fastest of [`RUNS`] runs, or `None` if every run failed.
fn fastest<F, T>(mut run: F) -> Option<f64>
where
    F: FnMut() -> Option<T>,
{
    let mut best: Option<f64> = None;
    for _ in 0..RUNS {
        let started = Instant::now();
        let result = run();
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        if result.is_none() {
            return None;
        }
        best = Some(best.map_or(elapsed, |current: f64| current.min(elapsed)));
    }
    best
}

/// `zune-jpeg` called directly, which is the path the viewer's fallback takes:
/// no `image` wrapper, and no colour-space conversion beyond what is asked for.
fn zune_direct(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = zune_jpeg::JpegDecoder::new(zune_core::bytestream::ZCursor::new(bytes));
    decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;
    Some((width as u32, height as u32))
}

fn decode_at(bytes: &[u8], request: u32) -> Option<(u32, u32)> {
    let request = request.min(u16::MAX as u32) as u16;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let (width, height) = decoder.scale(request, request).ok()?;
    decoder.decode().ok()?;
    Some((width as u32, height as u32))
}

fn source_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    Some((info.width as u32, info.height as u32))
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
