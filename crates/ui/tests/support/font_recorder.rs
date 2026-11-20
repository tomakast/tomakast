//! A headless text system that records the font family and features of
//! every laid-out line; `mod`-included by the tests that check which font
//! text is in.

use std::sync::Mutex;

use gpui_kit::{Bounds, Pixels};

/// A font as the recorder tells them apart: its family and its OpenType
/// features.
type Face = (String, Vec<(String, u32)>);

/// A headless text system that records which font family laid out which
/// text, with which features. It measures like GPUI's no-op one (every
/// glyph as wide as `m`), but gives every family and features their own
/// font id.
#[derive(Default)]
pub struct FontRecorder {
    /// How many font blobs were added.
    added: Mutex<usize>,
    faces: Mutex<Vec<Face>>,
    lines: Mutex<Vec<(String, Vec<Face>)>>,
}

impl gpui_kit::PlatformTextSystem for FontRecorder {
    fn add_fonts(&self, fonts: Vec<std::borrow::Cow<'static, [u8]>>) -> anyhow::Result<()> {
        *self.added.lock().unwrap() += fonts.len();
        gpui_kit::NoopTextSystem.add_fonts(fonts)
    }
    fn all_font_names(&self) -> Vec<String> {
        gpui_kit::NoopTextSystem.all_font_names()
    }
    fn font_id(&self, descriptor: &gpui_kit::Font) -> anyhow::Result<gpui_kit::FontId> {
        let mut faces = self.faces.lock().unwrap();
        let face = (
            descriptor.family.to_string(),
            descriptor.features.tag_value_list().to_vec(),
        );
        let id = match faces.iter().position(|f| *f == face) {
            Some(id) => id,
            None => {
                faces.push(face);
                faces.len() - 1
            }
        };
        Ok(gpui_kit::FontId(id))
    }
    fn font_metrics(&self, font_id: gpui_kit::FontId) -> gpui_kit::FontMetrics {
        gpui_kit::NoopTextSystem.font_metrics(font_id)
    }
    fn typographic_bounds(
        &self,
        font_id: gpui_kit::FontId,
        glyph_id: gpui_kit::GlyphId,
    ) -> anyhow::Result<Bounds<f32>> {
        gpui_kit::NoopTextSystem.typographic_bounds(font_id, glyph_id)
    }
    fn advance(
        &self,
        font_id: gpui_kit::FontId,
        glyph_id: gpui_kit::GlyphId,
    ) -> anyhow::Result<gpui_kit::Size<f32>> {
        gpui_kit::NoopTextSystem.advance(font_id, glyph_id)
    }
    fn glyph_for_char(&self, font_id: gpui_kit::FontId, ch: char) -> Option<gpui_kit::GlyphId> {
        gpui_kit::NoopTextSystem.glyph_for_char(font_id, ch)
    }
    fn glyph_raster_bounds(
        &self,
        params: &gpui_kit::RenderGlyphParams,
    ) -> anyhow::Result<Bounds<gpui_kit::DevicePixels>> {
        gpui_kit::NoopTextSystem.glyph_raster_bounds(params)
    }
    fn rasterize_glyph(
        &self,
        params: &gpui_kit::RenderGlyphParams,
        raster_bounds: Bounds<gpui_kit::DevicePixels>,
    ) -> anyhow::Result<(gpui_kit::Size<gpui_kit::DevicePixels>, Vec<u8>)> {
        gpui_kit::NoopTextSystem.rasterize_glyph(params, raster_bounds)
    }
    fn layout_line(
        &self,
        text: &str,
        font_size: Pixels,
        runs: &[gpui_kit::FontRun],
    ) -> gpui_kit::LineLayout {
        let faces = self.faces.lock().unwrap();
        let used = runs
            .iter()
            .map(|run| faces[run.font_id.0].clone())
            .collect();
        self.lines.lock().unwrap().push((text.to_owned(), used));
        gpui_kit::NoopTextSystem.layout_line(text, font_size, runs)
    }
    fn recommended_rendering_mode(
        &self,
        font_id: gpui_kit::FontId,
        font_size: Pixels,
    ) -> gpui_kit::TextRenderingMode {
        gpui_kit::NoopTextSystem.recommended_rendering_mode(font_id, font_size)
    }
}

/// The fonts that laid out `text`, over every layout of it so far.
fn faces_of(recorder: &FontRecorder, text: &str) -> Vec<Face> {
    let mut faces: Vec<Face> = recorder
        .lines
        .lock()
        .unwrap()
        .iter()
        .filter(|(line, _)| line == text)
        .flat_map(|(_, used)| used.clone())
        .collect();
    faces.sort();
    faces.dedup();
    faces
}

/// The families that laid out `text`, over every layout of it so far.
pub fn families_of(recorder: &FontRecorder, text: &str) -> Vec<String> {
    let mut families: Vec<String> = faces_of(recorder, text)
        .into_iter()
        .map(|(family, _)| family)
        .collect();
    families.dedup();
    families
}

/// The OpenType features `text` was laid out with, over every layout of it
/// so far: one list per font.
#[allow(dead_code)]
pub fn features_of(recorder: &FontRecorder, text: &str) -> Vec<Vec<(String, u32)>> {
    faces_of(recorder, text)
        .into_iter()
        .map(|(_, features)| features)
        .collect()
}

// Not every test file reads the lines themselves.
#[allow(dead_code)]
impl FontRecorder {
    /// How many font blobs were added so far.
    pub fn added(&self) -> usize {
        *self.added.lock().unwrap()
    }

    /// Every family that laid out a line so far.
    pub fn families(&self) -> Vec<String> {
        let mut families: Vec<String> = self
            .lines
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, used)| used.iter().map(|(family, _)| family.clone()))
            .collect();
        families.sort();
        families.dedup();
        families
    }

    /// Every line laid out so far.
    pub fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .map(|(line, _)| line.clone())
            .collect()
    }
}
