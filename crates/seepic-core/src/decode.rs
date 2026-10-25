//! Turning raw file or archive bytes into pixels the GPU can draw.
//!
//! Two details here are easy to get wrong and expensive to debug:
//!
//! * GPUI's `RenderImage` stores **BGRA**, and its constructor does no channel
//!   conversion at all — its own loader swaps bytes 0 and 2 of every pixel. We
//!   do the same, so the swap happens exactly once.
//! * Cameras record rotation in EXIF rather than rotating the pixels, so a photo
//!   shot in portrait arrives as a landscape image plus an orientation tag. We
//!   apply that tag here, once, rather than compensating at draw time.

use std::sync::Arc;

use crate::filters::Interpolation;
use crate::resample;
use image::metadata::Orientation;
use image::{DynamicImage, RgbaImage};

/// Why an image could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("不支持的文件格式")]
    Unsupported,
    #[error("文件已损坏: {0}")]
    Corrupt(String),
    #[error("解码失败: {0}")]
    Failed(String),
}

/// Where a frame's pixels live.
///
/// Decoding always produces heap pixels — the decoders write into a buffer they
/// own — and this is what decides where they end up afterwards. The heap is the
/// default; the swap store moves them into a mapped temporary file, which turns
/// private pages into file pages the OS can reclaim (see [`crate::swap`]).
#[derive(Debug)]
pub enum Pixels {
    /// Private pages of this process.
    Heap(Vec<u8>),
    /// A mapped slot file, owned for as long as the frame is.
    Mapped(Box<crate::swap::SwapFrame>),
}

impl Pixels {
    /// The pixels.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Pixels::Heap(bytes) => bytes,
            Pixels::Mapped(frame) => frame.as_slice(),
        }
    }

    /// How many bytes of pixel data this holds.
    pub fn len(&self) -> usize {
        match self {
            Pixels::Heap(bytes) => bytes.len(),
            Pixels::Mapped(frame) => frame.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// An owned copy, for the paths that must mutate the pixels.
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    /// Whether these pixels are in a mapped file rather than on the heap.
    pub fn is_mapped(&self) -> bool {
        matches!(self, Pixels::Mapped(_))
    }
}

impl std::ops::Deref for Pixels {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for Pixels {
    fn from(bytes: Vec<u8>) -> Self {
        Pixels::Heap(bytes)
    }
}

/// Copying a frame that lives in a mapped file produces heap pixels: a second
/// mapping of the same slot would be a second frame, not a copy.
impl Clone for Pixels {
    fn clone(&self) -> Self {
        Pixels::Heap(self.to_vec())
    }
}

/// One frame of an image, in the layout GPUI wants.
///
/// The pixels are behind an `Arc` so an animation stream can hand the viewer a
/// snapshot of the frames decoded so far without copying any of them: adding a
/// frame costs one `Vec` clone of the small frame descriptors, not a copy of
/// every pixel decoded so far.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Tightly packed BGRA8 pixels, `width * height * 4` bytes.
    pub bgra: Arc<Pixels>,
    /// How long to show this frame, for animated images.
    pub delay_ms: u32,
}

/// A decoded image, possibly animated.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<DecodedFrame>,
    /// True when there is more than one frame.
    pub animated: bool,
    /// The EXIF orientation value that was applied, when one was found.
    pub exif_orientation: Option<u8>,
    /// The file's own dimensions, when this is a display-size reduction.
    ///
    /// `None` means `width`/`height` are the file's. The info panel and the
    /// zoom maths must use the file's figure, not the reduced one, or a
    /// 12-megapixel photo would claim to be 2048 pixels wide.
    pub reduced_from: Option<(u32, u32)>,
}

impl DecodedImage {
    /// Move every frame's pixels into the swap store, when it will take them.
    ///
    /// This is the one copy that takes the pixels off the private heap: the
    /// store's slot is written and the heap buffer is dropped. Frames the store
    /// will not take — too small to be worth a file, or past its ceiling — stay
    /// where they are. Returns how many frames moved.
    pub fn move_to_swap(&mut self, store: &crate::swap::FrameStore) -> usize {
        let mut moved = 0;
        for frame in &mut self.frames {
            let Pixels::Heap(bytes) = &*frame.bgra else {
                continue;
            };
            let Some(mut slot) = store.acquire(bytes.len()) else {
                continue;
            };
            let Some(destination) = slot.as_mut() else {
                continue;
            };
            destination.copy_from_slice(bytes);
            frame.bgra = Arc::new(Pixels::Mapped(Box::new(slot)));
            moved += 1;
        }
        moved
    }

    /// The file's own dimensions, whatever this decode was reduced to.
    pub fn source_size(&self) -> (u32, u32) {
        self.reduced_from.unwrap_or((self.width, self.height))
    }

    /// The longest side this decode was reduced to, or `None` when it holds
    /// the file's full resolution.
    ///
    /// This is the figure the cache and the zoom check compare against. It must
    /// report the *actual* reduction rather than what the decoder was asked
    /// for: an image that turned out small enough to keep in full has all its
    /// detail, and claiming otherwise would make the view ask for an upgrade
    /// that changes nothing, every frame, forever.
    pub fn detail(&self) -> Option<u32> {
        self.reduced_from.map(|_| self.width.max(self.height))
    }
}

impl DecodedImage {
    /// Total pixel dimensions, for the fit and zoom maths.
    pub fn size(&self) -> crate::geom::Size {
        crate::geom::Size::new(self.width as f32, self.height as f32)
    }

    /// The average colour of the first frame as `0xRRGGBB`.
    ///
    /// Used by the Auto background, which only has to land in the right
    /// neighbourhood, so pixels are sampled rather than all of them: this runs
    /// once per image and a 4000x3000 photo has twelve million pixels.
    pub fn average_color(&self) -> u32 {
        let Some(frame) = self.frames.first() else {
            return 0;
        };
        average_color(self.width, self.height, &frame.bgra)
    }

    /// A small copy of the first frame, for use as a blurred backdrop.
    ///
    /// Deliberately tiny: the backdrop is drawn stretched over the whole canvas,
    /// and it is the GPU's bilinear magnification of a 32-pixel image that
    /// reads as a blur. No blur kernel is needed.
    ///
    /// The reduction reads the BGRA buffer directly. Going through
    /// `image::imageops::resize` would first have to clone and channel-swap the
    /// whole frame, which for a 12-megapixel photo is a fifty-megabyte copy to
    /// produce a four-kilobyte result — and this runs on the UI thread.
    pub fn blur_source(&self, longest_side: u32) -> Option<(u32, u32, Vec<u8>)> {
        let frame = self.frames.first()?;
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let scale = (longest_side as f32 / self.width.max(self.height) as f32).min(1.);
        let width = ((self.width as f32 * scale).round() as u32).max(1);
        let height = ((self.height as f32 * scale).round() as u32).max(1);

        box_downsample_bgra(self.width, self.height, &frame.bgra, width, height)
    }
}

/// Reduce a BGRA buffer by averaging blocks of source pixels.
///
/// The block for each output pixel is sampled at a stride rather than in full:
/// at a fifty-to-one reduction a full average would read every source pixel to
/// produce a handful of bytes, and the result is a backdrop that is magnified
/// until it is a smear anyway.
fn box_downsample_bgra(
    width: u32,
    height: u32,
    bytes: &[u8],
    out_width: u32,
    out_height: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    if bytes.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut out = vec![0u8; (out_width as usize) * (out_height as usize) * 4];

    for out_y in 0..out_height {
        let top = (out_y as u64 * height as u64 / out_height as u64) as u32;
        let bottom = (((out_y as u64 + 1) * height as u64) / out_height as u64).max(top as u64 + 1) as u32;
        // Four samples per axis across the block is enough for an average, and
        // keeps the read count proportional to the output rather than the input.
        let step_y = ((bottom - top) / 4).max(1);

        for out_x in 0..out_width {
            let left = (out_x as u64 * width as u64 / out_width as u64) as u32;
            let right = (((out_x as u64 + 1) * width as u64) / out_width as u64).max(left as u64 + 1) as u32;
            let step_x = ((right - left) / 4).max(1);

            let (mut b, mut g, mut r, mut a, mut count) = (0u32, 0u32, 0u32, 0u32, 0u32);
            let mut y = top;
            while y < bottom {
                let mut x = left;
                while x < right {
                    let at = ((y as usize) * (width as usize) + x as usize) * 4;
                    b += bytes[at] as u32;
                    g += bytes[at + 1] as u32;
                    r += bytes[at + 2] as u32;
                    a += bytes[at + 3] as u32;
                    count += 1;
                    x += step_x;
                }
                y += step_y;
            }

            let at = ((out_y as usize) * (out_width as usize) + out_x as usize) * 4;
            let count = count.max(1);
            out[at] = (b / count) as u8;
            out[at + 1] = (g / count) as u8;
            out[at + 2] = (r / count) as u8;
            out[at + 3] = (a / count) as u8;
        }
    }

    Some((out_width, out_height, out))
}

