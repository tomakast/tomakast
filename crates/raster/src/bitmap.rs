//! PNG and JPEG: the size their header declares, read before anything decodes, and a
//! decoder held to it.

use std::io::Cursor;

use tomakast_core::ImageFormat;

use crate::{Pixels, ThreadClock};

/// The largest width and height a widget image may have, in pixels.
pub const MAX_SIDE: u32 = 1024;

/// The format and the width and height an image declares in its header: PNG (`IHDR`) or
/// JPEG (the first start-of-frame marker). Anything else is refused.
pub fn header(bytes: &[u8]) -> Result<(ImageFormat, u32, u32), String> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.starts_with(PNG) {
        if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
            return Err("a PNG without its IHDR header".into());
        }
        let be = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        return Ok((ImageFormat::Png, be(16), be(20)));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let (width, height) = jpeg_size(bytes).ok_or("a JPEG without a readable frame header")?;
        return Ok((ImageFormat::Jpeg, width, height));
    }
    Err("only PNG and JPEG images are shown".into())
}

/// A PNG or JPEG decoded to straight RGBA: its header declares 1..=[`MAX_SIDE`] pixels on
/// both sides, it decodes within those limits, and its pixels are that size.
pub fn decode(bytes: &[u8]) -> Result<Pixels, String> {
    let clock = ThreadClock::start();
    let (format, width, height) = header(bytes)?;
    let side = 1..=MAX_SIDE;
    if !side.contains(&width) || !side.contains(&height) {
        return Err(format!(
            "declares {width}x{height} pixels, at most {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    let format = match format {
        ImageFormat::Png => image::ImageFormat::Png,
        _ => image::ImageFormat::Jpeg,
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| format!("{e}"))?;
    if (decoded.width(), decoded.height()) != (width, height) {
        return Err(format!(
            "declares {width}x{height} pixels but decodes to {}x{}",
            decoded.width(),
            decoded.height()
        ));
    }
    Ok(Pixels {
        width,
        height,
        rgba: decoded.into_rgba8().into_raw(),
        cpu: clock.elapsed(),
    })
}

/// Walks the JPEG markers up to the first start-of-frame (SOF0–SOF15 but DHT, JPG and DAC)
/// and returns its width and height.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        // Fill bytes (0xff) may precede a marker.
        while *bytes.get(at)? == 0xff && *bytes.get(at + 1)? == 0xff {
            at += 1;
        }
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        // Markers without a length.
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        // Start of scan or end of image before any frame header.
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        if len < 2 {
            return None;
        }
        let is_sof = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if is_sof {
            // Length (2), precision (1), height (2), width (2).
            let height = u16::from_be_bytes([*bytes.get(at + 3)?, *bytes.get(at + 4)?]);
            let width = u16::from_be_bytes([*bytes.get(at + 5)?, *bytes.get(at + 6)?]);
            return Some((u32::from(width), u32::from(height)));
        }
        at += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn bitmaps_over_1024_pixels_are_not_decoded() {
        let mut bytes = png(1, 1);
        // The IHDR width, 1025.
        bytes[16..20].copy_from_slice(&1025u32.to_be_bytes());
        let err = decode(&bytes).unwrap_err();
        assert!(err.contains("1025x1"), "{err}");

        let image = decode(&png(3, 2)).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(&image.rgba[..4], [255, 0, 0, 255]);
    }

    /// A JPEG whose frame header declares 65 500 x 65 500 pixels is refused before it
    /// decodes.
    #[test]
    fn a_jpeg_bomb_header_is_refused() {
        let mut bytes = Vec::new();
        image::RgbImage::new(8, 8)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
            .unwrap();
        let sof = bytes
            .windows(2)
            .position(|w| w == [0xff, 0xc0])
            .expect("a baseline frame header");
        // Marker (2), length (2), precision (1), height (2), width (2).
        bytes[sof + 5..sof + 7].copy_from_slice(&65_500u16.to_be_bytes());
        bytes[sof + 7..sof + 9].copy_from_slice(&65_500u16.to_be_bytes());
        let err = decode(&bytes).unwrap_err();
        assert!(err.contains("65500x65500"), "{err}");
    }
}
