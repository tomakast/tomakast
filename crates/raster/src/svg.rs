//! Plugin SVG: parsed with no external reference, checked against a cost model before
//! anything renders, and rasterized.
//!
//! - The XML is parsed as the host parses it (no DTD, at most [`MAX_SVG_NODES`] nodes),
//!   then converted by usvg with [`svg_options`]: no `href` string is resolved, `data:`
//!   images are kept only as PNG or JPEG of at most 1024x1024 pixels, and a character the
//!   named fonts lack is not drawn (no walk over every system face).
//! - [`check_tree`] counts the parsed document, where `<use>` copies are real nodes:
//!   [`MAX_SVG_ELEMENTS`] nodes, [`MAX_SVG_FILTERS`] filter primitives,
//!   [`MAX_SVG_FILTER_APPLICATIONS`] primitives applied, at most [`MAX_FILTER_COST`] of
//!   filter work, [`MAX_SVG_IMAGES`] raster images
//!   of [`MAX_SVG_IMAGE_PIXELS`] in all, turbulence octaves, morphology radius (in device
//!   pixels), convolution order, and filter regions at most [`MAX_FILTER_REGION`] times the
//!   raster.
//! - A filter inside a pattern, mask, clip path or filter image is refused: its transform
//!   there is not the one it draws at, so neither its cost nor its radius can be told.
//! - [`rasterize`] times itself in the thread's CPU time.
//!
//! Static caps cannot bound everything resvg may spend (layers of opacity, pattern tiles):
//! the helper process that runs this has limits of its own, and is stopped past its time.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use roxmltree::{Document, ParsingOptions};
use tomakast_core::{
    ImageFit, MAX_SVG_CONVOLVE_ORDER, MAX_SVG_ELEMENTS, MAX_SVG_FILTER_APPLICATIONS,
    MAX_SVG_FILTERS, MAX_SVG_IMAGE_PIXELS, MAX_SVG_IMAGES, MAX_SVG_MORPHOLOGY_RADIUS,
    MAX_SVG_NODES, MAX_SVG_OCTAVES,
};
use usvg::filter::Kind;
use usvg::fontdb;

use crate::bitmap::{self, MAX_SIDE};
use crate::{Device, Pixels};

/// How many times the raster's width and height a filter region may reach, in device pixels.
pub const MAX_FILTER_REGION: f32 = 2.;

/// The most filter work a document may ask, refused before it renders: the sum, over every
/// filter primitive applied, of its filter region in device pixels times the primitive's
/// weight ([`filter_weight`]).
///
/// Calibrated in a release build (2026-11-20, the development machine), rendering each kind
/// over a 718 x 240 region with resvg 0.46: the costliest work per unit is `feTurbulence` of
/// one octave (~75 ns), then `feGaussianBlur` of a small deviation (~67 ns); every other
/// kind costs 1–20 ns, morphology and convolution under 5 ns. At 160 000 units a turbulence
/// document took 14.5 ms in all (parse, check, render), too close to the 16 ms deadline;
/// at 120 000 the worst documents take 11.0 ms (turbulence) and 8.8 ms (blur)
/// (`a_document_at_the_budget_renders_in_time`).
pub const MAX_FILTER_COST: f64 = 120_000.;

/// The system fonts, for SVG text: loaded once, as the helper starts.
static FONTS: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = fontdb::Database::new();
    fonts.load_system_fonts();
    Arc::new(fonts)
});

/// Loads the system fonts, if not yet.
pub fn load_fonts() {
    LazyLock::force(&FONTS);
}

