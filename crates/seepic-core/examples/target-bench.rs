//! What each decode target costs, per file.
//!
//! The burst path decodes at a quarter of the view; this measures whether that
//! is actually cheaper than decoding at the view's size, which is what decides
//! whether a held key can show sharp pages.
//!
//! ```text
//! cargo run --release -p seepic-core --example target-bench -- "C:\Users\mx\Pictures\Wallpaper"
//! ```

use std::path::PathBuf;
use std::time::Instant;

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

    let targets = [0u32, 512, 768, 1400];
    println!(
        "\n{:>26} {:>10} {:>9} {:>9} {:>9} {:>9}",
        "file", "kind", "full", "512", "768", "1400"
    );
    let mut sums = [0f64; 4];
    let mut sums_prog = [0f64; 4];
    let mut n = 0f64;
    let mut n_prog = 0f64;

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut times = [0f64; 4];
        let mut ok = true;
        for (slot, target) in targets.iter().enumerate() {
            let Some(ms) = fastest(|| {
                let side = if *target == 0 { None } else { Some(*target) };
                seepic_core::decode::decode_for_display(&bytes, true, side)
                    .ok()
                    .map(|_| ())
            }) else {
                ok = false;
                break;
            };
            times[slot] = ms;
        }
        if !ok {
            continue;
        }
        let progressive = is_progressive(&bytes);
        if progressive {
            n_prog += 1.;
            for i in 0..4 {
                sums_prog[i] += times[i];
            }
        } else {
            n += 1.;
            for i in 0..4 {
                sums[i] += times[i];
            }
        }
        println!(
            "{:>26} {:>10} {:>7.1}ms {:>7.1}ms {:>7.1}ms {:>7.1}ms",
            shorten(&name, 26),
            if progressive { "progressive" } else { "baseline" },
            times[0],
            times[1],
            times[2],
            times[3]
        );
    }

    report("baseline", &sums, n);
    report("progressive", &sums_prog, n_prog);
}

fn report(label: &str, sums: &[f64; 4], n: f64) {
    if n == 0. {
        return;
    }
    println!("\n{} ({} files):", label, n);
    println!("  full   {:>7.1} ms", sums[0] / n);
    println!("  512 px {:>7.1} ms  {:.2}x of full", sums[1] / n, sums[1] / sums[0]);
    println!("  768 px {:>7.1} ms  {:.2}x of full", sums[2] / n, sums[2] / sums[0]);
    println!("  1400px {:>7.1} ms  {:.2}x of full", sums[3] / n, sums[3] / sums[0]);
}

fn is_progressive(bytes: &[u8]) -> bool {
    let mut i = 2;
    while i + 3 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            0xC0 | 0xC1 => return false,
            0xC2 => return true,
            0xD8 | 0xD9 | 0xD0..=0xD7 => {
                i += 2;
                continue;
            }
            _ => {
                let length = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
                i += 2 + length;
            }
        }
    }
    false
}

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

fn shorten(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    format!("…{}", chars[chars.len() - width + 1..].iter().collect::<String>())
}
