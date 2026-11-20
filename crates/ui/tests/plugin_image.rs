//! The bounded loader for plugin image files (I-7 of the final review): no symbolic link, no
//! FIFO, at most 1 MiB, PNG or JPEG only, and the declared size is checked before decoding.

use std::path::{Path, PathBuf};

use gpui_kit::ImageFormat;
use tomakast_ui::plugin_image::{self, MAX_PLUGIN_IMAGE_BYTES, MAX_PLUGIN_IMAGE_SIDE};

/// The kitchen-sink plugin's 1x1 PNG.
fn tiny_png() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/kitchen-sink/icons/echo.png");
    std::fs::read(path).unwrap()
}

/// `tiny_png` with the width and height in its IHDR replaced (the CRC is not fixed: the
/// header is all the loader reads).
fn png_declaring(width: u32, height: u32) -> Vec<u8> {
    let mut png = tiny_png();
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    png
}

/// A JPEG header: SOI, an APP0 segment, then SOF0 declaring `width` x `height`.
fn jpeg_declaring(width: u16, height: u16) -> Vec<u8> {
    let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
    jpeg.extend([0xff, 0xc0, 0x00, 0x0b, 0x08]);
    jpeg.extend(height.to_be_bytes());
    jpeg.extend(width.to_be_bytes());
    jpeg.extend([0x01, 0x01, 0x11, 0x00]);
    jpeg
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_small_png_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "ok.png", &tiny_png());
    let image = plugin_image::load(&path).unwrap();
    assert_eq!(image.format, ImageFormat::Png);
    let edge = write(
        tmp.path(),
        "edge.png",
        &png_declaring(MAX_PLUGIN_IMAGE_SIDE, MAX_PLUGIN_IMAGE_SIDE),
    );
    assert!(plugin_image::load(&edge).is_ok());
}

#[test]
fn large_declared_dimensions_are_refused_before_decoding() {
    let tmp = tempfile::tempdir().unwrap();
    for (w, h) in [
        (MAX_PLUGIN_IMAGE_SIDE + 1, 1),
        (1, MAX_PLUGIN_IMAGE_SIDE + 1),
        (100_000, 100_000),
        (u32::MAX, u32::MAX),
        (0, 1),
    ] {
        // A 70-byte file that, decoded, would ask for w*h*4 bytes.
        let path = write(tmp.path(), "big.png", &png_declaring(w, h));
        let problem = plugin_image::load(&path).unwrap_err();
        assert!(problem.contains("declares"), "{w}x{h}: {problem}");
    }
    assert_eq!(
        plugin_image::dimensions(&jpeg_declaring(640, 480)).unwrap(),
        (ImageFormat::Jpeg, 640, 480)
    );
    let path = write(tmp.path(), "big.jpg", &jpeg_declaring(60_000, 60_000));
    assert!(plugin_image::load(&path).unwrap_err().contains("declares"));
}

#[test]
fn other_formats_and_broken_headers_are_refused() {
    for bytes in [
        &b"GIF89a\x01\x00\x01\x00"[..],
        b"RIFF\x00\x00\x00\x00WEBPVP8 ",
        b"<svg xmlns='http://www.w3.org/2000/svg'/>",
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff\xda\x00\x02",
        b"",
    ] {
        assert!(plugin_image::dimensions(bytes).is_err(), "{bytes:?}");
    }
}

#[test]
fn files_over_1_mib_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut png = tiny_png();
    png.resize(MAX_PLUGIN_IMAGE_BYTES as usize + 1, 0);
    let path = write(tmp.path(), "huge.png", &png);
    assert!(plugin_image::load(&path).unwrap_err().contains("larger"));
    png.truncate(MAX_PLUGIN_IMAGE_BYTES as usize);
    let path = write(tmp.path(), "max.png", &png);
    assert!(plugin_image::load(&path).is_ok());
}

#[cfg(unix)]
#[test]
fn links_and_fifos_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let target = write(tmp.path(), "ok.png", &tiny_png());
    let link = tmp.path().join("link.png");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(
        plugin_image::load(&link).is_err(),
        "followed a symbolic link"
    );
    let fifo = tmp.path().join("fifo.png");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    // Returns at once (O_NONBLOCK) instead of waiting for a writer.
    assert!(plugin_image::load(&fifo).unwrap_err().contains("regular"));
}