/// Average colour of a BGRA8 buffer, weighted by alpha so the transparent
/// margin of a sticker does not drag the result toward black.
pub fn average_color(width: u32, height: u32, bgra: &[u8]) -> u32 {
    let pixels = (width as usize) * (height as usize);
    if pixels == 0 || bgra.len() < pixels * 4 {
        return 0;
    }
    // Around 4096 samples, whatever the image size.
    let step = (pixels / 4096).max(1);

    let (mut r, mut g, mut b, mut weight) = (0u64, 0u64, 0u64, 0u64);
    let mut index = 0usize;
    while index < pixels {
        let at = index * 4;
        let alpha = bgra[at + 3] as u64;
        // A fully transparent pixel carries no colour to average.
        if alpha > 0 {
            b += bgra[at] as u64 * alpha;
            g += bgra[at + 1] as u64 * alpha;
            r += bgra[at + 2] as u64 * alpha;
            weight += alpha;
        }
        index += step;
    }
    if weight == 0 {
        return 0;
    }
    let r = (r / weight) as u32;
    let g = (g / weight) as u32;
    let b = (b / weight) as u32;
    (r << 16) | (g << 8) | b
}

/// Decode `bytes` into frames ready for the GPU.
///
/// `auto_rotate` applies the EXIF orientation tag, which is the behaviour the
/// "rotate images automatically from EXIF" setting controls.
pub fn decode(bytes: &[u8], auto_rotate: bool) -> Result<DecodedImage, DecodeError> {
    decode_for_display(bytes, auto_rotate, None)
}

/// How much slack to allow before bothering to reduce an image.
///
/// Reducing is not free, so an image only a little larger than the target is
/// left alone; the threshold also keeps a small window from turning a 1000px
/// image into a 900px one for no gain.
const REDUCE_SLACK: u32 = 5;
const REDUCE_SLACK_DIVISOR: u32 = 4;

/// Decode an image, keeping only as much detail as the view can show.
///
/// `target_side` is the longest side of the drawn image in physical pixels, or
/// `None` for the full resolution. A 12-megapixel photo shown in a 1000-pixel
/// window never needs its full pixel count: the extra detail costs a memcpy
/// into the GPU texture and, more importantly, cache budget — the LRU holds
/// whole buffers, so a reduced one keeps many more pages resident and paging
/// back and forth stops re-decoding.
///
/// JPEGs take a different route when the view is much smaller than the file:
/// their DCT coefficients can be inverse transformed at reduced resolution, so
/// the image is decoded straight to roughly the size the view needs and the
/// full-resolution pixels never exist at all. Everything else — and every JPEG
/// the scaled route would not actually speed up — is decoded in full and
/// reduced here.
///
/// Animations are the exception: they are never reduced. The view asks for the
/// full size back as soon as it knows an image is animated (every frame would
/// otherwise be resampled and then thrown away by a second, full-size decode),
/// and the streaming WebP path decodes at the file's own size for the same
/// reason.
pub fn decode_for_display(
    bytes: &[u8],
    auto_rotate: bool,
    target_side: Option<u32>,
) -> Result<DecodedImage, DecodeError> {
    let format = image::guess_format(bytes)
        .map_err(|_| DecodeError::Unsupported)?;

    if format == image::ImageFormat::Gif {
        if let Some(animated) = decode_gif(bytes) {
            // Animations keep their full resolution. Reducing them costs a
            // resample of every frame, and the view asks the full size back as
            // soon as it knows the image is animated — so a reduced animation
            // would be decoded twice for nothing.
            return Ok(animated);
        }
        // Fall through to the static path if the GIF had no usable frames.
    }

    if format == image::ImageFormat::WebP {
        if let Some(animated) = decode_webp_animation(bytes) {
            // Same rule as GIF: animations are not reduced.
            return Ok(animated);
        }
        // A static WebP takes the general path below, where its pixels keep the
        // decoder's three channels instead of being widened to RGBA.
    }

    let orientation = auto_rotate.then(|| read_exif_orientation(bytes)).flatten();

    if format == image::ImageFormat::Jpeg {
        if let Some(side) = target_side {
            if let Some(decoded) = decode_jpeg_scaled(bytes, side, orientation) {
                return Ok(decoded);
            }
            // Progressive files cannot be scaled by `jpeg-decoder`, and a
            // folder of photographs has plenty of them. The second decoder
            // reads them and reduces from its RGB output, which keeps the
            // peak at the decoded picture instead of the four full-size
            // copies the general path would make on the way through.
            if let Some(decoded) = decode_jpeg_progressive(bytes, side, orientation) {
                return Ok(decoded);
            }
            // Anything else the scaled path cannot read — CMYK, an unusual
            // subsampling — falls through to the general decoder.
        }
    }

    let mut image = image::load_from_memory_with_format(bytes, format)
        .map_err(|e| DecodeError::Corrupt(e.to_string()))?;

    if let Some(value) = orientation {
        if let Some(orientation) = Orientation::from_exif(value) {
            image.apply_orientation(orientation);
        }
    }

    Ok(finish(image, orientation, target_side))
}

/// The most a decode may exceed the size the view asked for.
///
/// A little extra detail is what keeps a modest zoom-in from forcing a fresh
/// decode, and — because the cache serves any request up to the detail it
/// holds — it also absorbs the frame-to-frame jitter in the measured draw size.
/// Without it the target moves by a pixel between frames and every page turn
/// misses the cache and re-decodes.
///
/// It is deliberately modest: the pixels are paid for in the cache budget, in
/// the texture upload, and in the high-water mark the GPU allocator keeps after
/// the texture is released, so a generous figure buys nothing but memory.
/// Measured over a folder of 4000-6000px photographs, 1.25 keeps the same cache
/// hit rate as 1.4 while keeping noticeably less resident.
const DECODE_HEADROOM: f32 = 1.25;

/// How far the DCT ladder must step down before it is worth taking.
///
/// At 1/2 the two decoders' entropy stages cost about the same, so a scaled
/// decode is only break-even; from 1/4 down it is both faster and a quarter of
/// the pixels, which also keeps the transient buffer small.
const DCT_MIN_REDUCTION: u32 = 2;

/// How far the DCT ladder steps down for a request, as a factor of 1, 2, 4 or 8.
///
/// Mirrors the decoder's own choice: it takes the most aggressive IDCT whose
/// output still reaches the request, so the answer is the first factor that
/// leaves the image at least as large as the view asked for.
fn jpeg_reduction(source_longest: u32, target_side: u32) -> u32 {
    for idct in [1u32, 2, 4] {
        if (source_longest * idct - 1) / 8 + 1 >= target_side {
            return 8 / idct;
        }
    }
    1
}

/// Decode a JPEG with a reduced IDCT, straight to roughly the needed size.
///
/// A JPEG's DCT coefficients can be inverse-transformed at 1/2, 1/4 or 1/8
/// resolution, so a 24-megapixel photo bound for a small window is decoded at a
/// fraction of its pixels. The entropy decode still reads the whole file, so
/// this only pays when the ladder steps down far enough — see
/// [`DCT_MIN_REDUCTION`].
///
/// Progressive files scale the same way: the same coefficients come out of the
/// entropy stage, only the inverse transform is smaller. They used to be
/// declined here on the grounds that the general decoder is quicker to write
/// for, which was wrong in the way that costs the most — it materialises every
/// pixel of the file, and a 20-megapixel progressive photo was the largest
/// single allocation the viewer made.
///
/// Returns `None` when the file is not worth decoding this way, so the caller
/// can fall back to the general decoder.
fn decode_jpeg_scaled(bytes: &[u8], target_side: u32, orientation: Option<u8>) -> Option<DecodedImage> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    // Read the headers first: the decoder reports the *output* size, so the
    // file's own dimensions are only visible before the scale is chosen.
    decoder.read_info().ok()?;
    let header = decoder.info()?;
    let source = (header.width as u32, header.height as u32);

    if jpeg_reduction(source.0.max(source.1), target_side) < DCT_MIN_REDUCTION {
        return None;
    }

    let channels = match header.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        // CMYK needs a colour transform the decoder does not perform, and
        // 16-bit samples are not the 8-bit layout the frames carry.
        jpeg_decoder::PixelFormat::CMYK32 | jpeg_decoder::PixelFormat::L16 => return None,
    };

    let requested = target_side.min(u16::MAX as u32) as u16;
    let (width, height) = decoder.scale(requested, requested).ok()?;
    let (width, height) = (width as u32, height as u32);

    let pixels = decoder.decode().ok()?;
    if pixels.len() < (width as usize) * (height as usize) * channels {
        return None;
    }

    finish_scaled(pixels, width, height, channels, source, target_side, orientation)
}

/// Reduce a decoded JPEG to what the view asked for, and wrap it up.
///
/// Shared by both JPEG paths: one step further down when the ladder landed
/// well above what is needed, then the channel swap and the rotation — all of
/// it acting on the reduced buffer, never on the file's full pixel count.
fn finish_scaled(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    channels: usize,
    source: (u32, u32),
    target_side: u32,
    orientation: Option<u8>,
) -> Option<DecodedImage> {
    let wanted = ((target_side as f32) * DECODE_HEADROOM).round() as u32;
    let longest = width.max(height);
    let (pixels, width, height) = if wanted > 0 && longest > wanted {
        let (out_width, out_height) = scaled_size(width, height, wanted);
        let reduced = resample::resize(
            width,
            height,
            pixels,
            out_width,
            out_height,
            channels,
            resample::Kernel::Bilinear,
            resample::Alpha::Ignored,
        )
        .ok()?;
        (reduced, out_width, out_height)
    } else {
        (pixels, width, height)
    };

    let bgra = to_bgra(pixels, channels);
    // Rotating in place on the BGRA buffer avoids the round trip through
    // `DynamicImage`, which would allocate a second full-size image and swap
    // the channel order twice on the way through.
    let (width, height, bgra) =
        match orientation.and_then(crate::orientation::Orientation::from_exif) {
            Some(orientation) if !orientation.is_identity() => {
                transform_bgra(width, height, &bgra, &orientation)?
            }
            _ => (width, height, bgra),
        };

    // Reporting the file's own size is what lets the zoom maths ask for more
    // detail, and what keeps the info panel honest about the file.
    let reduced = (width, height) != source;

    Some(DecodedImage {
        width,
        height,
        frames: vec![DecodedFrame {
            bgra: Arc::new(Pixels::Heap(bgra)),
            delay_ms: 0,
        }],
        animated: false,
        exif_orientation: orientation,
        reduced_from: reduced.then_some(source),
    })
}

