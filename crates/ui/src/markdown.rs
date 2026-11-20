//! Safe markdown for module-provided text, which is untrusted.
//!
//! The renderer supports a CommonMark subset: headings, paragraphs, emphasis,
//! inline code, code blocks, lists, links and horizontal rules.
//!
//! - HTML, block and inline, is shown as literal text; nothing interprets it.
//! - An image is shown as its alt text and URL. Nothing is fetched.
//! - A link opens only when it is clicked, and only `http`, `https` and
//!   `mailto` links that pass the core's URL check open at all; see
//!   [`open_link`].

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, InteractiveText, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, StyledText, UnderlineStyle, div, px,
};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::theme::{self, Palette};

/// A block of the supported subset.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    /// Level 1 to 6.
    Heading(u8, Vec<Span>),
    Code(String),
    /// `start` is the number of the first item of an ordered list.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Rule,
}

/// A run of text with one style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// The link target, as written; it is checked only when clicked.
    pub link: Option<String>,
}

/// Opens a link target. The renderer calls it only with the parsed, filtered
/// form [`open_link`] got from [`tomakast_core::checked_url`].
pub trait LinkOpener {
    fn open(&self, url: &str, cx: &mut App);
}

pub use tomakast_core::LinkPolicy;

/// The default policy: every module's links may open.
pub(crate) struct AllowLinks;

impl LinkPolicy for AllowLinks {
    fn may_open_links(&self, _provider: &str) -> bool {
        true
    }
}

/// Opens links of `provider`'s views in the system's default handler while
/// `policy` allows it; asked on every click.
pub(crate) struct PolicyOpener {
    pub provider: String,
    pub policy: Rc<dyn LinkPolicy>,
}

impl LinkOpener for PolicyOpener {
    fn open(&self, url: &str, cx: &mut App) {
        if self.policy.may_open_links(&self.provider) {
            cx.open_url(url);
        } else {
            tracing::info!(module = %self.provider, "link not opened: the module may not open links");
        }
    }
}

/// Opens links in the system's default handler.
struct SystemOpener;

impl LinkOpener for SystemOpener {
    fn open(&self, url: &str, cx: &mut App) {
        cx.open_url(url);
    }
}

/// Opens `url` through `opener` if it passes [`tomakast_core::checked_url`],
/// the rules of the plugin `open` import (`http`, `https` or `mailto`, no
/// spaces, control characters, user name or password; a `mailto` keeps only
/// its message fields). The opener gets the parsed, filtered URL, never the
/// text as written; anything else is ignored with a warning.
pub fn open_link(url: &str, opener: &dyn LinkOpener, cx: &mut App) {
    match tomakast_core::checked_url(url) {
        Ok(checked) => opener.open(&checked, cx),
        Err(why) => tracing::warn!(url = ?url, "link not opened: {why}"),
    }
}

/// Parses `md` into the supported subset. Other constructs are flattened to
/// their text.
pub fn parse(md: &str) -> Vec<Block> {
    // The document's own frame is the bottom of the stack, from the start:
    // input that yields no block (empty, blank, a lone link reference
    // definition) must give an empty document, not a panic.
    let mut builder = Builder {
        frames: vec![Vec::new()],
        ..Builder::default()
    };
    for event in Parser::new(md) {
        builder.event(event);
    }
    builder.finish()
}

/// The text of `md` as rendered, without styling: blocks on separate lines.
pub fn plain_text(md: &str) -> String {
    let mut lines = Vec::new();
    collect_text(&parse(md), &mut lines);
    lines.join("\n")
}

fn collect_text(blocks: &[Block], lines: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(spans) | Block::Heading(_, spans) => {
                lines.push(spans.iter().map(|s| s.text.as_str()).collect());
            }
            Block::Code(code) => lines.push(code.clone()),
            Block::List { items, .. } => {
                for item in items {
                    collect_text(item, lines);
                }
            }
            Block::Rule => {}
        }
    }
}

#[derive(Default)]
struct Builder {
    /// Blocks of the document, then of each open list item.
    frames: Vec<Vec<Block>>,
    /// Open lists: first number and the finished items.
    lists: Vec<(Option<u64>, Vec<Vec<Block>>)>,
    spans: Vec<Span>,
    heading: Option<u8>,
    bold: u32,
    italic: u32,
    link: Option<String>,
    /// Target and alt text of the image being read.
    image: Option<(String, String)>,
    code_block: Option<String>,
    html_block: Option<String>,
}

impl Builder {
    fn frame(&mut self) -> &mut Vec<Block> {
        if self.frames.is_empty() {
            self.frames.push(Vec::new());
        }
        self.frames.last_mut().expect("a frame was just pushed")
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        // Unbalanced events cannot come from the parser, but the bottom frame
        // is the document either way.
        self.frames.into_iter().next().unwrap_or_default()
    }

    /// Ends the pending inline run as a heading or a paragraph.
    fn flush(&mut self) {
        let heading = self.heading.take();
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        self.frame().push(match heading {
            Some(level) => Block::Heading(level, spans),
            None => Block::Paragraph(spans),
        });
    }