/// How SVG documents are converted: no `href` string is ever resolved (no file, no URL),
/// `data:` images are kept only as PNG or JPEG of at most 1024x1024 pixels, nothing is
/// relative to a directory, and text uses the fonts of `fontdb` it names, with no fallback.
pub fn svg_options(fontdb: Arc<fontdb::Database>) -> usvg::Options<'static> {
    usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|mime: &str, data: Arc<Vec<u8>>, _: &usvg::Options| {
                let kind = match mime {
                    "image/png" => usvg::ImageKind::PNG,
                    "image/jpeg" => usvg::ImageKind::JPEG,
                    _ => return None,
                };
                let (width, height) = image_size(&data)?;
                let side = 1..=MAX_SIDE;
                (side.contains(&width) && side.contains(&height)).then(|| kind(data))
            }),
            resolve_string: Box::new(|_: &str, _: &usvg::Options| None),
        },
        font_resolver: usvg::FontResolver {
            select_font: usvg::FontResolver::default_font_selector(),
            // The default walks every system face for each character the named fonts lack.
            select_fallback: Box::new(|_, _, _| None),
        },
        fontdb,
        ..usvg::Options::default()
    }
}

/// The width and height a PNG or JPEG declares.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    bitmap::header(bytes)
        .ok()
        .map(|(_, width, height)| (width, height))
}