/// Decode a progressive JPEG, which the DCT ladder cannot scale.
///
/// `jpeg-decoder` only offers scaling for baseline images, so a progressive
/// file used to fall through to `image`'s general path: a full-size RGBA
/// intermediate plus several copies of it, about 360 MB of peak for a
/// 20-megapixel photograph — and one photograph in six in a typical folder is
/// progressive. The second decoder in the tree (`image` uses it too) reads
/// progressive files, so the picture comes out once as RGB8 and is reduced from
/// there; the peak is the decoded picture and the small output.
///
/// CMYK needs a colour transform this path does not perform, so those still
/// take the general route.
fn decode_jpeg_progressive(
    bytes: &[u8],
    target_side: u32,
    orientation: Option<u8>,
) -> Option<DecodedImage> {
    let mut decoder = zune_jpeg::JpegDecoder::new(zune_core::bytestream::ZCursor::new(bytes));
    decoder.decode_headers().ok()?;
    let (width, height) = decoder.dimensions()?;
    let source = (width as u32, height as u32);
    if jpeg_reduction(source.0.max(source.1), target_side) < DCT_MIN_REDUCTION {
        return None;
    }
    let channels = match decoder.output_colorspace()? {
        zune_core::colorspace::ColorSpace::Luma => 1,
        zune_core::colorspace::ColorSpace::RGB => 3,
        _ => return None,
    };

    let pixels = decoder.decode().ok()?;
    if pixels.len() < width * height * channels {
        return None;
    }
    finish_scaled(
        pixels,
        source.0,
        source.1,
        channels,
        source,
        target_side,
        orientation,
    )
}

/// Widen `channels`-byte pixels to the BGRA8 layout the frames use.
///
/// Takes the buffer by value: a four-channel source only needs its red and blue
/// swapped, which is done in place, so the common RGBA case allocates nothing.
fn to_bgra(pixels: Vec<u8>, channels: usize) -> Vec<u8> {
    match channels {
        3 => {
            let mut out = Vec::with_capacity(pixels.len() / 3 * 4);
            for pixel in pixels.chunks_exact(3) {
                out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 0xff]);
            }
            out
        }
        1 => {
            let mut out = Vec::with_capacity(pixels.len() * 4);
            for grey in pixels {
                out.extend_from_slice(&[grey, grey, grey, 0xff]);
            }
            out
        }
        _ => rgba_to_bgra(pixels),
    }
}

/// Turn a decoded image into display frames, reducing it first when the view
/// cannot show the extra detail.
///
/// The reduction happens on the decoder's own buffer, before it is expanded to
/// RGBA and before the channel swap. A JPEG decodes to three channels, so this
/// resizes three quarters of the pixels and expands the result once, instead of
/// expanding, copying and converting twelve megapixels four times over.
fn finish(image: DynamicImage, orientation: Option<u8>, target_side: Option<u32>) -> DecodedImage {
    let source = (image.width(), image.height());

    // Keep the decoder's own channel count: a JPEG or a plain PNG has three
    // channels, and resampling three quarters of the pixels is free speed.
    let (raw, channels) = match image {
        DynamicImage::ImageRgb8(buffer) => (buffer.into_raw(), 3),
        DynamicImage::ImageRgba8(buffer) => (buffer.into_raw(), 4),
        DynamicImage::ImageLuma8(buffer) => (buffer.into_raw(), 1),
        other => (other.into_rgba8().into_raw(), 4),
    };

    let wanted = target_side
        .filter(|side| *side > 0)
        .map(|side| ((side as f32) * DECODE_HEADROOM).round() as u32)
        .filter(|side| worth_reducing(source.0, source.1, *side));

    let (raw, width, height) = match wanted {
        Some(side) => {
            let (out_width, out_height) = scaled_size(source.0, source.1, side);
            match resample::resize(
                source.0,
                source.1,
                raw,
                out_width,
                out_height,
                channels,
                resample::Kernel::Bilinear,
                resample::Alpha::Ignored,
            ) {
                Ok(reduced) => (reduced, out_width, out_height),
                // A buffer that does not match the stated size means a decoder
                // produced something unexpected, so leave it rather than guess.
                Err(original) => (original, source.0, source.1),
            }
        }
        None => (raw, source.0, source.1),
    };

    let reduced = (width, height) != source;

    DecodedImage {
        width,
        height,
        frames: vec![DecodedFrame {
            bgra: Arc::new(Pixels::Heap(to_bgra(raw, channels))),
            delay_ms: 0,
        }],
        animated: false,
        exif_orientation: orientation,
        // Remember what the file held, so the reduction stays an internal
        // detail rather than something the UI reports.
        reduced_from: reduced.then_some(source),
    }
}


/// The size a `width` x `height` image takes when its longest side becomes
/// `target_side`.
fn scaled_size(width: u32, height: u32, target_side: u32) -> (u32, u32) {
    let longest = width.max(height) as f32;
    if longest <= 0. {
        return (width, height);
    }
    let factor = target_side as f32 / longest;
    (
        ((width as f32 * factor).round() as u32).max(1),
        ((height as f32 * factor).round() as u32).max(1),
    )
}


/// Longest side an image may keep before it is worth reducing.
fn worth_reducing(width: u32, height: u32, target_side: u32) -> bool {
    let longest = width.max(height);
    longest > target_side.saturating_mul(REDUCE_SLACK) / REDUCE_SLACK_DIVISOR
}

/// Scale every frame down so the longest side is at most `target_side`.
///
/// Frames share one factor, so an animation keeps its proportions and its
/// frames stay interchangeable.
pub fn reduce_to(image: DecodedImage, target_side: u32) -> DecodedImage {
    if target_side == 0 || !worth_reducing(image.width, image.height, target_side) {
        return image;
    }
    let longest = image.width.max(image.height) as f32;
    let factor = target_side as f32 / longest;
    let width = ((image.width as f32 * factor).round() as u32).max(1);
    let height = ((image.height as f32 * factor).round() as u32).max(1);

    let Some(first) = image.frames.first() else {
        return image;
    };
    // A buffer that does not match the stated size means a decoder produced
    // something unexpected, so leave it rather than guess.
    let expected = (image.width as usize) * (image.height as usize) * 4;
    if first.bgra.len() != expected {
        return image;
    }

    let mut frames = Vec::with_capacity(image.frames.len());
    for index in 0..image.frames.len() {
        // Straight on the BGRA buffer through the SIMD resampler: the previous
        // path wrapped each frame as an RGBA image, resized it with the scalar
        // `imageops` filter and swapped the channels twice per frame.
        let Some(scaled) = resample::resize_bgra(
            image.width,
            image.height,
            image.frames[index].bgra.to_vec(),
            width,
            height,
            resample::Kernel::Bilinear,
        )
        .ok()
        else {
            return image;
        };
        let delay_ms = image.frames[index].delay_ms;
        frames.push(DecodedFrame {
            bgra: Arc::new(Pixels::Heap(scaled)),
            delay_ms,
        });
    }

    DecodedImage {
        width,
        height,
        frames,
        animated: image.animated,
        exif_orientation: image.exif_orientation,
        // Remember what the file held, so the reduction stays an internal
        // detail rather than something the UI reports.
        reduced_from: Some(image.source_size()),
    }
}

/// Decode every frame of a GIF, or `None` if it cannot be read as one.
fn decode_gif(bytes: &[u8]) -> Option<DecodedImage> {
    use image::AnimationDecoder;
    use image::codecs::gif::GifDecoder;

    let decoder = GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    animation_frames(frames)
}

/// True when `bytes` hold an animated WebP.
///
/// Only the header is parsed, so this is cheap enough to call before deciding
/// how to decode a file.
pub fn is_animated_webp(bytes: &[u8]) -> bool {
    image::guess_format(bytes).ok() == Some(image::ImageFormat::WebP)
        && image_webp::WebPDecoder::new(std::io::Cursor::new(bytes))
            .is_ok_and(|decoder| decoder.is_animated())
}

/// An animated WebP decoded frame by frame.
///
/// Waiting for the whole animation before showing anything is what turns a
/// 157-frame 1280x1560 file into four or five seconds of blank window (and 1.2
/// GB of pixels). The viewer opens this instead, shows the frames as they land
/// and starts playing while the rest are still being decoded.
pub struct WebPAnimation {
    decoder: image_webp::WebPDecoder<std::io::Cursor<Vec<u8>>>,
    source: (u32, u32),
    frames: Vec<DecodedFrame>,
    total: usize,
}

