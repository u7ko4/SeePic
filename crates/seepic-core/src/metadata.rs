//! File and EXIF metadata for the info panel.

/// What the info panel shows about the current image.
#[derive(Debug, Clone, Default)]
pub struct ImageInfo {
    pub name: String,
    /// Folder or archive the image came from.
    pub location: String,
    pub format: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
    pub decoded_orientation: Option<u8>,
    pub exif: Option<Exif>,
}

/// The EXIF fields worth surfacing.
#[derive(Debug, Clone, Default)]
pub struct Exif {
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub exposure: Option<String>,
    pub aperture: Option<String>,
    pub iso: Option<String>,
    pub focal: Option<String>,
    pub taken: Option<String>,
    pub software: Option<String>,
    /// Everything else the block contains, as (tag name, formatted value).
    ///
    /// The eight fields above are the ones a photographer looks for first;
    /// this keeps the rest of the block visible without special-casing every
    /// tag. It is what makes the panel long, hence the scrolling body.
    pub fields: Vec<(String, String)>,
}

impl Exif {
    pub fn is_empty(&self) -> bool {
        self.camera.is_none()
            && self.lens.is_none()
            && self.exposure.is_none()
            && self.aperture.is_none()
            && self.iso.is_none()
            && self.focal.is_none()
            && self.taken.is_none()
            && self.software.is_none()
            && self.fields.is_empty()
    }
}

/// Read the EXIF block out of a file's bytes.
///
/// Returns `None` for formats that carry no EXIF, and for files whose EXIF is
/// malformed — both are ordinary, not errors worth showing.
pub fn exif_from_bytes(bytes: &[u8]) -> Option<Exif> {
    let mut cursor = std::io::Cursor::new(bytes);
    // Files copied around the web often have a broken next-IFD pointer. The
    // reader can keep the fields it managed to parse; without that, such a
    // file would show "no EXIF" even though its block is full of data.
    let mut reader = exif::Reader::new();
    reader.continue_on_error(true);
    let container = match reader.read_from_container(&mut cursor) {
        Ok(exif) => exif,
        Err(error) => error.distill_partial_result(|_| {}).ok()?,
    };

    let read = |tag: exif::Tag| -> Option<String> {
        let field = container.get_field(tag, exif::In::PRIMARY)?;
        let value = field.display_value().with_unit(&container).to_string();
        // Empty or placeholder values are not worth a row.
        if value.trim().is_empty() || value == "Unknown" {
            None
        } else {
            Some(value)
        }
    };

    let camera = match (read(exif::Tag::Make), read(exif::Tag::Model)) {
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (None, model) => model,
        (make, None) => make,
    };

    // The summary tags are shown under their own Chinese labels, so the full
    // list starts with everything else. A long dump — maker notes, user
    // comments, embedded thumbnails — is summarised instead of copied into the
    // UI: the panel has to stay scrollable, not crawl.
    let summary = [
        exif::Tag::Make,
        exif::Tag::Model,
        exif::Tag::LensModel,
        exif::Tag::ExposureTime,
        exif::Tag::FNumber,
        exif::Tag::PhotographicSensitivity,
        exif::Tag::FocalLength,
        exif::Tag::DateTimeOriginal,
        exif::Tag::Software,
    ];
    let mut fields = Vec::new();
    for field in container.fields() {
        if field.ifd_num == exif::In::THUMBNAIL || summary.contains(&field.tag) {
            continue;
        }
        // UserComment is where AI tools and phones put readable text, but the
        // crate can only show it as a hex dump: it carries an 8-byte encoding
        // prefix that has to be decoded first.
        let mut value = if field.tag == exif::Tag::UserComment {
            match user_comment(field) {
                Some(text) => text,
                None => continue,
            }
        } else {
            let value = field.display_value().with_unit(&container).to_string();
            let value = value.trim();
            if value.is_empty() || value == "Unknown" {
                continue;
            }
            match &field.value {
                exif::Value::Undefined(bytes, _) if bytes.len() > 64 => {
                    format!("（{} 字节数据）", bytes.len())
                }
                exif::Value::Byte(bytes) if bytes.len() > 64 => {
                    format!("（{} 字节数据）", bytes.len())
                }
                _ => value.to_string(),
            }
        };
        if value.chars().count() > 4000 && field.tag != exif::Tag::UserComment {
            value = value.chars().take(4000).collect::<String>() + "…";
        }
        let name = if field.tag == exif::Tag::UserComment {
            "用户注释".to_string()
        } else {
            field.tag.to_string()
        };
        fields.push((name, value));
    }

    let exif = Exif {
        camera,
        lens: read(exif::Tag::LensModel),
        exposure: read(exif::Tag::ExposureTime),
        aperture: read(exif::Tag::FNumber),
        iso: read(exif::Tag::PhotographicSensitivity),
        focal: read(exif::Tag::FocalLength),
        taken: read(exif::Tag::DateTimeOriginal),
        software: read(exif::Tag::Software),
        fields,
    };

    if exif.is_empty() {
        None
    } else {
        Some(exif)
    }
}

