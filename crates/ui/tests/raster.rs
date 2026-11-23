//! The raster helper process end to end: the launcher's client runs the real
//! `tomakast-raster` binary, with its limits, and stops it when a job runs late.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tomakast_core::ImageFit;
use tomakast_raster::{Client, Job};

/// What the launcher gives an SVG before it stops the helper: twice the raster deadline,
/// four times as long in a debug build.
fn stall_limit() -> Duration {
    Duration::from_millis(if cfg!(debug_assertions) { 128 } else { 32 })
}

fn client() -> Client {
    Client::new(env!("CARGO_BIN_EXE_tomakast-raster").into())
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn svg(body: &str) -> Job {
    let doc =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{body}</svg>"#);
    Job::Svg {
        bytes: Arc::from(doc.as_bytes()),
        device: (200, 100),
        fit: ImageFit::Fill,
    }
}

/// A red square on the left half.
fn square() -> Job {
    svg(r#"<rect width="50" height="100" fill="red"/>"#)
}

fn run(client: &mut Client, job: &Job) -> Result<tomakast_raster::Pixels, String> {
    client.run(job, stall_limit(), |_| {})
}

#[test]
fn the_helper_rasterizes_an_svg_and_decodes_a_png() {
    let mut client = client();

    let raster = run(&mut client, &square()).unwrap();
    assert_eq!((raster.width, raster.height), (200, 100));
    let at = |x: u32, y: u32| {
        let ix = ((y * raster.width + x) * 4) as usize;
        raster.rgba[ix..ix + 4].to_vec()
    };
    assert_eq!(at(10, 50), [255, 0, 0, 255]);
    assert_eq!(at(150, 50), [0, 0, 0, 0]);

    let bitmap = run(&mut client, &Job::Bitmap(Arc::from(png(3, 2)))).unwrap();
    assert_eq!((bitmap.width, bitmap.height), (3, 2));
    assert_eq!(&bitmap.rgba[..4], [255, 0, 0, 255]);
}

/// A job that runs past its limit stops the helper and is refused; the next job starts a
/// new helper.
#[test]
fn a_hung_job_is_stopped_at_the_limit_and_the_next_one_runs() {
    let mut client = client();
    run(&mut client, &square()).unwrap();

    let start = Instant::now();
    let err = run(&mut client, &Job::Sleep(Duration::from_secs(30))).unwrap_err();
    assert!(err.contains("still rasterizing"), "{err}");
    assert!(start.elapsed() < stall_limit() * 4, "{:?}", start.elapsed());

    assert!(run(&mut client, &square()).is_ok());
}

/// A panic is answered as a refusal; the helper goes on.
#[test]
fn a_panic_is_a_refusal() {
    let mut client = client();
    let err = run(&mut client, &Job::Panic).unwrap_err();
    assert!(err.contains("panicked"), "{err}");
    assert!(run(&mut client, &square()).is_ok());
}

/// A job that needs more memory than the helper may have stops the helper, not the app.
#[cfg(unix)]
#[test]
fn a_job_past_the_memory_limit_is_refused() {
    let mut client = client();
    let mib = (tomakast_raster::MEMORY_LIMIT >> 20) as u32 * 2;
    let err = client
        .run(&Job::Allocate(mib), Duration::from_secs(10), |_| {})
        .unwrap_err();
    assert!(err.contains("stopped"), "{err}");
    assert!(run(&mut client, &square()).is_ok());
}

/// The re-review's probes: a turbulence in a pattern scaled 50 times (refused by the
/// checks), and 2 000 translucent layers (stopped at the limit); each within about twice
/// the limit, the helper's start aside.
#[test]
fn costly_documents_are_refused_in_time() {
    let mut client = client();
    run(&mut client, &square()).unwrap();
    let pattern = svg(
        r#"<filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="10" height="10"><feTurbulence baseFrequency="0.05" numOctaves="10"/></filter><pattern id="p" patternUnits="userSpaceOnUse" width="10" height="10" patternTransform="scale(50)"><rect width="10" height="10" filter="url(#f)"/></pattern><rect width="100" height="100" fill="url(#p)"/>"#,
    );
    let layers = svg(
        &r#"<g opacity="0.5"><rect x="-200" y="-200" width="500" height="500"/></g>"#.repeat(2000),
    );
    for job in [pattern, layers] {
        let start = Instant::now();
        assert!(run(&mut client, &job).is_err());
        assert!(start.elapsed() < stall_limit() * 2, "{:?}", start.elapsed());
        // The helper may have been stopped: the next job starts it again.
        run(&mut client, &square()).unwrap();
    }
}

/// A file's thumbnail (file search) through the real helper: more bytes than a widget image
/// may carry, shrunk to fit. (Refusals are unit tests of `tomakast_raster::thumbnail`.)
#[test]
fn the_helper_makes_a_thumbnail_of_a_large_file() {
    let mut client = client();
    let mut bytes = Vec::new();
    image::RgbImage::from_pixel(1200, 600, image::Rgb([10, 200, 10]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    // Past the end of the JPEG: the file is over a widget image's cap, the picture is not.
    bytes.resize(
        tomakast_raster::protocol::MAX_JOB_BYTES as usize + (1 << 20),
        0,
    );
    let job = Job::Thumbnail {
        bytes: Arc::from(bytes),
        max_side: 256,
    };
    let thumb = client.run(&job, Duration::from_secs(5), |_| {}).unwrap();
    assert_eq!((thumb.width, thumb.height), (256, 128));
}