impl WebPAnimation {
    /// Open the animation, or `None` when the bytes are not an animated WebP.
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        if image::guess_format(&bytes).ok() != Some(image::ImageFormat::WebP) {
            return None;
        }
        let decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
        if !decoder.is_animated() {
            return None;
        }
        let total = decoder.num_frames() as usize;
        if total < 2 {
            return None;
        }
        let source = decoder.dimensions();
        Some(Self {
            decoder,
            source,
            // A crafted file can claim many frames; the descriptor vector grows
            // with what actually decodes.
            frames: Vec::with_capacity(total.min(1024)),
            total,
        })
    }

    /// The file's own dimensions; animations are never reduced.
    pub fn source_size(&self) -> (u32, u32) {
        self.source
    }

    /// Frames the file declares, whether or not they have been decoded yet.
    pub fn total_frames(&self) -> usize {
        self.total
    }

    /// Frames decoded so far, which is what the viewer can play.
    pub fn decoded_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn is_done(&self) -> bool {
        self.frames.len() >= self.total
    }

    /// Decode up to `max` more frames, moving each into `store` when it will
    /// take it. Returns how many frames were appended.
    pub fn decode_chunk(
        &mut self,
        max: usize,
        store: Option<&crate::swap::FrameStore>,
    ) -> Result<usize, DecodeError> {
        let channels = if self.decoder.has_alpha() { 4 } else { 3 };
        let (width, height) = self.source;
        let buffer_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(channels))
            .ok_or_else(|| DecodeError::Corrupt("WebP 尺寸无效".to_string()))?;

        let mut added = 0;
        while added < max && !self.is_done() {
            let mut raw = vec![0u8; buffer_len];
            let delay = self
                .decoder
                .read_frame(&mut raw)
                .map_err(|error| DecodeError::Corrupt(error.to_string()))?;
            let bgra = to_bgra(raw, channels);
            let pixels = match store {
                Some(store) => frame_into_store(bgra, store),
                None => Pixels::Heap(bgra),
            };
            self.frames.push(DecodedFrame {
                bgra: Arc::new(pixels),
                delay_ms: animation_delay_ms(delay),
            });
            added += 1;
        }
        Ok(added)
    }

    /// The frames decoded so far, as a displayable image.
    ///
    /// Frames share their pixels, so handing out a snapshot after every chunk
    /// costs a clone of the small descriptors rather than of the whole
    /// animation.
    pub fn snapshot(&self) -> DecodedImage {
        DecodedImage {
            width: self.source.0,
            height: self.source.1,
            frames: self.frames.clone(),
            animated: self.total > 1,
            exif_orientation: None,
            reduced_from: None,
        }
    }
}

/// Move freshly decoded pixels into the swap store, or leave them on the heap.
fn frame_into_store(bytes: Vec<u8>, store: &crate::swap::FrameStore) -> Pixels {
    let Some(mut slot) = store.acquire(bytes.len()) else {
        return Pixels::Heap(bytes);
    };
    let Some(destination) = slot.as_mut() else {
        return Pixels::Heap(bytes);
    };
    destination.copy_from_slice(&bytes);
    Pixels::Mapped(Box::new(slot))
}

/// Clamp a frame delay the way browsers do.
fn animation_delay_ms(ms: u32) -> u32 {
    if ms == 0 {
        100
    } else {
        ms.clamp(20, 10_000)
    }
}

/// Decode every frame of an animated WebP, or `None` if the file is not one.
///
/// A static WebP is declined here: the general decoder reads it with the
/// decoder's own three channels, where this path would widen every frame to
/// RGBA for pixels that carry no alpha.
fn decode_webp_animation(bytes: &[u8]) -> Option<DecodedImage> {
    use image::AnimationDecoder;
    use image::codecs::webp::WebPDecoder;

    let decoder = WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    if !decoder.has_animation() {
        return None;
    }
    // The animation decoder composites each frame onto the full canvas, so a
    // frame that only patches part of the picture still arrives whole.
    let frames = decoder.into_frames().collect_frames().ok()?;
    animation_frames(frames)
}

/// Turn a sequence of decoded animation frames into display frames.
fn animation_frames(frames: Vec<image::Frame>) -> Option<DecodedImage> {
    if frames.is_empty() {
        return None;
    }

    let mut width = 0;
    let mut height = 0;
    let mut decoded = Vec::with_capacity(frames.len());

    for frame in frames {
        let delay = frame.delay();
        let (numerator, denominator) = delay.numer_denom_ms();
        // A zero or absurd delay makes an animation flash uncontrollably;
        // browsers clamp the same way.
        let delay_ms = if denominator == 0 {
            100
        } else {
            (numerator / denominator).clamp(20, 10_000)
        };

        let buffer = frame.into_buffer();
        width = width.max(buffer.width());
        height = height.max(buffer.height());
        decoded.push(DecodedFrame {
            bgra: Arc::new(Pixels::Heap(rgba_to_bgra(buffer.into_raw()))),
            delay_ms,
        });
    }

    Some(DecodedImage {
        width,
        height,
        animated: decoded.len() > 1,
        frames: decoded,
        // Rotating an animation frame by frame is not meaningful here.
        exif_orientation: None,
        reduced_from: None,
    })
}

/// Read the EXIF orientation tag, if the file has one.
fn read_exif_orientation(bytes: &[u8]) -> Option<u8> {
    let mut cursor = std::io::Cursor::new(bytes);
    let exif = exif::Reader::new().read_from_container(&mut cursor).ok()?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    u8::try_from(field.value.get_uint(0)?).ok()
}

/// Swap the red and blue channels in place.
///
/// This is the same transformation GPUI's own image loader performs, and it must
/// happen exactly once per image.
pub fn rgba_to_bgra(mut pixels: Vec<u8>) -> Vec<u8> {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    pixels
}

/// The smallest side a mip level may have before we stop halving.
const MIN_MIP_SIDE: u32 = 32;

/// A downscale pyramid for `bytes`, each level half the size of the last.
///
/// Drawing a large image at a small zoom with the GPU's bilinear sampler alone
/// produces visible aliasing, because it only ever reads four texels. Sampling a
/// level that is already close to the drawn size avoids that. `bytes` are BGRA8
/// and every level keeps that channel order; resize is channel-agnostic.
pub fn mip_chain(
    width: u32,
    height: u32,
    bytes: &[u8],
    interpolation: Interpolation,
) -> Vec<(u32, u32, Vec<u8>)> {
    if bytes.len() != (width as usize) * (height as usize) * 4 {
        return Vec::new();
    }

    // Each level is resampled straight from the one before it and then moved
    // into the list, so the chain costs one allocation per level and no copies
    // of the levels themselves.
    let kernel = resample::kernel(interpolation);
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    let (mut current_w, mut current_h) = (width, height);
    // The buffer the next step reads: `None` means the caller's own bytes.
    let mut previous: Option<Vec<u8>> = None;

    loop {
        let (next_w, next_h) = (current_w / 2, current_h / 2);
        if next_w < MIN_MIP_SIDE || next_h < MIN_MIP_SIDE {
            break;
        }
        // The resampling kernel the user picked, so the interpolation setting
        // genuinely changes how the downscaled level looks.
        let source = previous.as_deref().unwrap_or(bytes);
        let Some(next) = resample::resize_bgra_ref(
            current_w,
            current_h,
            source,
            next_w,
            next_h,
            kernel,
        ) else {
            break;
        };
        // The buffer just read is finished with, so it becomes a level now.
        if let Some(done) = previous.take() {
            levels.push((current_w, current_h, done));
        }
        previous = Some(next);
        current_w = next_w;
        current_h = next_h;
    }

    // The last level was never read from, so it is still pending.
    if let Some(last) = previous {
        levels.push((current_w, current_h, last));
    }

    levels
}

/// Pick the mip level to draw at a given scale, or `None` to use full size.
///
/// Returns the index into the level list, where 0 is the first halving.
pub fn mip_level_for_scale(
    levels: &[(u32, u32, Vec<u8>)],
    width: u32,
    height: u32,
    scale: f32,
) -> Option<usize> {
    if levels.is_empty() || scale >= 1.0 {
        return None;
    }

    // Choose the smallest level that is still at least as large as what we are
    // about to draw, so we never sample up from a coarser level than needed.
    let wanted_w = (width as f32 * scale).max(1.);
    let wanted_h = (height as f32 * scale).max(1.);

    levels
        .iter()
        .position(|(w, h, _)| (*w as f32) < wanted_w || (*h as f32) < wanted_h)
        .map(|index| index.saturating_sub(1))
        .or(Some(levels.len() - 1))
}

/// Rotate and mirror a BGRA buffer.
///
/// This is a scatter built on [`Orientation::map_point`], which is unit-tested
/// for bijectivity across every rotation and flip combination.
pub fn transform_bgra(
    width: u32,
    height: u32,
    bytes: &[u8],
    orientation: &crate::orientation::Orientation,
) -> Option<(u32, u32, Vec<u8>)> {
    let source = crate::geom::Size::new(width as f32, height as f32);
    if bytes.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    if orientation.is_identity() {
        return Some((width, height, bytes.to_vec()));
    }

    let dest = orientation.applied_size(source);
    let (dest_w, dest_h) = (dest.w as u32, dest.h as u32);
    let mut out = vec![0u8; (dest_w as usize) * (dest_h as usize) * 4];

    for y in 0..height {
        for x in 0..width {
            let (dest_x, dest_y) = orientation.map_point(x, y, source);
            let from = ((y * width + x) * 4) as usize;
            let to = ((dest_y * dest_w + dest_x) * 4) as usize;
            out[to..to + 4].copy_from_slice(&bytes[from..from + 4]);
        }
    }

    Some((dest_w, dest_h, out))
}