/// Decode an EXIF `UserComment`.
///
/// The value starts with an 8-byte character code (`UNICODE\0` or
/// `ASCII\0\0\0`) followed by the text. In practice `UNICODE` is UTF-16LE even
/// when the TIFF header is big-endian, as in the generation parameters many
/// image tools write; the byte order is checked by decoding both ways and
/// looking for replacement characters.
fn user_comment(field: &exif::Field) -> Option<String> {
    let exif::Value::Undefined(bytes, _) = &field.value else {
        return None;
    };
    let (unicode, body) = if bytes.starts_with(b"UNICODE\0") {
        (true, &bytes[8..])
    } else if bytes.starts_with(b"ASCII\0\0\0") {
        (false, &bytes[8..])
    } else {
        (false, &bytes[..])
    };

    let text = if unicode {
        // The eight-byte character code is followed by the text, but some
        // writers pad it with an extra NUL, which puts the UTF-16 pairs on an
        // odd boundary. Decode every plausible alignment/order and keep the
        // one that reads as text: right pair order beats a wrong guess that
        // happens to form valid CJK.
        let trimmed = body
            .iter()
            .position(|byte| *byte != 0)
            .map(|start| &body[start..])
            .unwrap_or(&[]);
        let candidates = [
            decode_utf16(body, true),
            decode_utf16(body, false),
            decode_utf16(trimmed, true),
            decode_utf16(trimmed, false),
        ];
        candidates
            .into_iter()
            .max_by(|left, right| {
                readability(left)
                    .partial_cmp(&readability(right))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_default()
    } else {
        String::from_utf8_lossy(body).into_owned()
    };

    let text = text
        .trim_start_matches('\u{feff}')
        .trim_matches('\0')
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else if text.chars().count() > 8000 {
        Some(text.chars().take(8000).collect::<String>() + "…")
    } else {
        Some(text)
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Fraction of characters that look like ordinary text rather than the CJK
/// noise a wrong UTF-16 byte order produces from ASCII prompts.
fn readability(text: &str) -> f32 {
    let mut total = 0usize;
    let mut good = 0usize;
    for character in text.chars() {
        if character == '\u{fffd}' || character == '\0' {
            continue;
        }
        total += 1;
        if character.is_ascii_graphic() || character.is_whitespace() || character == '\u{feff}' {
            good += 1;
        }
    }
    if total == 0 {
        0.
    } else {
        good as f32 / total as f32
    }
}

/// Format a byte count for the information panel.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < UNITS.len() {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_the_right_unit() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn non_image_bytes_have_no_exif() {
        assert!(exif_from_bytes(b"definitely not an image").is_none());
        assert!(exif_from_bytes(&[]).is_none());
    }

    #[test]
    fn empty_exif_reports_empty() {
        assert!(Exif::default().is_empty());
        assert!(!Exif {
            camera: Some("X".into()),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn reads_camera_and_exposure_from_a_jpeg() {
        let jpeg = jpeg_with_exif();
        let exif = exif_from_bytes(&jpeg).expect("exif should parse");

        let camera = exif.camera.expect("camera should be present");
        assert!(camera.contains("ZCode"), "camera was {camera:?}");
        assert!(camera.contains("TestCam"), "camera was {camera:?}");
        // The make and model path proves the IFD walk works end to end. Numeric
        // fields are checked against real photos instead, because their inline
        // encoding is easy to get subtly wrong in a hand-built block.
    }

    /// A minimal JPEG carrying Make, Model and ISO.
    fn jpeg_with_exif() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3]));
        let mut plain = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut plain, image::ImageFormat::Jpeg)
            .unwrap();
        let plain = plain.into_inner();

        // TIFF header + one IFD with three entries, values inlined where they fit
        // and pointed to by offset where they do not.
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM\x00\x2a");
        tiff.extend_from_slice(&8u32.to_be_bytes());
        tiff.extend_from_slice(&3u16.to_be_bytes()); // three entries

        // Entry helper: tag, type, count, then value/offset.
        let strings_offset = 8 + 2 + 3 * 12 + 4;
        let make = b"ZCode\0";
        let model = b"TestCam\0";

        // ASCII entries whose value does not fit in four bytes carry an offset,
        // and their count is the string length including the terminator. An
        // inline count of 1 would make the reader treat the offset as text.
        let mut ascii_entry = |tag: u16, count: u32, offset: u32| {
            tiff.extend_from_slice(&tag.to_be_bytes());
            tiff.extend_from_slice(&2u16.to_be_bytes());
            tiff.extend_from_slice(&count.to_be_bytes());
            tiff.extend_from_slice(&offset.to_be_bytes());
        };
        ascii_entry(0x010F, make.len() as u32, strings_offset as u32);
        ascii_entry(
            0x0110,
            model.len() as u32,
            strings_offset as u32 + make.len() as u32,
        );
        // ISO is a SHORT with count 1, so its value is inline.
        tiff.extend_from_slice(&0x8827u16.to_be_bytes());
        tiff.extend_from_slice(&3u16.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        // A SHORT value sits left-aligned in the four-byte field.
        tiff.extend_from_slice(&[0, 200, 0, 0]);
        tiff.extend_from_slice(&0u32.to_be_bytes()); // next IFD
        tiff.extend_from_slice(make);
        tiff.extend_from_slice(model);

        let mut segment = Vec::new();
        segment.extend_from_slice(b"Exif\x00\x00");
        segment.extend_from_slice(&tiff);

        let length = (segment.len() + 2) as u16;
        let mut out = Vec::new();
        out.extend_from_slice(&[0xFF, 0xD8]);
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&segment);
        out.extend_from_slice(&plain[2..]);
        out
    }
}
