//! End-to-end checks that each archive backend really decompresses images.
//!
//! The fixtures are genuine archives: the TAR is built here with the `tar`
//! crate, and `archive.cbr` / `archive.cb7` hold the same three coloured PNGs
//! in the two other formats. Each test asserts the bytes that come out
//! decode back to the colour that went in, so a backend that silently returned
//! the wrong member, or corrupt data, would fail here.

use std::io::Write;
use std::path::{Path, PathBuf};

use seepic_core::Container;

/// The colours baked into `page_1`, `page_2` and `page_3`.
const COLORS: [[u8; 3]; 3] = [[220, 40, 40], [40, 200, 60], [50, 60, 220]];

fn png_bytes(color: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(40, 40, image::Rgb(color));
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("iv-arch-test-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a TAR containing the three pages, at `path`.
fn write_tar(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut builder = tar::Builder::new(file);

    for (index, color) in COLORS.iter().enumerate() {
        let bytes = png_bytes(*color);
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("page_{}.png", index + 1),
                std::io::Cursor::new(bytes),
            )
            .unwrap();
    }
    builder.finish().unwrap();
}

/// Read every page out of `container` and assert the colours match.
fn assert_pages(container: &mut Container) {
    assert_eq!(container.len(), 3, "expected three pages");

    for index in 0..3 {
        let bytes = container.read(index).expect("member should decompress");
        let decoded = image::load_from_memory(&bytes).expect("member should decode");
        let pixel = decoded.to_rgb8().get_pixel(20, 20).0;
        assert_eq!(
            pixel, COLORS[index],
            "page {} decoded the wrong image",
            index + 1
        );
    }
}

#[test]
fn reads_a_prefix_of_a_page() {
    let dir = scratch("prefix");
    let path = dir.join("archive.cbt");
    write_tar(&path);

    let mut container = Container::open_archive(&path).unwrap();
    let whole = container.read(0).unwrap();

    let prefix = container.read_prefix(0, 100).unwrap();
    assert_eq!(prefix.bytes, whole[..100], "the front of the member");
    assert!(!prefix.complete, "100 bytes is not the whole page");

    let all = container.read_prefix(0, 1_000_000).unwrap();
    assert!(all.complete);
    assert_eq!(all.bytes, whole, "a limit past the end is the whole page");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reads_a_real_zip_and_its_prefix() {
    let dir = scratch("zip");
    let path = dir.join("pages.cbz");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (index, color) in COLORS.iter().enumerate() {
            writer
                .start_file(format!("page_{}.png", index + 1), options)
                .unwrap();
            writer.write_all(&png_bytes(*color)).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut container = Container::open_archive(&path).expect("zip should open");
    assert_pages(&mut container);

    let whole = container.read(0).unwrap();
    let prefix = container.read_prefix(0, 16).unwrap();
    assert_eq!(prefix.bytes, whole[..16], "the front of the member");
    assert!(!prefix.complete);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reads_a_real_tar() {
    let dir = scratch("tar");
    let path = dir.join("archive.cbt");
    write_tar(&path);

    let mut container = Container::open_archive(&path).expect("tar should open");
    assert_pages(&mut container);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reads_a_real_7z() {
    let path = fixture_dir().join("archive.cb7");
    if !path.exists() {
        eprintln!("skipping: {} is missing", path.display());
        return;
    }

    let mut container = Container::open_archive(&path).expect("7z should open");
    assert_pages(&mut container);
}

#[test]
fn reads_a_real_rar() {
    let path = fixture_dir().join("archive.cbr");
    if !path.exists() {
        eprintln!("skipping: {} is missing", path.display());
        return;
    }

    let mut container = Container::open_archive(&path).expect("rar should open");
    assert_pages(&mut container);
}

#[test]
fn backends_agree_on_the_entry_list() {
    // Name order should be identical regardless of format, which is what
    // natural sorting is responsible for.
    let dir = scratch("agree");
    let tar_path = dir.join("a.cbt");
    write_tar(&tar_path);

    let mut tar_container = Container::open_archive(&tar_path).unwrap();
    let tar_names: Vec<String> = tar_container
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();

    let rar_path = fixture_dir().join("archive.cbr");
    if rar_path.exists() {
        let rar_container = Container::open_archive(&rar_path).unwrap();
        let rar_names: Vec<String> = rar_container.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(tar_names, rar_names, "TAR and RAR disagree on entry names");
    }

    let sz_path = fixture_dir().join("archive.cb7");
    if sz_path.exists() {
        let sz_container = Container::open_archive(&sz_path).unwrap();
        let sz_names: Vec<String> = sz_container.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(tar_names, sz_names, "TAR and 7z disagree on entry names");
    }

    // Keep the scratch dir alive until the containers are dropped.
    let _ = &mut tar_container;
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn writes_a_tar_that_we_can_reopen() {
    // Guards the fixture builder itself: if this drifts, the tests above would
    // silently start checking an empty archive.
    let dir = scratch("roundtrip");
    let path = dir.join("check.cbt");
    write_tar(&path);

    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > 512, "tar should contain data");

    let container = Container::open_archive(&path).unwrap();
    assert_eq!(container.len(), 3);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_archive_without_images_reports_it() {
    let dir = scratch("noimg");
    let path = dir.join("docs.cbt");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut builder = tar::Builder::new(file);
        let bytes = b"not an image";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(&mut header, "readme.txt", std::io::Cursor::new(bytes))
            .unwrap();
        builder.finish().unwrap();
    }

    assert!(Container::open_archive(&path).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

/// Silence an unused-import warning when the `Write` trait is only needed by
/// other tests in this file.
#[allow(dead_code)]
fn _write_is_used(mut sink: impl Write) {
    let _ = sink.write_all(b"");
}