/// Scale a BGRA buffer down to fit inside a `max_width` × `max_height` box.
///
/// The source is usually the DCT-sized JPEG output or a decoded frame, and this
/// is the only reduction it gets, so a cubic filter is used: bilinear samples
/// just 2 x 2 pixels of a 4–8x larger source and visibly erases fine detail in
/// a thumbnail. The output is tiny, so the better kernel costs almost nothing.
///
/// Constraining both sides matters for webtoon pages: scaling only the width
/// leaves a 112 × 5000 thumbnail for a very tall image, which is several times
/// the memory of a normal tile. The source is borrowed, so the full frame is not
/// copied before it is reduced.
pub fn thumbnail(
    width: u32,
    height: u32,
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return None;
    }
    // Validated before the shortcut below, so a buffer that does not match its
    // stated size is refused rather than passed straight through.
    if bytes.len() < (width as usize) * (height as usize) * 4 {
        return None;
    }

    // Never upscale: a small source is already its own thumbnail.
    if width <= max_width && height <= max_height {
        return Some((width, height, bytes.to_vec()));
    }

    let scale = (max_width as f32 / width as f32).min(max_height as f32 / height as f32);
    let target_width = ((width as f32) * scale).round().max(1.) as u32;
    let target_height = ((height as f32) * scale).round().max(1.) as u32;
    let scaled = resample::resize_bgra_ref(
        width,
        height,
        bytes,
        target_width,
        target_height,
        resample::Kernel::CatmullRom,
    )?;
    Some((target_width, target_height, scaled))
}

/// Decode only what a thumbnail box needs, never the original-sized frame.
///
/// The viewer can hold tens of thousands of thumbnails' worth of entries, and
/// the old path decoded every file at full resolution just to shrink it. JPEGs
/// now go through the scaled DCT route, so a 20-megapixel photo decodes to
/// roughly the box size; every other format is decoded once and reduced here.
///
/// The final reduction is the only one. Asking `decode_for_display` for the box
/// size would shrink it to the target plus headroom and then `thumbnail` would
/// shrink it again; two bilinear steps, the first of them 3–4x, visibly soften
/// fine detail. The JPEG path therefore asks the decoder for the DCT output
/// size, and every other format is left at its decoded size.
///
/// Animations contribute their first frame only. The display path decodes all
/// of them, which for a long WebP is hundreds of megabytes spent to produce one
/// 112-pixel tile.
pub fn thumbnail_from_bytes(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    if max_width == 0 || max_height == 0 {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    let target_side = max_width.max(max_height);

    let decoded = if format == image::ImageFormat::Gif {
        // `load_from_memory_with_format` reads the first frame, unlike
        // `decode_for_display`, which expands the whole animation.
        let image = image::load_from_memory_with_format(bytes, format).ok()?;
        finish(image, None, None)
    } else if format == image::ImageFormat::WebP && is_animated_webp(bytes) {
        let image = image::load_from_memory_with_format(bytes, format).ok()?;
        finish(image, None, None)
    } else if format == image::ImageFormat::Jpeg {
        // Ask for the size the DCT ladder will produce: `finish_scaled` then
        // leaves it alone, and the single resize in `thumbnail` does the work.
        let side = jpeg_thumbnail_side(bytes, target_side);
        decode_for_display(bytes, true, Some(side)).ok()?
    } else {
        // These formats cannot decode at a reduced size, so there is nothing to
        // gain from a first pass; one reduction from the decoded pixels keeps
        // all the detail the thumbnail can hold.
        decode(bytes, true).ok()?
    };

    let frame = decoded.frames.first()?;
    thumbnail(decoded.width, decoded.height, &frame.bgra, max_width, max_height)
}

/// The longest side a JPEG should be decoded to for a thumbnail box.
///
/// Mirrors [`jpeg_reduction`]'s choice without decoding the pixels: the DCT
/// ladder's output for this box is at least `target_side`, and asking for
/// exactly that size keeps `finish_scaled` from reducing it a second time. A
/// file the scaled route cannot handle reports `target_side`, which is where
/// the general decoder would land anyway.
fn jpeg_thumbnail_side(bytes: &[u8], target_side: u32) -> u32 {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let Ok(()) = decoder.read_info() else {
        return target_side;
    };
    let Some(info) = decoder.info() else {
        return target_side;
    };
    let source = u32::from(info.width).max(u32::from(info.height));
    source
        .div_ceil(jpeg_reduction(source, target_side))
        .max(target_side)
}

/// Decode just enough of an image to learn its dimensions.
///
/// Used to fill in the file list without paying for a full decode.
pub fn probe_dimensions(bytes: &[u8]) -> Option<crate::geom::Size> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    Some(crate::geom::Size::new(w as f32, h as f32))
}

/// Convert a decoded image into a `DynamicImage`, for tests and thumbnailing.
#[doc(hidden)]
pub fn to_dynamic(image: &DecodedImage) -> Option<DynamicImage> {
    let frame = image.frames.first()?;
    // Undo the BGRA swap so this is a normal RGBA buffer again.
    let rgba = rgba_to_bgra(frame.bgra.to_vec());
    RgbaImage::from_raw(image.width, image.height, rgba).map(DynamicImage::ImageRgba8)
}

#[cfg(test)]
mod tests {
    /// A 64x48 progressive JPEG. Progressive files are common in a photo
    /// folder, and they used to be decoded at full size — the single largest
    /// allocation the viewer made — so both scaled routes are covered.
    const PROGRESSIVE: &[u8] = include_bytes!("../tests/fixtures/progressive-64x48.jpg");

    /// Two 2x2 frames, red then green, 60 ms apart, looped forever.
    const WEBP_ANIMATED: &[u8] = include_bytes!("../tests/fixtures/animated.webp");
    /// The same red frame as a single still.
    const WEBP_STATIC: &[u8] = include_bytes!("../tests/fixtures/static.webp");

    use super::*;

