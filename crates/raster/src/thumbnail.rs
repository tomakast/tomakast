//! Thumbnails of files: a PNG, JPEG, WebP or GIF decoded under limits and shrunk to fit a
//! square, for the file search grid. The bytes come from any file on the disk (a download,
//! say), so this runs in the helper too.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::protocol::MAX_REPLY_SIDE;
use crate::{Pixels, ThreadClock};

/// Images over this many pixels are refused before they decode.
pub const MAX_PIXELS: u64 = 100_000_000;

/// The most the decoder may allocate: a 24-megapixel photo with room to spare, under the
/// helper's address space ([`crate::MEMORY_LIMIT`]) with the file's bytes beside it.
pub const MAX_ALLOC: u64 = 320 << 20;

/// A thumbnail of the image in `bytes` (PNG, JPEG, WebP or GIF; a GIF's first frame), at
/// most `max_side` on its longer side and never larger than the image, turned as its EXIF
/// orientation says. The header's width and height are checked before anything decodes.
pub fn thumbnail(bytes: &[u8], max_side: u32) -> Result<Pixels, String> {
    let clock = ThreadClock::start();
    if !(1..=MAX_REPLY_SIDE).contains(&max_side) {
        return Err(format!("a thumbnail side of {max_side} pixels"));
    }
    let (mut decoder, limits) = decoder(bytes, MAX_ALLOC)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "declares {width}x{height} pixels, over {} megapixels",
            MAX_PIXELS / 1_000_000
        ));
    }
    // The decoders take `max_alloc` only partly, so the decoded image, the largest
    // allocation, is checked here.
    if decoder.total_bytes() > MAX_ALLOC {
        return Err(format!(
            "{width}x{height} pixels need {} bytes decoded",
            decoder.total_bytes()
        ));
    }
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    // Shrunk before it is turned: a quarter turn copies the whole image. The box is square,
    // so the result is the same.
    if image.width() > max_side || image.height() > max_side {
        image = image.thumbnail(max_side, max_side);
    }
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    let rgba = image.into_rgba8();
    Ok(Pixels {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        cpu: clock.elapsed(),
    })
}

/// The decoder of `bytes`, its header read under `max_alloc`, and the limits it was made
/// with. The limits are given before the decoder is made: the PNG decoder takes them only
/// then, and it inflates chunks such as the colour profile while reading the header. No
/// width or height cap is set, so a huge image is refused by [`thumbnail`], saying why.
fn decoder(bytes: &[u8], max_alloc: u64) -> Result<(Box<dyn ImageDecoder + '_>, Limits), String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    match reader.format() {
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Gif) => {}
        _ => return Err("only PNG, JPEG, WebP and GIF images get thumbnails".into()),
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(max_alloc);
    reader.limits(limits.clone());
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    Ok((Box::new(decoder), limits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Job, run};
    use std::sync::Arc;

    fn encoded(image: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    fn rgb(w: u32, h: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])))
    }

    fn job(bytes: Vec<u8>, max_side: u32) -> Job {
        Job::Thumbnail {
            bytes: Arc::from(bytes),
            max_side,
        }
    }

    #[test]
    fn thumbnails_fit_the_side_and_never_grow() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
            image::ImageFormat::Gif,
        ] {
            let image = if format == image::ImageFormat::Gif {
                image::DynamicImage::ImageRgba8(rgb(1000, 500).into_rgba8())
            } else {
                rgb(1000, 500)
            };
            let wide = run(&job(encoded(image, format), 256)).unwrap();
            assert_eq!((wide.width, wide.height), (256, 128), "{format:?}");
            assert_eq!(wide.rgba.len(), 256 * 128 * 4);
        }
        let tall = run(&job(encoded(rgb(300, 600), image::ImageFormat::Png), 256)).unwrap();
        assert_eq!((tall.width, tall.height), (128, 256));
        let small = run(&job(encoded(rgb(40, 20), image::ImageFormat::Png), 256)).unwrap();
        assert_eq!((small.width, small.height), (40, 20));
        assert_eq!(&small.rgba[..4], [200, 30, 30, 255]);
    }

    #[test]
    fn images_over_100_megapixels_are_refused() {
        // A real PNG whose header says 20000x20000; the body is never decoded.
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 20_000, 20_000);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_chunk(
                png::chunk::IDAT,
                &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
            )
            .unwrap();
        drop(writer);
        let err = run(&job(bytes, 256)).unwrap_err();
        assert!(err.contains("megapixels"), "{err}");
    }

    #[test]
    fn what_is_not_an_image_is_refused() {
        for bytes in [
            &b"%PDF-1.4 not an image"[..],
            b"",
            b"\x89PNG\r\n\x1a\nbroken",
        ] {
            assert!(run(&job(bytes.to_vec(), 256)).is_err(), "{bytes:?}");
        }
        // An image type that gets no thumbnail.
        let mut bmp = b"BM".to_vec();
        bmp.resize(64, 0);
        let err = run(&job(bmp, 256)).unwrap_err();
        assert!(err.contains("PNG, JPEG, WebP and GIF"), "{err}");
        // A side the reply cannot carry.
        let png = encoded(rgb(4, 4), image::ImageFormat::Png);
        assert!(run(&job(png.clone(), 0)).is_err());
        assert!(run(&job(png, crate::protocol::MAX_REPLY_SIDE + 1)).is_err());
    }

    #[test]
    fn a_colour_profile_bomb_is_not_inflated() {
        // A 4x4 image whose colour profile is 4 MiB of zeros: a few KiB compressed.
        let mut info = png::Info::with_size(4, 4);
        info.color_type = png::ColorType::Rgb;
        info.bit_depth = png::BitDepth::Eight;
        info.icc_profile = Some(vec![0u8; 4 << 20].into());
        let mut bytes = Vec::new();
        let mut writer = png::Encoder::with_info(&mut bytes, info)
            .unwrap()
            .write_header()
            .unwrap();
        writer.write_image_data(&[7u8; 4 * 4 * 3]).unwrap();
        writer.finish().unwrap();
        assert!(bytes.len() < 64 * 1024);

        // Under a 1 MiB limit, reading the header does not inflate the profile (`png` stops
        // at the limit and drops it); with room, it does: the limit is what kept it out.
        {
            let (mut tight, _) = decoder(&bytes, 1 << 20).unwrap();
            assert_eq!(tight.icc_profile().unwrap().map(|p| p.len()), None);
            let (mut roomy, _) = decoder(&bytes, 64 << 20).unwrap();
            assert_eq!(roomy.icc_profile().unwrap().map(|p| p.len()), Some(4 << 20));
        }
        // The picture itself still decodes.
        let thumb = run(&job(bytes, 256)).unwrap();
        assert_eq!((thumb.width, thumb.height), (4, 4));
    }

    #[test]
    fn a_panic_is_an_error() {
        let err = run(&Job::Panic).unwrap_err();
        assert!(err.contains("panicked"), "{err}");
    }
}