/// `bytes`, an SVG document, parsed as the host parses it and converted with
/// [`svg_options`] over `fonts`.
pub fn parse(bytes: &[u8], fonts: Arc<fontdb::Database>) -> Result<usvg::Tree, String> {
    // Text only: usvg's `from_data` would also take gzip.
    let text = std::str::from_utf8(bytes).map_err(|_| "an SVG that is not UTF-8".to_string())?;
    let options = ParsingOptions {
        nodes_limit: MAX_SVG_NODES as u32,
        // DTDs stay refused (the default).
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(text, options).map_err(|e| format!("{e}"))?;
    usvg::Tree::from_xmltree(&doc, &svg_options(fonts)).map_err(|e| format!("{e}"))
}

/// The transform that places `tree` on a raster of `device` pixels with `fit`: `Fill`
/// stretches its size onto the raster; `Contain` and `Cover` scale it evenly, centred, so
/// that a `Cover` raster holds only the part that shows.
pub fn placement(tree: &usvg::Tree, device: Device, fit: ImageFit) -> usvg::Transform {
    let size = tree.size();
    let (width, height) = (device.0 as f32, device.1 as f32);
    let (sx, sy) = (width / size.width(), height / size.height());
    let scale = match fit {
        ImageFit::Fill => return usvg::Transform::from_scale(sx, sy),
        ImageFit::Contain => sx.min(sy),
        ImageFit::Cover => sx.max(sy),
    };
    usvg::Transform::from_row(
        scale,
        0.,
        0.,
        scale,
        (width - size.width() * scale) / 2.,
        (height - size.height() * scale) / 2.,
    )
}

/// Refuses a document, placed on a raster of `device` pixels by `to_device`, that breaks
/// the cost model (see the module docs). The UI counts usvg's nodes against
/// [`MAX_SVG_ELEMENTS`]: the parsed document, where every `<use>` is copied.
pub fn check_tree(
    tree: &usvg::Tree,
    device: Device,
    to_device: usvg::Transform,
) -> Result<(), String> {
    let primitives: usize = tree
        .filters()
        .iter()
        .map(|filter| filter.primitives().len())
        .sum();
    if primitives > MAX_SVG_FILTERS {
        return Err(format!(
            "{primitives} filter primitives, at most {MAX_SVG_FILTERS}"
        ));
    }
    let mut walk = Walk {
        nodes: 0,
        applied: 0,
        cost: 0.,
        images: 0,
        pixels: 0,
        to_device,
        max: (
            MAX_FILTER_REGION * device.0 as f32,
            MAX_FILTER_REGION * device.1 as f32,
        ),
        subroots: 0,
        problem: None,
    };
    walk.group(tree.root());
    walk.problem.map_or(Ok(()), Err)
}

/// Walks a tree, its clip paths, masks, patterns and filter images too, and stops at the
/// first thing the cost model refuses.
struct Walk {
    nodes: usize,
    applied: usize,
    /// Filter work so far ([`MAX_FILTER_COST`]).
    cost: f64,
    images: usize,
    pixels: u64,
    to_device: usvg::Transform,
    max: (f32, f32),
    /// How deep in patterns, masks, clip paths and filter images the walk is.
    subroots: usize,
    problem: Option<String>,
}

/// A morphology's radii in device pixels under `transform`: times the lengths of the
/// transformed unit axes.
fn device_radius(morphology: &usvg::filter::Morphology, transform: usvg::Transform) -> (f32, f32) {
    let sx = transform.sx.hypot(transform.ky);
    let sy = transform.kx.hypot(transform.sy);
    (
        morphology.radius_x().get() * sx,
        morphology.radius_y().get() * sy,
    )
}

/// The work of one filter primitive per device pixel of its region, roughly: a morphology
/// looks at (2rx + 1)(2ry + 1) pixels (resvg's is naive), a turbulence adds an octave at a
/// time, a convolution reads its whole kernel, a blur makes three passes, a lighting reads
/// a neighbourhood and lights it, a merge composites each of its inputs; everything else
/// touches each pixel once.
pub fn filter_weight(kind: &Kind, transform: usvg::Transform) -> f64 {
    match kind {
        Kind::Morphology(morphology) => {
            let (rx, ry) = device_radius(morphology, transform);
            f64::from(2. * rx + 1.) * f64::from(2. * ry + 1.)
        }
        Kind::Turbulence(turbulence) => f64::from(turbulence.num_octaves().max(1)),
        Kind::ConvolveMatrix(convolve) => {
            let matrix = convolve.matrix();
            f64::from(matrix.columns()) * f64::from(matrix.rows())
        }
        Kind::GaussianBlur(_) | Kind::DropShadow(_) => 3.,
        // It composites every input in turn.
        Kind::Merge(merge) => merge.inputs().len().max(1) as f64,
        Kind::DiffuseLighting(_) | Kind::SpecularLighting(_) => 4.,
        _ => 1.,
    }
}

impl Walk {
    fn refuse(&mut self, problem: String) {
        self.problem.get_or_insert(problem);
    }

    fn group(&mut self, group: &usvg::Group) {
        if self.problem.is_some() {
            return;
        }
        if self.subroots > 0 && !group.filters().is_empty() {
            return self
                .refuse("a filter inside a pattern, mask, clip path or filter image".into());
        }
        let transform = self.to_device.pre_concat(group.abs_transform());
        if let Some(region) = group.filters_bounding_box() {
            let wide = region
                .transform(transform)
                .is_none_or(|r| r.width() > self.max.0 || r.height() > self.max.1);
            if wide {
                self.refuse(format!(
                    "a filter region past {MAX_FILTER_REGION} times the raster's size"
                ));
            }
        }
        for filter in group.filters() {
            self.applied += filter.primitives().len();
            if self.applied > MAX_SVG_FILTER_APPLICATIONS {
                self.refuse(format!(
                    "more than {MAX_SVG_FILTER_APPLICATIONS} filter primitives applied"
                ));
            }
            // Its region in device pixels: the box around it, transformed.
            let area = filter
                .rect()
                .transform(transform)
                .map_or(f64::INFINITY, |r| {
                    f64::from(r.width()) * f64::from(r.height())
                });
            for primitive in filter.primitives() {
                self.primitive(primitive.kind(), transform);
                self.cost += area * filter_weight(primitive.kind(), transform);
            }
            if self.cost.is_nan() || self.cost > MAX_FILTER_COST {
                self.refuse(format!(
                    "filters that cost {:.0} units, at most {MAX_FILTER_COST}",
                    self.cost
                ));
            }
        }
        for node in group.children() {
            if self.problem.is_some() {
                return;
            }
            self.nodes += 1;
            if self.nodes > MAX_SVG_ELEMENTS {
                self.refuse(format!("more than {MAX_SVG_ELEMENTS} elements once parsed"));
            }
            match node {
                usvg::Node::Group(child) => self.group(child),
                usvg::Node::Image(image) => self.image(image),
                _ => {}
            }
            self.subroots += 1;
            node.subroots(|root| self.group(root));
            self.subroots -= 1;
        }
    }

    fn primitive(&mut self, kind: &Kind, transform: usvg::Transform) {
        match kind {
            Kind::Turbulence(turbulence) if turbulence.num_octaves() > MAX_SVG_OCTAVES => {
                self.refuse(format!(
                    "feTurbulence of {} octaves, at most {MAX_SVG_OCTAVES}",
                    turbulence.num_octaves()
                ));
            }
            Kind::Morphology(morphology) => {
                let (rx, ry) = device_radius(morphology, transform);
                let radius = rx.max(ry);
                if radius.is_nan() || radius > MAX_SVG_MORPHOLOGY_RADIUS {
                    self.refuse(format!(
                        "feMorphology of radius {radius} device pixels, at most \
                         {MAX_SVG_MORPHOLOGY_RADIUS}"
                    ));
                }
            }
            Kind::ConvolveMatrix(convolve) => {
                let matrix = convolve.matrix();
                if matrix.columns().max(matrix.rows()) > MAX_SVG_CONVOLVE_ORDER {
                    self.refuse(format!(
                        "feConvolveMatrix of order {}x{}, at most {MAX_SVG_CONVOLVE_ORDER}",
                        matrix.columns(),
                        matrix.rows()
                    ));
                }
            }
            _ => {}
        }
    }

    fn image(&mut self, image: &usvg::Image) {
        let data = match image.kind() {
            usvg::ImageKind::PNG(data) | usvg::ImageKind::JPEG(data) => data,
            // The resolver keeps PNG and JPEG only.
            _ => return self.refuse("an image that is not PNG or JPEG".into()),
        };
        self.images += 1;
        let (width, height) = image_size(data).unwrap_or((0, 0));
        self.pixels += u64::from(width) * u64::from(height);
        if self.images > MAX_SVG_IMAGES {
            self.refuse(format!("more than {MAX_SVG_IMAGES} images drawn"));
        } else if self.pixels > MAX_SVG_IMAGE_PIXELS {
            self.refuse(format!(
                "images of more than {MAX_SVG_IMAGE_PIXELS} pixels in all"
            ));
        }
    }
}

/// `bytes`, an SVG document, parsed, checked ([`check_tree`]) and rasterized at `device`
/// with `fit` ([`placement`]), as straight RGBA, with the thread's CPU time it took. The
/// clock starts once the system fonts are loaded.
pub fn rasterize(bytes: &[u8], device: Device, fit: ImageFit) -> Result<Pixels, String> {
    let fonts = FONTS.clone();
    let clock = ThreadClock::start();
    let tree = parse(bytes, fonts)?;
    let transform = placement(&tree, device, fit);
    check_tree(&tree, device, transform)?;
    let (width, height) = device;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("an empty raster".to_string())?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(Pixels {
        width,
        height,
        rgba,
        cpu: clock.elapsed(),
    })
}

/// The CPU time of the calling thread since [`ThreadClock::start`]; the wall time where
/// that clock is missing.
pub(crate) struct ThreadClock(Duration);

impl ThreadClock {
    pub(crate) fn start() -> ThreadClock {
        ThreadClock(thread_time())
    }

    pub(crate) fn elapsed(&self) -> Duration {
        thread_time().saturating_sub(self.0)
    }
}

/// Wall time since the first call, for platforms without a thread clock.
fn wall_time() -> Duration {
    static EPOCH: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    EPOCH.elapsed()
}

#[cfg(unix)]
fn thread_time() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes one `timespec` through a valid pointer to ours.
    let read = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    if read != 0 {
        return wall_time();
    }
    Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

#[cfg(not(unix))]
fn thread_time() -> Duration {
    wall_time()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The deadline the launcher gives a raster in a debug build, four times its 16 ms.
    fn raster_deadline() -> Duration {
        Duration::from_millis(if cfg!(debug_assertions) { 64 } else { 16 })
    }

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    fn base64(bytes: &[u8]) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (ix, &b)| n | u32::from(b) << (16 - 8 * ix));
            for ix in 0..4 {
                if ix <= chunk.len() {
                    out.push(ABC[(n >> (18 - 6 * ix) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn data_png(width: u32, height: u32) -> String {
        format!("data:image/png;base64,{}", base64(&png(width, height)))
    }

    fn doc(body: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">{body}</svg>"#
        )
    }

    fn tree(svg: &str) -> usvg::Tree {
        parse(svg.as_bytes(), Arc::new(fontdb::Database::new())).unwrap()
    }

    fn device(width: u32, height: u32) -> Device {
        (width, height)
    }

    fn check(svg: &str) -> Result<(), String> {
        let tree = tree(svg);
        let at = device(200, 200);
        check_tree(&tree, at, placement(&tree, at, ImageFit::Fill))
    }

    /// Raster images in the tree, its subroots (filter images, patterns) included.
    fn images_in(tree: &usvg::Tree) -> usize {
        fn count(group: &usvg::Group) -> usize {
            group
                .children()
                .iter()
                .map(|node| {
                    let mut n = match node {
                        usvg::Node::Image(_) => 1,
                        usvg::Node::Group(group) => count(group),
                        _ => 0,
                    };
                    node.subroots(|root| n += count(root));
                    n
                })
                .sum()
        }
        count(tree.root())
    }

    fn png_file() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        std::fs::write(&path, png(4, 4)).unwrap();
        let path = path.display().to_string();
        (dir, path)
    }

    #[test]
    fn string_hrefs_are_never_resolved() {
        let (_dir, path) = png_file();
        for href in [path.clone(), format!("file://{path}")] {
            for attr in ["href", "xlink:href"] {
                let svg = doc(&format!(
                    r#"<image {attr}="{href}" width="10" height="10"/>"#
                ));
                assert_eq!(images_in(&tree(&svg)), 0, "{attr}={href}");
            }
        }
    }

    #[test]
    fn an_fe_image_never_reads_a_file() {
        let (_dir, path) = png_file();
        let svg = doc(&format!(
            r#"<filter id="f"><feImage href="{path}"/></filter>
            <rect width="10" height="10" filter="url(#f)"/>"#
        ));
        assert_eq!(images_in(&tree(&svg)), 0);
        // The same filter with a data: PNG keeps its image: the count sees it.
        let svg = doc(&format!(
            r#"<filter id="f"><feImage href="{}"/></filter>
            <rect width="10" height="10" filter="url(#f)"/>"#,
            data_png(4, 4)
        ));
        assert_eq!(images_in(&tree(&svg)), 1);
    }

    #[test]
    fn data_png_is_kept_data_svg_is_refused() {
        let image = |href: &str| doc(&format!(r#"<image href="{href}" width="10" height="10"/>"#));
        assert_eq!(images_in(&tree(&image(&data_png(4, 4)))), 1);

        let inner = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;
        let svg = format!("data:image/svg+xml;base64,{}", base64(inner.as_bytes()));
        assert_eq!(images_in(&tree(&image(&svg))), 0);
        // Nor a PNG of more than 1024 pixels on a side.
        assert_eq!(images_in(&tree(&image(&data_png(1025, 1)))), 0);
    }

    #[test]
    fn a_dtd_is_refused() {
        let svg = format!(
            r#"<!DOCTYPE svg [<!ENTITY x "x">]>{}"#,
            doc("<text>&x;</text>")
        );
        assert!(parse(svg.as_bytes(), Arc::new(fontdb::Database::new())).is_err());
    }

    #[test]
    fn a_huge_filter_region_is_refused() {
        let svg = |x: &str, width: &str| {
            doc(&format!(
                r#"<filter id="f" x="{x}" width="{width}"><feFlood/></filter>
                <rect width="100" height="100" fill="red" filter="url(#f)"/>"#
            ))
        };
        let err = check(&svg("-1000%", "3000%")).unwrap_err();
        assert!(err.contains("filter region"), "{err}");
        assert_eq!(check(&svg("-10%", "120%")), Ok(()));
    }

    #[test]
    fn a_use_bomb_is_refused() {
        let mut body = String::from(r#"<g id="l0"><rect width="1" height="1"/></g>"#);
        for level in 1..8 {
            let prev = level - 1;
            body.push_str(&format!(
                r##"<g id="l{level}">{}</g>"##,
                format!(r##"<use href="#l{prev}"/>"##).repeat(4)
            ));
        }
        let err = check(&doc(&body)).unwrap_err();
        assert!(err.contains("elements"), "{err}");
    }

    /// Checked in less than the deadline: refused before anything renders.
    fn refused_fast(svg: &str, problem: &str) {
        let start = Instant::now();
        let err = check(svg).unwrap_err();
        assert!(err.contains(problem), "{err}");
        assert!(start.elapsed() < raster_deadline(), "{:?}", start.elapsed());
    }

    fn turbulence(octaves: u32) -> String {
        doc(&format!(
            r#"<filter id="f"><feTurbulence baseFrequency="0.05" numOctaves="{octaves}"/></filter>
            <rect width="10" height="10" filter="url(#f)"/>"#
        ))
    }

    /// The review's probes: turbulence of 20 000 and 200 octaves, and 1 000 rects sharing
    /// one filter of 32 blurs, each of which took seconds to minutes to render.
    #[test]
    fn the_costly_documents_are_refused_before_rendering() {
        refused_fast(&turbulence(20_000), "octaves");
        refused_fast(&turbulence(200), "octaves");
        let blurs = r#"<feGaussianBlur stdDeviation="3"/>"#.repeat(32);
        let rects: String = (0..1000)
            .map(|ix| {
                format!(
                    r#"<rect x="{}" y="{}" width="5" height="5" filter="url(#f)"/>"#,
                    ix % 100,
                    ix / 100 * 10
                )
            })
            .collect();
        let shared = doc(&format!(
            r#"<filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="5" height="5">{blurs}</filter>{rects}"#
        ));
        refused_fast(&shared, "applied");
    }

    #[test]
    fn filter_costs_stop_at_their_caps() {
        assert_eq!(check(&turbulence(10)), Ok(()));
        assert!(check(&turbulence(11)).is_err());

        // On a 100 x 100 document at 200 x 200: a radius of 16 is 32 device pixels.
        // On a region of 2 x 2 device pixels, inside the cost budget.
        let morphology = |radius: f32| {
            doc(&format!(
                r#"<filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="1" height="1"><feMorphology radius="{radius}"/></filter>
                <rect width="1" height="1" filter="url(#f)"/>"#
            ))
        };
        assert_eq!(check(&morphology(16.)), Ok(()));
        let err = check(&morphology(16.5)).unwrap_err();
        assert!(err.contains("feMorphology"), "{err}");

        let convolve = |order: u32| {
            let kernel = vec!["1"; (order * order) as usize].join(" ");
            doc(&format!(
                r#"<filter id="f"><feConvolveMatrix order="{order}" kernelMatrix="{kernel}"/></filter>
                <rect width="10" height="10" filter="url(#f)"/>"#
            ))
        };
        assert_eq!(check(&convolve(9)), Ok(()));
        let err = check(&convolve(10)).unwrap_err();
        assert!(err.contains("feConvolveMatrix"), "{err}");
    }

    /// One image `<use>`d many times is drawn, and decoded, as many times.
    #[test]
    fn images_drawn_are_counted_with_their_uses() {
        let image = |width: u32, uses: usize| {
            doc(&format!(
                r##"<defs><image id="i" width="10" height="10" href="{}"/></defs>{}"##,
                data_png(width, width),
                r##"<use href="#i"/>"##.repeat(uses)
            ))
        };
        assert_eq!(check(&image(4, MAX_SVG_IMAGES)), Ok(()));
        let err = check(&image(4, MAX_SVG_IMAGES + 1)).unwrap_err();
        assert!(err.contains("images drawn"), "{err}");
        // Four 1024 x 1024 images are the pixel cap; five are past it.
        assert_eq!(check(&image(1024, 4)), Ok(()));
        let err = check(&image(1024, 5)).unwrap_err();
        assert!(err.contains("pixels"), "{err}");
    }

    /// `count` copies of `primitive` in one filter whose region is just under
    /// twice a 1436 x 240 raster, on a document of that size.
    fn wide_filter(primitive: &str, count: usize) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1436" height="240"><filter id="f" filterUnits="userSpaceOnUse" x="-700" y="-110" width="2836" height="460">{}</filter><rect width="1436" height="240" fill="red" filter="url(#f)"/></svg>"#,
            primitive.repeat(count)
        )
    }

    /// The re-review's probes, each within every other cap: from 1.6 s (32 blurs) to 135 s
    /// (32 morphologies of radius 32) of CPU in a release build.
    #[test]
    fn filters_past_the_cost_budget_are_refused_before_rendering() {
        let kernel = vec!["1"; 81].join(" ");
        let convolve = format!(r#"<feConvolveMatrix order="9" kernelMatrix="{kernel}"/>"#);
        let documents = [
            wide_filter(r#"<feMorphology operator="dilate" radius="32"/>"#, 32),
            wide_filter(r#"<feMorphology operator="dilate" radius="32"/>"#, 1),
            wide_filter(
                r#"<feTurbulence baseFrequency="0.05" numOctaves="10"/>"#,
                32,
            ),
            wide_filter(&convolve, 32),
            wide_filter(
                r#"<feDiffuseLighting><fePointLight x="10" y="10" z="20"/></feDiffuseLighting>"#,
                32,
            ),
            wide_filter(r#"<feGaussianBlur stdDeviation="3"/>"#, 32),
        ];
        let at = device(1436, 240);
        for svg in documents {
            let tree = tree(&svg);
            let start = Instant::now();
            let err = check_tree(&tree, at, placement(&tree, at, ImageFit::Fill)).unwrap_err();
            assert!(err.contains("cost"), "{err}");
            assert!(start.elapsed() < raster_deadline(), "{:?}", start.elapsed());
        }
    }

    /// The documents at the budget of its two costliest kinds per unit: a turbulence of one
    /// octave and a blur of a small deviation, each over the most region the budget allows.
    /// They are accepted, and in a release build they rasterize within the deadline (a
    /// debug build only checks that they do rasterize).
    #[test]
    fn a_document_at_the_budget_renders_in_time() {
        let region = |side: (u32, u32), primitive: &str| {
            let (w, h) = side;
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}">{primitive}</filter><rect width="{w}" height="{h}" fill="red" filter="url(#f)"/></svg>"#
            )
        };
        let documents = [
            (
                (346, 346),
                region(
                    (346, 346),
                    r#"<feTurbulence baseFrequency="0.05" numOctaves="1"/>"#,
                ),
            ),
            (
                (199, 200),
                region((199, 200), r#"<feGaussianBlur stdDeviation="1"/>"#),
            ),
        ];
        for ((w, h), svg) in documents {
            let at = device(w, h);
            let parsed = tree(&svg);
            assert_eq!(
                check_tree(&parsed, at, placement(&parsed, at, ImageFit::Fill)),
                Ok(())
            );
            let raster = rasterize(svg.as_bytes(), at, ImageFit::Fill).unwrap();
            println!("{w}x{h}: {:?}", raster.cpu);
            if !cfg!(debug_assertions) {
                assert!(raster.cpu < raster_deadline(), "{:?}", raster.cpu);
            }
        }
    }

    /// 32 filter primitives applied pass, 33 do not.
    #[test]
    fn at_most_32_filter_primitives_are_applied() {
        let rects = |count: usize| {
            doc(&format!(
                // In user space, so usvg keeps one filter for all the rects.
                r#"<filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="1" height="1"><feFlood/></filter>{}"#,
                r#"<rect width="1" height="1" filter="url(#f)"/>"#.repeat(count)
            ))
        };
        assert_eq!(check(&rects(MAX_SVG_FILTER_APPLICATIONS)), Ok(()));
        let err = check(&rects(MAX_SVG_FILTER_APPLICATIONS + 1)).unwrap_err();
        assert!(err.contains("applied"), "{err}");
    }

    /// The re-review's probes: a turbulence in a pattern scaled 50 times (3.6 s), a
    /// morphology whose radius the pattern makes ~860 px; and a filter in a mask.
    #[test]
    fn filters_inside_patterns_and_masks_are_refused() {
        let pattern = |filter: &str, side: u32, scale: u32| {
            doc(&format!(
                r#"<filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="{side}" height="{side}">{filter}</filter><pattern id="p" patternUnits="userSpaceOnUse" width="{side}" height="{side}" patternTransform="scale({scale})"><rect width="{side}" height="{side}" filter="url(#f)"/></pattern><rect width="100" height="100" fill="url(#p)"/>"#
            ))
        };
        let documents = [
            pattern(
                r#"<feTurbulence baseFrequency="0.05" numOctaves="10"/>"#,
                10,
                50,
            ),
            pattern(r#"<feMorphology radius="2"/>"#, 1, 30),
            doc(
                r#"<filter id="f"><feFlood/></filter><mask id="m"><rect width="10" height="10" filter="url(#f)"/></mask><rect width="10" height="10" mask="url(#m)"/>"#,
            ),
        ];
        for svg in documents {
            let err = check(&svg).unwrap_err();
            assert!(err.contains("inside a pattern"), "{err}");
        }
    }

    /// A merge composites each input: it weighs as many.
    #[test]
    fn a_merge_weighs_its_inputs() {
        let merge = doc(&format!(
            r#"<filter id="f"><feMerge>{}</feMerge></filter><rect width="10" height="10" filter="url(#f)"/>"#,
            "<feMergeNode/>".repeat(31)
        ));
        let tree = tree(&merge);
        let filter = &tree.filters()[0];
        let weight = filter_weight(filter.primitives()[0].kind(), usvg::Transform::identity());
        assert_eq!(weight, 31.);
    }

    #[test]
    fn a_raster_has_the_device_size() {
        let svg = doc(r#"<circle cx="50" cy="50" r="40" fill="red"/>"#);
        for (width, height) in [(710, 240), (1065, 360)] {
            let at = device(width, height);
            let raster = rasterize(svg.as_bytes(), at, ImageFit::Fill).unwrap();
            assert_eq!((raster.width, raster.height), at);
        }
    }

    /// `Cover` fills the raster with the middle of the document; `Contain` leaves margins.
    #[test]
    fn contain_and_cover_place_the_document_evenly() {
        let svg = tree(&doc(""));
        let at = device(200, 100);
        let contain = placement(&svg, at, ImageFit::Contain);
        assert_eq!(
            (contain.sx, contain.sy, contain.tx, contain.ty),
            (1., 1., 50., 0.)
        );
        let cover = placement(&svg, at, ImageFit::Cover);
        assert_eq!((cover.sx, cover.sy, cover.tx, cover.ty), (2., 2., 0., -50.));
        let fill = placement(&svg, at, ImageFit::Fill);
        assert_eq!((fill.sx, fill.sy, fill.tx, fill.ty), (2., 1., 0., 0.));
    }

    #[test]
    fn usvg_tree_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<usvg::Tree>();
    }
}