    /// The largest longest-side a decode for `request` may produce.
    fn ceiling(request: u32) -> u32 {
        ((request as f32) * DECODE_HEADROOM).round() as u32
    }

    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, image::Rgba(color));
        let mut out = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode png");
        out.into_inner()
    }

    #[test]
    fn decodes_a_png_and_swaps_to_bgra() {
        let bytes = png(4, 3, [200, 30, 40, 255]);
        let decoded = decode(&bytes, true).unwrap();

        assert_eq!((decoded.width, decoded.height), (4, 3));
        assert_eq!(decoded.frames.len(), 1);
        assert!(!decoded.animated);

        // Source was RGBA (200, 30, 40, 255); BGRA must be (40, 30, 200, 255).
        let frame = &decoded.frames[0];
        assert_eq!(frame.bgra.len(), 4 * 3 * 4);
        assert_eq!(&frame.bgra[0..4], &[40, 30, 200, 255]);
    }

    #[test]
    fn swap_is_its_own_inverse() {
        let original = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let swapped = rgba_to_bgra(original.clone());
        assert_eq!(swapped, vec![3, 2, 1, 4, 7, 6, 5, 8]);
        assert_eq!(rgba_to_bgra(swapped), original);
    }

    /// `Orientation::from_exif` has to agree with the `image` crate's own EXIF
    /// table, which is what the unscaled decode path applies. Every one of the
    /// eight tag values is checked against it, on an image whose pixels are all
    /// distinct so a wrong rotation cannot pass by symmetry.
    #[test]
    fn a_progressive_jpeg_is_reduced_too() {
        let image = decode_for_display(PROGRESSIVE, false, Some(16)).expect("decodes");
        // The file's own size is what the zoom maths and the info panel use.
        assert_eq!(image.source_size(), (64, 48));
        assert!(
            image.width < 64 && image.height < 48,
            "a progressive file must be reduced, not kept at full size: {}x{}",
            image.width,
            image.height
        );
        assert_eq!(
            image.frames[0].bgra.len(),
            (image.width * image.height * 4) as usize,
            "one BGRA byte per channel per pixel"
        );
        // The fixture is a gradient, so the rows must not be identical.
        let stride = image.width as usize * 4;
        assert_ne!(
            image.frames[0].bgra[..stride],
            image.frames[0].bgra[stride..stride * 2],
            "the decoded pixels are the picture, not a blank buffer"
        );
        assert!(image.frames[0].bgra.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn exif_orientation_matches_the_image_crate_for_all_eight_values() {
        let (width, height) = (5u32, 3u32);
        // Distinct pixels: blue channel encodes the source index.
        let source: Vec<u8> = (0..(width * height))
            .flat_map(|index| [10, 20, index as u8, 255])
            .collect();

        for value in 1u8..=8 {
            let ours = crate::orientation::Orientation::from_exif(value)
                .unwrap_or_else(|| panic!("value {value} must map to an orientation"));

            let (out_w, out_h, ours_pixels) = if ours.is_identity() {
                (width, height, source.clone())
            } else {
                transform_bgra(width, height, &source, &ours)
                    .unwrap_or_else(|| panic!("value {value} must transform"))
            };

            // The reference: the same table, applied by `image`.
            let reference = image::metadata::Orientation::from_exif(value)
                .expect("the image crate knows all eight values");
            let mut as_image = image::DynamicImage::ImageRgba8(
                RgbaImage::from_raw(width, height, rgba_to_bgra(source.clone())).unwrap(),
            );
            as_image.apply_orientation(reference);
            let expected_size = (as_image.width(), as_image.height());
            let expected = rgba_to_bgra(as_image.into_rgba8().into_raw());

            assert_eq!(
                (out_w, out_h),
                expected_size,
                "size differs for EXIF {value}"
            );
            assert_eq!(
                ours_pixels, expected,
                "pixels differ for EXIF {value} (ours {ours:?})"
            );
        }
    }

    #[test]
    fn an_unknown_exif_orientation_is_left_alone() {
        assert!(crate::orientation::Orientation::from_exif(0).is_none());
        assert!(crate::orientation::Orientation::from_exif(9).is_none());
        assert!(crate::orientation::Orientation::from_exif(1).unwrap().is_identity());
    }

    #[test]
    fn averages_a_flat_frame_in_bgra_order() {
        // A single colour: BGRA (40, 30, 200) is RGB 0xC81E28.
        let bytes = vec![40, 30, 200, 255].repeat(16);
        assert_eq!(average_color(4, 4, &bytes), 0xC81E28);
    }

    #[test]
    fn the_average_ignores_fully_transparent_pixels() {
        // Half red, half transparent black: the transparent half must not pull
        // the result toward black, which is what makes it usable for a sticker.
        let mut bytes = vec![0, 0, 255, 255].repeat(8);
        bytes.extend(vec![0, 0, 0, 0].repeat(8));
        assert_eq!(average_color(4, 4, &bytes), 0xFF0000);
    }

    #[test]
    fn an_entirely_transparent_frame_has_no_colour() {
        let bytes = vec![0, 0, 0, 0].repeat(4);
        assert_eq!(average_color(2, 2, &bytes), 0);
    }

    #[test]
    fn the_blur_source_is_small_and_keeps_the_channel_order() {
        let bytes = png(64, 32, [10, 20, 30, 255]);
        let decoded = decode(&bytes, true).unwrap();
        let (width, height, bgra) = decoded.blur_source(32).unwrap();
        assert_eq!((width, height), (32, 16));
        assert_eq!(bgra.len(), 32 * 16 * 4);
        // RGBA (10, 20, 30) stays BGRA (30, 20, 10) after the resize.
        assert_eq!(&bgra[0..4], &[30, 20, 10, 255]);
    }

    #[test]
    fn a_display_sized_decode_keeps_only_what_the_view_needs() {
        let bytes = png(400, 300, [10, 20, 30, 255]);
        let reduced = decode_for_display(&bytes, true, Some(100)).unwrap();
        // 400 -> 100 at least, and no more than the decode headroom over it.
        let longest = reduced.width.max(reduced.height);
        assert!(
            (100..=ceiling(100)).contains(&longest),
            "expected the headroom window, got {longest}"
        );
        // The aspect ratio is kept, whatever size was chosen. Each side is
        // rounded independently, so allow the pixel that can come from that.
        assert!(
            (reduced.width * 3).abs_diff(reduced.height * 4) <= 4,
            "4:3 became {}x{}",
            reduced.width,
            reduced.height
        );
        assert_eq!(reduced.frames.len(), 1);
        assert_eq!(
            reduced.frames[0].bgra.len(),
            (reduced.width * reduced.height * 4) as usize
        );
        // The channel order survives the resize.
        assert_eq!(&reduced.frames[0].bgra[0..4], &[30, 20, 10, 255]);
    }

    #[test]
    fn a_decode_with_no_target_is_the_full_image() {
        let bytes = png(400, 300, [10, 20, 30, 255]);
        let full = decode_for_display(&bytes, true, None).unwrap();
        assert_eq!((full.width, full.height), (400, 300));
    }

    #[test]
    fn an_image_close_to_the_target_is_left_alone() {
        // 1000 against a 900 target is inside the slack, so the resize is
        // skipped rather than throwing away detail for nothing.
        let bytes = png(1000, 500, [1, 2, 3, 255]);
        let decoded = decode_for_display(&bytes, true, Some(900)).unwrap();
        assert_eq!((decoded.width, decoded.height), (1000, 500));
    }

    #[test]
    fn reducing_never_upscales() {
        let bytes = png(80, 60, [1, 2, 3, 255]);
        let decoded = decode_for_display(&bytes, true, Some(4000)).unwrap();
        assert_eq!((decoded.width, decoded.height), (80, 60));
    }

    #[test]
    fn reducing_keeps_the_aspect_ratio_of_a_portrait_page() {
        let bytes = png(800, 1200, [9, 9, 9, 255]);
        let decoded = decode_for_display(&bytes, true, Some(300)).unwrap();
        assert!(
            (decoded.width * 3).abs_diff(decoded.height * 2) <= 4,
            "2:3 became {}x{}",
            decoded.width,
            decoded.height
        );
        assert!((300..=ceiling(300)).contains(&decoded.height));
    }

    #[test]
    fn an_unreduced_image_reports_full_detail() {
        // Asked for 4000 but the image is only 400 wide: it is not reduced, so
        // it holds full detail and must not claim to be a 400px reduction.
        let bytes = png(400, 300, [1, 2, 3, 255]);
        let decoded = decode_for_display(&bytes, true, Some(4000)).unwrap();
        assert_eq!(decoded.reduced_from, None);
        assert_eq!(decoded.detail(), None, "full detail, so no upgrade is due");

        // A large image against a 2048 target is reduced and says so.
        let big = png(4000, 3000, [1, 2, 3, 255]);
        let reduced = decode_for_display(&big, true, Some(2048)).unwrap();
        assert_eq!(reduced.detail(), Some(reduced.width.max(reduced.height)));
        assert!(reduced.detail().unwrap() >= 2048);
    }

    #[test]
    fn a_reduction_remembers_the_files_own_size() {
        let bytes = png(4000, 3000, [1, 2, 3, 255]);
        let reduced = decode_for_display(&bytes, true, Some(1000)).unwrap();
        assert!(
            (reduced.width * 3).abs_diff(reduced.height * 4) <= 4,
            "4:3 became {}x{}",
            reduced.width,
            reduced.height
        );
        assert!((1000..=ceiling(1000)).contains(&reduced.width));
        assert_eq!(
            reduced.source_size(),
            (4000, 3000),
            "the info panel must still report the file's dimensions"
        );

        let full = decode_for_display(&bytes, true, None).unwrap();
        assert_eq!(full.source_size(), (4000, 3000));
        assert_eq!(full.reduced_from, None);
    }

    #[test]
    fn probing_reports_size_without_decoding_pixels() {
        let bytes = png(123, 45, [0, 0, 0, 255]);
        let (size, orientation) = probe_details(&bytes).unwrap();
        assert_eq!((size.w, size.h), (123., 45.));
        // The generated PNG carries no EXIF block.
        assert_eq!(orientation, None);
    }

    #[test]
    fn the_blur_source_never_upscales() {
        let bytes = png(8, 8, [1, 2, 3, 255]);
        let decoded = decode(&bytes, true).unwrap();
        let (width, height, _) = decoded.blur_source(32).unwrap();
        assert_eq!((width, height), (8, 8));
    }

    #[test]
    fn the_blur_source_is_the_average_of_each_block() {
        // Left half black, right half white: a two-pixel-wide result must be
        // the average of each half, not a sample of one corner.
        let (w, h) = (8u32, 2u32);
        let mut bytes = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let value = if x < 4 { 0 } else { 200 };
                bytes.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let decoded = DecodedImage {
            width: w,
            height: h,
            frames: vec![DecodedFrame {
                bgra: Arc::new(Pixels::Heap(bytes)),
                delay_ms: 0,
            }],
            animated: false,
            exif_orientation: None,
            reduced_from: None,
        };

        let (out_w, out_h, out) = decoded.blur_source(2).unwrap();
        assert_eq!((out_w, out_h), (2, 1));
        assert_eq!(&out[0..4], &[0, 0, 0, 255], "left block is all black");
        assert_eq!(&out[4..8], &[200, 200, 200, 255], "right block is all white");
    }

    #[test]
    fn the_blur_source_rejects_a_mismatched_buffer() {
        let decoded = DecodedImage {
            width: 4,
            height: 4,
            frames: vec![DecodedFrame {
                bgra: Arc::new(Pixels::Heap(vec![0; 16])),
                delay_ms: 0,
            }],
            animated: false,
            exif_orientation: None,
            reduced_from: None,
        };
        assert!(decoded.blur_source(2).is_none());
    }

    #[test]
    fn a_downsample_of_the_same_size_copies_the_pixels() {
        let bytes: Vec<u8> = (0..16u8).collect();
        let (w, h, out) = box_downsample_bgra(2, 2, &bytes, 2, 2).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(out, bytes);
    }

    /// A JPEG of the given size, all one colour.
    fn jpeg(width: u32, height: u32, colour: [u8; 3]) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb(colour));
        let mut out = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .expect("encode jpeg");
        out.into_inner()
    }

    #[test]
    fn a_jpeg_is_decoded_at_a_reduced_scale_when_the_view_is_small() {
        // 1024 wide against a 256 request: the DCT ladder lands on 1/4, so the
        // full-resolution pixels are never produced.
        let bytes = jpeg(1024, 768, [200, 60, 40]);
        let decoded = decode_for_display(&bytes, false, Some(256)).unwrap();

        assert!(
            (256..=512).contains(&decoded.width),
            "expected a quarter-scale decode, got {}x{}",
            decoded.width,
            decoded.height
        );
        assert_eq!(
            decoded.source_size(),
            (1024, 768),
            "the file's own size must still be reported"
        );
        assert!(
            decoded.detail().is_some(),
            "a scaled decode holds less than the file, so it must say so"
        );
    }

    #[test]
    fn a_scaled_jpeg_keeps_the_channel_order() {
        // The scaled path converts RGB to BGRA itself, so a swapped red and
        // blue would show up here and nowhere else.
        let bytes = jpeg(512, 512, [200, 60, 40]);
        let scaled = decode_for_display(&bytes, false, Some(128)).unwrap();
        let full = decode_for_display(&bytes, false, None).unwrap();

        for (label, image) in [("scaled", &scaled), ("full", &full)] {
            let frame = &image.frames[0];
            // JPEG is lossy, so the colour is only approximately preserved.
            let (b, g, r) = (frame.bgra[0], frame.bgra[1], frame.bgra[2]);
            assert!(b.abs_diff(40) < 12, "{label}: blue was {b}");
            assert!(g.abs_diff(60) < 12, "{label}: green was {g}");
            assert!(r.abs_diff(200) < 12, "{label}: red was {r}");
            assert_eq!(frame.bgra[3], 255, "{label}: alpha must be opaque");
        }
    }

    #[test]
    fn a_scaled_jpeg_is_at_least_as_large_as_asked_for() {
        let bytes = jpeg(1600, 1200, [10, 20, 30]);
        for request in [100u32, 200, 400, 800, 1600] {
            let decoded = decode_for_display(&bytes, false, Some(request)).unwrap();
            let longest = decoded.width.max(decoded.height);
            assert!(
                longest >= request,
                "asked for {request}, got a {longest}px decode"
            );
        }
    }

    #[test]
    fn a_scaled_jpeg_never_exceeds_the_decode_headroom() {
        // Whatever the ladder picks, the buffer must not be wildly bigger than
        // the view asked for: that memory is paid for and never shown.
        let bytes = jpeg(4000, 3000, [10, 20, 30]);
        let decoded = decode_for_display(&bytes, false, Some(1000)).unwrap();
        let longest = decoded.width.max(decoded.height);
        assert!(
            longest <= ceiling(1000),
            "a 1000px request produced {longest}px of pixels"
        );
    }

    #[test]
    fn a_scaled_jpeg_applies_the_exif_orientation() {
        // An 8x4 file tagged "rotate 90" must come back portrait through the
        // scaled path too, not just through the general decoder.
        let jpeg = jpeg_with_orientation(6);
        let scaled = decode_for_display(&jpeg, true, Some(4)).unwrap();
        let full = decode_for_display(&jpeg, true, None).unwrap();

        assert_eq!(scaled.exif_orientation, Some(6));
        assert_eq!((full.width, full.height), (4, 8), "the file is 8x4 rotated");
        assert!(
            scaled.width < scaled.height,
            "the scaled decode must be rotated as well, got {}x{}",
            scaled.width,
            scaled.height
        );
        assert_eq!(
            scaled.width * full.height,
            scaled.height * full.width,
            "both paths must agree on the shape"
        );
    }

    #[test]
    fn a_jpeg_asked_for_full_resolution_skips_the_scaled_path() {
        let bytes = jpeg(600, 400, [1, 2, 3]);
        let decoded = decode_for_display(&bytes, false, None).unwrap();
        assert_eq!((decoded.width, decoded.height), (600, 400));
        assert_eq!(decoded.detail(), None, "full resolution holds all detail");
    }

    #[test]
    fn the_dct_ladder_steps_down_only_as_far_as_the_request_allows() {
        // The decoder's own rule: the most aggressive IDCT whose output still
        // reaches the request.
        assert_eq!(jpeg_reduction(6000, 6000), 1, "full size for a full request");
        assert_eq!(jpeg_reduction(6000, 3000), 2);
        assert_eq!(jpeg_reduction(6000, 1500), 4);
        assert_eq!(jpeg_reduction(6000, 750), 8);
        // A request between two steps takes the shallower one, so the output
        // never falls below what the view asked for.
        assert_eq!(jpeg_reduction(6000, 1600), 2);
        assert_eq!(jpeg_reduction(6000, 760), 4);
    }

    #[test]
    fn a_jpeg_only_a_little_larger_than_the_view_is_not_scaled() {
        // 1000px against a 900px request steps down by one at most, which is
        // break-even; the general decoder is the better choice.
        let bytes = jpeg(1000, 800, [90, 120, 150]);
        let decoded = decode_for_display(&bytes, false, Some(900)).unwrap();
        assert_eq!(
            decoded.source_size(),
            (1000, 800),
            "the file must still be reported at its own size"
        );
        assert!(
            decoded.width <= ceiling(900),
            "and the buffer stays within the headroom, got {}",
            decoded.width
        );
    }

    #[test]
    fn a_progressive_jpeg_is_scaled_by_the_ladder() {
        // Progressive files go through the DCT ladder now: the same
        // coefficients come out of the entropy stage, only the inverse
        // transform is smaller. They used to be declined here, which sent every
        // one of them to a full-size decode.
        let scaled = decode_jpeg_scaled(PROGRESSIVE, 16, None)
            .expect("the ladder scales a progressive file");
        assert_eq!(scaled.source_size(), (64, 48));
        assert!(scaled.width < 64, "and reduces it");

        // The full path takes that route, not the general decoder.
        let decoded = decode_for_display(PROGRESSIVE, false, Some(16)).unwrap();
        assert_eq!(decoded.source_size(), (64, 48));
        assert!(decoded.width < 64);

        // The second route stays as the fallback for what the first declines.
        assert!(decode_jpeg_progressive(PROGRESSIVE, 16, None).is_some());
    }

    #[test]
    fn rejects_bytes_that_are_not_an_image() {
        assert!(matches!(
            decode(b"definitely not an image", true),
            Err(DecodeError::Unsupported)
        ));
    }

    #[test]
    fn decodes_every_frame_of_a_gif() {
        // Two frames, 60 ms apart.
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for color in [[255u8, 0, 0, 255], [0, 255, 0, 255]] {
                let frame = image::Frame::from_parts(
                    RgbaImage::from_pixel(2, 2, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(60, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }

        let decoded = decode(&gif, true).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert!(decoded.animated);
        assert_eq!(decoded.frames[0].delay_ms, 60);
        assert_eq!((decoded.width, decoded.height), (2, 2));
    }

    #[test]
    fn decodes_every_frame_of_an_animated_webp() {
        let decoded = decode(WEBP_ANIMATED, true).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert!(decoded.animated);
        assert_eq!((decoded.width, decoded.height), (2, 2));
        // The frames arrive composed and in BGRA, like every other decode.
        assert_eq!(&decoded.frames[0].bgra.as_slice()[..4], &[0, 0, 255, 255]);
        let green = &decoded.frames[1].bgra.as_slice()[..4];
        assert!(
            green[1] > 240 && green[0] < 10 && green[2] < 10,
            "the second frame is green, got {green:?}"
        );
        assert_eq!(decoded.frames[0].delay_ms, 60);
        assert_eq!(decoded.frames[1].delay_ms, 60);
    }

    #[test]
    fn streams_an_animated_webp_frame_by_frame() {
        let mut animation = WebPAnimation::new(WEBP_ANIMATED.to_vec()).expect("animated");
        assert_eq!(animation.total_frames(), 2);
        assert_eq!(animation.decoded_frames(), 0);

        // The first chunk is playable on its own, and already reports itself as
        // an animation so the viewer starts its play clock.
        assert_eq!(animation.decode_chunk(1, None).unwrap(), 1);
        let first = animation.snapshot();
        assert_eq!(first.frames.len(), 1);
        assert!(first.animated);
        assert_eq!(&first.frames[0].bgra.as_slice()[..4], &[0, 0, 255, 255]);
        assert_eq!(first.frames[0].delay_ms, 60);

        assert!(!animation.is_done());
        assert_eq!(animation.decode_chunk(4, None).unwrap(), 1);
        assert!(animation.is_done());

        let all = animation.snapshot();
        assert_eq!(all.frames.len(), 2);
        // Snapshots share frames rather than copying them; that is what makes
        // taking one after every chunk cheap.
        assert!(Arc::ptr_eq(&first.frames[0].bgra, &all.frames[0].bgra));
    }

    #[test]
    fn only_animated_webp_files_are_streamed() {
        assert!(!is_animated_webp(WEBP_STATIC));
        assert!(!is_animated_webp(b"not an image"));
        assert!(is_animated_webp(WEBP_ANIMATED));
        assert!(WebPAnimation::new(WEBP_STATIC.to_vec()).is_none());
    }

    #[test]
    fn a_static_webp_stays_a_single_still() {
        // Static files must not be routed through the animation decoder: the
        // general path keeps the decoder's three channels.
        let decoded = decode(WEBP_STATIC, true).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert!(!decoded.animated);
        assert_eq!(&decoded.frames[0].bgra.as_slice()[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn builds_a_halving_mip_chain() {
        let bytes = vec![0u8; 512 * 256 * 4];
        let levels = mip_chain(512, 256, &bytes, Interpolation::Bicubic);

        let sizes: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, vec![(256, 128), (128, 64), (64, 32)]);
        for (w, h, data) in &levels {
            assert_eq!(data.len(), (*w as usize) * (*h as usize) * 4);
        }
    }

    #[test]
    fn stops_halving_before_levels_get_tiny() {
        let bytes = vec![0u8; 64 * 64 * 4];
        let levels = mip_chain(64, 64, &bytes, Interpolation::Bicubic);
        let sizes: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
        // 64 -> 32 is allowed; 32 -> 16 is not, because 16 is under the floor.
        assert_eq!(sizes, vec![(32, 32)]);
    }

    #[test]
    fn rejects_a_mismatched_buffer() {
        // Fewer bytes than the dimensions claim.
        assert!(mip_chain(100, 100, &[0u8; 16], Interpolation::Bicubic).is_empty());
    }

    #[test]
    fn picks_a_mip_level_close_to_the_drawn_size() {
        let bytes = vec![0u8; 512 * 256 * 4];
        let levels = mip_chain(512, 256, &bytes, Interpolation::Bicubic);

        // Full size when drawing at or above 100%.
        assert_eq!(mip_level_for_scale(&levels, 512, 256, 1.0), None);
        assert_eq!(mip_level_for_scale(&levels, 512, 256, 2.0), None);

        // Half size: the first level is 256 wide, which still covers a 256-wide
        // draw, so level 0 is right.
        assert_eq!(mip_level_for_scale(&levels, 512, 256, 0.5), Some(0));

        // Drawing at a quarter needs something at least 128 wide: level 1.
        assert_eq!(mip_level_for_scale(&levels, 512, 256, 0.25), Some(1));

        // Very small draws land on the coarsest level available.
        assert_eq!(
            mip_level_for_scale(&levels, 512, 256, 0.01),
            Some(levels.len() - 1)
        );
    }

    #[test]
    fn no_mip_levels_is_never_an_index() {
        assert_eq!(mip_level_for_scale(&[], 100, 100, 0.1), None);
    }

    #[test]
    fn thumbnails_keep_the_aspect_ratio() {
        let bytes = vec![0u8; 400 * 200 * 4];
        let (w, h, out) = thumbnail(400, 200, &bytes, 100, 100).unwrap();
        assert_eq!((w, h), (100, 50));
        assert_eq!(out.len(), 100 * 50 * 4);
    }

    #[test]
    fn thumbnails_fit_the_height_too() {
        // A webtoon strip: constraining only the width would keep a 112 x 1160
        // tile, ten times the pixels of a normal one.
        let bytes = vec![0u8; 400 * 4000 * 4];
        let (w, h, out) = thumbnail(400, 4000, &bytes, 112, 116).unwrap();
        assert_eq!((w, h), (12, 116));
        assert_eq!(out.len(), 12 * 116 * 4);
    }

    #[test]
    fn thumbnails_never_upscale() {
        let bytes = vec![0u8; 40 * 20 * 4];
        let (w, h, out) = thumbnail(40, 20, &bytes, 100, 100).unwrap();
        assert_eq!((w, h), (40, 20), "a small image is already its own thumbnail");
        assert_eq!(out, bytes);
    }

    #[test]
    fn thumbnails_reject_degenerate_input() {
        assert!(thumbnail(0, 10, &[], 100, 100).is_none());
        // A buffer that does not match the stated size is refused even when the
        // source is small enough that no resampling would happen.
        assert!(thumbnail(10, 10, &[0u8; 16], 100, 100).is_none());
        assert!(thumbnail(10, 10, &[0u8; 400], 0, 100).is_none());
        assert!(thumbnail(10, 10, &[0u8; 400], 100, 0).is_none());
    }

    #[test]
    fn thumbnail_bytes_stay_within_the_box() {
        let bytes = png(400, 200, [10, 20, 30, 255]);
        let (w, h, out) = thumbnail_from_bytes(&bytes, 100, 100).unwrap();
        assert!(w <= 100 && h <= 100, "{w}x{h} exceeds the box");
        assert_eq!(out.len(), (w * h * 4) as usize);
    }

    #[test]
    fn thumbnail_bytes_read_only_the_first_animation_frame() {
        // Two 2x2 frames; thumbnailing must not expand both just to keep one.
        let (w, h, out) = thumbnail_from_bytes(WEBP_ANIMATED, 112, 116).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(out.len(), 2 * 2 * 4);
    }

    #[test]
    fn probes_dimensions_without_a_full_decode() {
        let bytes = png(37, 19, [1, 2, 3, 255]);
        assert_eq!(probe_dimensions(&bytes), Some(crate::geom::Size::new(37., 19.)));
        assert_eq!(probe_dimensions(b"nonsense"), None);
    }

    /// One BGRA pixel per coordinate, so a transform's effect is readable.
    fn labelled_bgra(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                out.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        out
    }

    fn pixel(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    }

    #[test]
    fn identity_transform_is_a_pass_through() {
        let source = labelled_bgra(4, 2);
        let (w, h, out) =
            transform_bgra(4, 2, &source, &crate::orientation::Orientation::default()).unwrap();
        assert_eq!((w, h), (4, 2));
        assert_eq!(out, source);
    }

    #[test]
    fn quarter_turn_rotates_pixels_and_swaps_dimensions() {
        let source = labelled_bgra(4, 2);
        let orientation = crate::orientation::Orientation {
            rotation: crate::orientation::Rotation::Cw90,
            ..Default::default()
        };
        let (w, h, out) = transform_bgra(4, 2, &source, &orientation).unwrap();

        assert_eq!((w, h), (2, 4));
        // Source (0,0) carried pixel (0,0) and must land at (1,0).
        assert_eq!(pixel(&out, w, 1, 0), [0, 0, 0, 255]);
        // Source (3,0) carried pixel (3,0) and must land at (1,3).
        assert_eq!(pixel(&out, w, 1, 3), [3, 0, 0, 255]);
    }

    #[test]
    fn mirroring_reverses_the_axis() {
        let source = labelled_bgra(4, 2);
        let orientation = crate::orientation::Orientation {
            flip_h: true,
            ..Default::default()
        };
        let (w, h, out) = transform_bgra(4, 2, &source, &orientation).unwrap();

        assert_eq!((w, h), (4, 2));
        assert_eq!(pixel(&out, w, 3, 0), [0, 0, 0, 255]);
        assert_eq!(pixel(&out, w, 0, 0), [3, 0, 0, 255]);
    }

    #[test]
    fn transform_rejects_a_buffer_of_the_wrong_length() {
        let orientation = crate::orientation::Orientation {
            flip_v: true,
            ..Default::default()
        };
        assert!(transform_bgra(4, 2, &[0u8; 8], &orientation).is_none());
    }

    #[test]
    fn every_transform_preserves_the_pixel_multiset() {
        // Guards against a transform dropping or duplicating pixels.
        let (w, h) = (5u32, 3u32);
        let source = labelled_bgra(w, h);
        let rotations = [
            crate::orientation::Rotation::None,
            crate::orientation::Rotation::Cw90,
            crate::orientation::Rotation::Cw180,
            crate::orientation::Rotation::Cw270,
        ];

        for rotation in rotations {
            for flip_h in [false, true] {
                for flip_v in [false, true] {
                    let orientation = crate::orientation::Orientation {
                        rotation,
                        flip_h,
                        flip_v,
                    };
                    let (dw, dh, out) = transform_bgra(w, h, &source, &orientation).unwrap();
                    assert_eq!(dw * dh, w * h);

                    let mut got: Vec<[u8; 4]> = Vec::new();
                    for y in 0..dh {
                        for x in 0..dw {
                            got.push(pixel(&out, dw, x, y));
                        }
                    }
                    let mut want: Vec<[u8; 4]> = Vec::new();
                    for y in 0..h {
                        for x in 0..w {
                            want.push(pixel(&source, w, x, y));
                        }
                    }
                    got.sort_unstable();
                    want.sort_unstable();
                    assert_eq!(got, want, "{orientation:?} changed the pixel set");
                }
            }
        }
    }

    #[test]
    fn reads_the_exif_orientation_tag() {
        // A minimal JPEG with an EXIF APP1 segment carrying Orientation = 6.
        let jpeg = jpeg_with_orientation(6);
        assert_eq!(read_exif_orientation(&jpeg), Some(6));
        assert_eq!(read_exif_orientation(b"no exif here"), None);
    }

    #[test]
    fn applying_orientation_swaps_the_dimensions() {
        // A 8x4 image tagged "rotate 90" must come back as 4x8.
        let jpeg = jpeg_with_orientation(6);
        let rotated = decode(&jpeg, true).unwrap();
        let unrotated = decode(&jpeg, false).unwrap();

        assert_eq!(unrotated.exif_orientation, None);
        assert_eq!(
            (rotated.width, rotated.height),
            (unrotated.height, unrotated.width)
        );
        assert_eq!(rotated.exif_orientation, Some(6));
    }

    /// Build a tiny JPEG whose EXIF says it needs rotating.
    fn jpeg_with_orientation(orientation: u8) -> Vec<u8> {
        // JPEG has no alpha channel, so encode RGB rather than RGBA.
        let image = image::RgbImage::from_pixel(8, 4, image::Rgb([10, 20, 30]));
        let mut plain = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut plain, image::ImageFormat::Jpeg)
            .unwrap();
        let plain = plain.into_inner();

        // Build an APP1/EXIF block by hand: TIFF header, one IFD with a single
        // Orientation entry.
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM\x00\x2a"); // big endian
        tiff.extend_from_slice(&8u32.to_be_bytes()); // IFD offset
        tiff.extend_from_slice(&1u16.to_be_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_be_bytes()); // count
        tiff.extend_from_slice(&(orientation as u16).to_be_bytes());
        tiff.extend_from_slice(&0u16.to_be_bytes()); // padding
        tiff.extend_from_slice(&0u32.to_be_bytes()); // next IFD

        let mut segment = Vec::new();
        segment.extend_from_slice(b"Exif\x00\x00");
        segment.extend_from_slice(&tiff);

        let length = (segment.len() + 2) as u16;
        let mut out = Vec::new();
        out.extend_from_slice(&[0xFF, 0xD8]); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]); // APP1
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&segment);
        // The original JPEG minus its own SOI, so the EXIF lands first.
        out.extend_from_slice(&plain[2..]);
        out
    }
}

/// Read an image's dimensions and EXIF orientation without decoding pixels.
///
/// This is what the persistent metadata cache stores, so a second visit to a
/// folder or comic costs no reads at all.
pub fn probe_details(bytes: &[u8]) -> Option<(crate::geom::Size, Option<u8>)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    let orientation = read_exif_orientation(bytes);
    Some((crate::geom::Size::new(w as f32, h as f32), orientation))
}
