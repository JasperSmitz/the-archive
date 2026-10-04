use crate::error::Error;
use sha2::{Digest, Sha256};
use std::io::Cursor;
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_AXIS: u32 = 12000;
pub const MAX_PIXELS: u64 = 40_000_000;
pub const DECODER_BUDGET: usize = 512 * 1024 * 1024;
#[derive(Clone, Copy)]
pub struct Limits {
    pub file_bytes: usize,
    pub axis: u32,
    pub pixels: u64,
    pub allocation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: MAX_FILE_BYTES,
            axis: MAX_AXIS,
            pixels: MAX_PIXELS,
            allocation: DECODER_BUDGET,
        }
    }
}
#[derive(Debug)]
pub struct Validated {
    pub width: u32,
    pub height: u32,
    pub content_type: &'static str,
    pub extension: &'static str,
    pub sha256: Vec<u8>,
}
pub fn invalid(message: impl Into<String>) -> Error {
    Error::Validation(vec![("file".into(), message.into())])
}
fn corrupt<E: std::fmt::Display>(e: E) -> Error {
    tracing::debug!(error=%e,"image decoding rejected input");
    invalid("The image is corrupt, truncated, or exceeds decoder limits.")
}
fn animation() -> Error {
    invalid("Animated images are not supported. Select a static JPEG, PNG, or WebP.")
}
fn dimensions(w: u32, h: u32, l: Limits) -> Result<(), Error> {
    if w == 0 || h == 0 || w > l.axis || h > l.axis || u64::from(w) * u64::from(h) > l.pixels {
        return Err(invalid(format!(
            "Images must be at most {} pixels on either axis and {} decoded pixels.",
            l.axis, l.pixels
        )));
    }
    Ok(())
}
fn buffer(size: usize, l: Limits) -> Result<Vec<u8>, Error> {
    if size > l.allocation {
        return Err(invalid("The image exceeds the decoder allocation budget."));
    }
    let mut v = Vec::new();
    v.try_reserve_exact(size).map_err(corrupt)?;
    v.resize(size, 0);
    Ok(v)
}
/// Full decoding on a blocking thread. File, dimensions, pixel count, and output allocations
/// are strict bounds. PNG/WebP internal budgets are best-effort (see README).
pub fn inspect(bytes: &[u8], l: Limits) -> Result<Validated, Error> {
    if bytes.len() > l.file_bytes {
        return Err(Error::TooLarge);
    }
    if bytes.is_empty() {
        return Err(invalid("Select an image file."));
    }
    let format = image::guess_format(bytes).map_err(|_| {
        invalid("Select a JPEG, PNG, or WebP image. SVG and other formats are unsupported.")
    })?;
    let (width, height, content_type, extension) = match format {
        image::ImageFormat::Png => {
            // Walk the complete container, including chunks after IDAT, before decoding.
            let mut pos = 8_usize;
            let mut end = false;
            while pos < bytes.len() {
                let head = bytes
                    .get(pos..pos.checked_add(8).ok_or_else(|| corrupt("overflow"))?)
                    .ok_or_else(|| corrupt("short PNG chunk"))?;
                let size = u32::from_be_bytes(head[..4].try_into().map_err(corrupt)?) as usize;
                let kind = &head[4..8];
                let next = pos
                    .checked_add(12)
                    .and_then(|p| p.checked_add(size))
                    .filter(|p| *p <= bytes.len())
                    .ok_or_else(|| corrupt("truncated PNG"))?;
                if kind == b"acTL" {
                    return Err(animation());
                }
                if kind == b"IEND" {
                    if size != 0 || next != bytes.len() {
                        return Err(corrupt("invalid PNG end"));
                    }
                    end = true;
                    break;
                }
                pos = next;
            }
            if !end {
                return Err(corrupt("missing PNG end"));
            }
            let mut decoder = png::Decoder::new_with_limits(
                Cursor::new(bytes),
                png::Limits {
                    bytes: l.allocation,
                },
            );
            let info = decoder.read_header_info().map_err(corrupt)?;
            let (w, h) = (info.width, info.height);
            dimensions(w, h, l)?;
            if info.animation_control.is_some() {
                return Err(animation());
            }
            decoder.set_transformations(png::Transformations::EXPAND);
            let mut reader = decoder.read_info().map_err(corrupt)?;
            let size = reader
                .output_buffer_size()
                .ok_or_else(|| corrupt("PNG output too large"))?;
            let mut out = buffer(size, l)?;
            reader.next_frame(&mut out).map_err(corrupt)?;
            reader.finish().map_err(corrupt)?;
            (w, h, "image/png", "png")
        }
        image::ImageFormat::Jpeg => {
            // The high-level image wrapper intentionally uses tolerant JPEG decoding. Use strict
            // zune options directly so truncated/corrupt uploads are not silently repaired.
            if !bytes.ends_with(&[0xff, 0xd9]) {
                return Err(corrupt("missing JPEG end"));
            }
            let options = zune_core::options::DecoderOptions::default()
                .set_strict_mode(true)
                .set_max_width(l.axis as usize)
                .set_max_height(l.axis as usize);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
                zune_core::bytestream::ZCursor::new(bytes),
                options,
            );
            decoder.decode_headers().map_err(corrupt)?;
            let (w, h) = decoder
                .dimensions()
                .ok_or_else(|| corrupt("JPEG dimensions"))?;
            let (w, h) = (
                u32::try_from(w).map_err(corrupt)?,
                u32::try_from(h).map_err(corrupt)?,
            );
            dimensions(w, h, l)?;
            let size = decoder
                .output_buffer_size()
                .ok_or_else(|| corrupt("JPEG output size"))?;
            let mut out = buffer(size, l)?;
            decoder.decode_into(&mut out).map_err(corrupt)?;
            (w, h, "image/jpeg", "jpg")
        }
        image::ImageFormat::WebP => {
            let size = bytes.get(4..8).ok_or_else(|| corrupt("WebP header"))?;
            let declared = u32::from_le_bytes(size.try_into().map_err(corrupt)?) as u64;
            if declared + 8 != bytes.len() as u64 {
                return Err(corrupt("truncated WebP container"));
            }
            let mut pos = 12_usize;
            let mut coded_dimensions = Vec::new();
            while pos < bytes.len() {
                let head = bytes
                    .get(pos..pos.checked_add(8).ok_or_else(|| corrupt("overflow"))?)
                    .ok_or_else(|| corrupt("short WebP chunk"))?;
                if &head[..4] == b"ANIM" || &head[..4] == b"ANMF" {
                    return Err(animation());
                }
                let size = u32::from_le_bytes(head[4..8].try_into().map_err(corrupt)?) as usize;
                let next = pos
                    .checked_add(8)
                    .and_then(|p| p.checked_add(size))
                    .and_then(|p| p.checked_add(size % 2))
                    .filter(|p| *p <= bytes.len())
                    .ok_or_else(|| corrupt("short WebP chunk"))?;
                let payload = &bytes[pos + 8..pos + 8 + size];
                let actual = match &head[..4] {
                    b"VP8 " => {
                        let header = payload
                            .get(..10)
                            .ok_or_else(|| corrupt("short VP8 header"))?;
                        if header[3..6] != [0x9d, 0x01, 0x2a] {
                            return Err(corrupt("invalid VP8 frame"));
                        }
                        Some((
                            u32::from(u16::from_le_bytes([header[6], header[7]]) & 0x3fff),
                            u32::from(u16::from_le_bytes([header[8], header[9]]) & 0x3fff),
                        ))
                    }
                    b"VP8L" => {
                        let header = payload
                            .get(..5)
                            .ok_or_else(|| corrupt("short VP8L header"))?;
                        let bits = u32::from_le_bytes(header[1..5].try_into().map_err(corrupt)?);
                        Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
                    }
                    b"VP8X" => {
                        let header = payload
                            .get(..10)
                            .ok_or_else(|| corrupt("short VP8X header"))?;
                        if header[0] & 2 != 0 {
                            return Err(animation());
                        }
                        let w = u32::from_le_bytes([header[4], header[5], header[6], 0]) + 1;
                        let h = u32::from_le_bytes([header[7], header[8], header[9], 0]) + 1;
                        dimensions(w, h, l)?;
                        None
                    }
                    _ => None,
                };
                if let Some((w, h)) = actual {
                    dimensions(w, h, l)?;
                    coded_dimensions.push((w, h));
                }
                pos = next;
            }
            let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(corrupt)?;
            if decoder.is_animated() {
                return Err(animation());
            }
            let (w, h) = decoder.dimensions();
            dimensions(w, h, l)?;
            // Check coded frame dimensions too: a small VP8X canvas must never conceal
            // a much larger VP8/VP8L frame and trigger allocations before mismatch detection.
            if coded_dimensions.iter().any(|d| *d != (w, h)) {
                return Err(corrupt("inconsistent WebP dimensions"));
            }
            decoder.set_memory_limit(l.allocation);
            let size = decoder
                .output_buffer_size()
                .ok_or_else(|| corrupt("WebP output size"))?;
            let mut out = buffer(size, l)?;
            decoder.read_image(&mut out).map_err(corrupt)?;
            (w, h, "image/webp", "webp")
        }
        _ => {
            return Err(invalid(
                "Select a JPEG, PNG, or WebP image. SVG and other formats are unsupported.",
            ));
        }
    };
    Ok(Validated {
        width,
        height,
        content_type,
        extension,
        sha256: Sha256::digest(bytes).to_vec(),
    })
}
pub fn filename(value: &str) -> String {
    let base = value.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(255).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || matches!(trimmed, "." | "..") {
        "upload".into()
    } else {
        trimmed.into()
    }
}