    fn text(&mut self, text: &str, code: bool) {
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        self.spans.push(Span {
            text: text.to_string(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            code,
            link: self.link.clone(),
        });
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.code_block {
                Some(code) => code.push_str(&text),
                None => self.text(&text, false),
            },
            Event::Code(text) => self.text(&text, true),
            // HTML is data: it is shown as it was written.
            Event::Html(html) => match &mut self.html_block {
                Some(block) => block.push_str(&html),
                None => self.text(&html, false),
            },
            Event::InlineHtml(html) => self.text(&html, false),
            Event::SoftBreak => self.text(" ", false),
            Event::HardBreak => self.text("\n", false),
            Event::Rule => {
                self.flush();
                self.frame().push(Block::Rule);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(heading_level(level));
            }
            Tag::Paragraph => self.flush(),
            Tag::CodeBlock(_) => {
                self.flush();
                self.code_block = Some(String::new());
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html_block = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push((start, Vec::new()));
            }
            Tag::Item => {
                self.flush();
                self.frames.push(Vec::new());
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Link { dest_url, .. } => self.link = Some(dest_url.to_string()),
            Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) | TagEnd::Paragraph => self.flush(),
            TagEnd::CodeBlock => {
                if let Some(code) = self.code_block.take() {
                    self.frame()
                        .push(Block::Code(code.trim_end_matches('\n').to_string()));
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html_block.take() {
                    self.text(html.trim_end_matches('\n'), false);
                    self.flush();
                }
            }
            TagEnd::Item => {
                self.flush();
                if let Some(blocks) = self.frames.pop()
                    && let Some((_, items)) = self.lists.last_mut()
                {
                    items.push(blocks);
                }
            }
            TagEnd::List(_) => {
                if let Some((start, items)) = self.lists.pop() {
                    self.frame().push(Block::List { start, items });
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Link => self.link = None,
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    let shown = if alt.is_empty() {
                        url
                    } else {
                        format!("{alt} ({url})")
                    };
                    self.text(&shown, false);
                }
            }
            _ => {}
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Renders `md`; links open in the system's default handler on click.
pub fn render(md: &str, cx: &App) -> AnyElement {
    render_with(md, Rc::new(SystemOpener), cx)
}

/// Like [`render`], opening links through `opener`.
pub fn render_with(md: &str, opener: Rc<dyn LinkOpener>, cx: &App) -> AnyElement {
    let mut paint = Paint {
        palette: theme::palette(cx),
        link: cx.theme().link,
        mono: cx.theme().mono_font_family.clone(),
        opener,
        texts: 0,
    };
    blocks(&parse(md), &mut paint)
}

/// What rendering needs besides the blocks.
struct Paint {
    palette: Palette,
    link: gpui_kit::Hsla,
    mono: SharedString,
    opener: Rc<dyn LinkOpener>,
    /// Text elements made so far, for their ids.
    texts: usize,
}

fn blocks(list: &[Block], paint: &mut Paint) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(list.iter().map(|block| block_element(block, paint)))
        .into_any_element()
}

fn block_element(block: &Block, paint: &mut Paint) -> AnyElement {
    match block {
        Block::Paragraph(spans) => text_element(spans, paint),
        Block::Heading(level, spans) => {
            let heading = div().font_weight(FontWeight::BOLD);
            let heading = match level {
                1 => heading.text_xl(),
                2 => heading.text_lg(),
                _ => heading,
            };
            heading.child(text_element(spans, paint)).into_any_element()
        }
        Block::Code(code) => div()
            .p_2()
            .rounded(px(6.))
            .bg(paint.palette.selected)
            .font_family(paint.mono.clone())
            .font_features(theme::mono_features())
            .text_sm()
            .overflow_hidden()
            .child(code.clone())
            .into_any_element(),
        Block::List { start, items } => div()
            .flex()
            .flex_col()
            .gap_1()
            .children(items.iter().enumerate().map(|(ix, item)| {
                let marker = match start {
                    Some(start) => format!("{}.", start + ix as u64),
                    None => "•".to_string(),
                };
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_none().child(marker))
                    .child(div().flex_1().min_w_0().child(blocks(item, paint)))
                    .into_any_element()
            }))
            .into_any_element(),
        Block::Rule => div().h(px(1.)).bg(paint.palette.border).into_any_element(),
    }
}

/// One run of styled text; its links are clickable.
fn text_element(spans: &[Span], paint: &mut Paint) -> AnyElement {
    let mut text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut mono: Vec<(Range<usize>, SharedString)> = Vec::new();
    let mut link_ranges = Vec::new();
    let mut link_urls = Vec::new();
    for span in spans {
        let start = text.len();
        text.push_str(&span.text);
        let range = start..text.len();
        if range.is_empty() {
            continue;
        }
        let mut style = HighlightStyle::default();
        if span.bold {
            style.font_weight = Some(FontWeight::BOLD);
        }
        if span.italic {
            style.font_style = Some(FontStyle::Italic);
        }
        if span.code {
            style.background_color = Some(paint.palette.selected);
            mono.push((range.clone(), paint.mono.clone()));
        }
        if let Some(url) = &span.link {
            style.color = Some(paint.link);
            style.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            });
            link_ranges.push(range.clone());
            link_urls.push(url.clone());
        }
        highlights.push((range, style));
    }
    let styled = StyledText::new(text)
        .with_highlights(highlights)
        .with_font_family_overrides(mono);
    if link_ranges.is_empty() {
        return styled.into_any_element();
    }
    paint.texts += 1;
    let opener = paint.opener.clone();
    InteractiveText::new(("markdown-text", paint.texts), styled)
        .on_click(link_ranges, move |ix, _, cx| {
            if let Some(url) = link_urls.get(ix) {
                open_link(url, opener.as_ref(), cx);
            }
        })
        .into_any_element()
}
