//! Tests of the safe markdown renderer: untrusted text is data.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Context, IntoElement, Render, TestAppContext, Window};
use tomakast_ui::markdown::{self, Block, LinkOpener, Span};

/// Records every URL the renderer asks to open.
#[derive(Default)]
struct Recorder(RefCell<Vec<String>>);

impl LinkOpener for Recorder {
    fn open(&self, url: &str, _: &mut gpui_kit::App) {
        self.0.borrow_mut().push(url.to_string());
    }
}

fn spans(block: &Block) -> &[Span] {
    match block {
        Block::Paragraph(spans) | Block::Heading(_, spans) => spans,
        other => panic!("not a text block: {other:?}"),
    }
}

#[test]
fn html_is_literal() {
    assert_eq!(markdown::plain_text("<b>x</b>"), "<b>x</b>");
    assert_eq!(markdown::plain_text("a <i>b</i> c"), "a <i>b</i> c");
    assert_eq!(
        markdown::plain_text("<script>alert(1)</script>\n\ntext"),
        "<script>alert(1)</script>\ntext"
    );
}

#[test]
fn image_is_not_fetched() {
    let blocks = markdown::parse("![logo](https://example.com/a.png)");
    // The alt text and the URL as plain text: no image, no link.
    assert_eq!(blocks.len(), 1);
    let spans = spans(&blocks[0]);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].text, "logo (https://example.com/a.png)");
    assert_eq!(spans[0].link, None);
}

#[gpui_kit::test]
fn unsafe_link_scheme_ignored(cx: &mut TestAppContext) {
    let opener = Rc::new(Recorder::default());
    cx.update(|cx| {
        for url in [
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "ssh://host",
            "//example.com",
            "relative/path",
            " https://example.com",
            "https://exa\nmple.com",
        ] {
            markdown::open_link(url, opener.as_ref(), cx);
        }
    });
    assert!(
        opener.0.borrow().is_empty(),
        "opened {:?}",
        opener.0.borrow()
    );
}

#[gpui_kit::test]
fn safe_link_schemes_open(cx: &mut TestAppContext) {
    let opener = Rc::new(Recorder::default());
    cx.update(|cx| {
        for url in ["http://a.test", "HTTPS://a.test/x", "mailto:me@a.test"] {
            markdown::open_link(url, opener.as_ref(), cx);
        }
    });
    // The opener gets the parsed form, not the text as written.
    assert_eq!(
        *opener.0.borrow(),
        ["http://a.test/", "https://a.test/x", "mailto:me@a.test"]
    );
}

/// I-8 of the final review: a clicked link follows the same rules as the plugin `open`
/// import (one check in the core): no user name or password, no spaces, and a `mailto` keeps
/// only `to`, `cc`, `bcc`, `subject` and `body`. The opener gets the parsed, filtered URL.
#[gpui_kit::test]
fn links_follow_the_open_rules(cx: &mut TestAppContext) {
    let opener = Rc::new(Recorder::default());
    cx.update(|cx| {
        for url in [
            "https://user:pw@example.com/",
            "https://user@example.com/",
            "https://example.com/a b",
            "https://example.com/\tx",
        ] {
            markdown::open_link(url, opener.as_ref(), cx);
        }
    });
    assert!(
        opener.0.borrow().is_empty(),
        "opened {:?}",
        opener.0.borrow()
    );
    cx.update(|cx| {
        markdown::open_link(
            "mailto:help@a.test?subject=Hi&attach=/home/u/.ssh/id_ed25519&body=x",
            opener.as_ref(),
            cx,
        );
    });
    assert_eq!(*opener.0.borrow(), ["mailto:help@a.test?subject=Hi&body=x"]);
}

#[test]
fn subset_blocks_and_inline_styles() {
    let blocks = markdown::parse(
        "# Title\n\nSome *em* **strong** `code` [link](https://a.test)\n\n- one\n- two\n\n1. a\n\n---\n\n```\nlet x;\n```\n",
    );
    assert!(matches!(&blocks[0], Block::Heading(1, _)));
    let para = spans(&blocks[1]);
    assert!(para.iter().any(|s| s.text == "em" && s.italic));
    assert!(para.iter().any(|s| s.text == "strong" && s.bold));
    assert!(para.iter().any(|s| s.text == "code" && s.code));
    assert!(
        para.iter()
            .any(|s| s.text == "link" && s.link.as_deref() == Some("https://a.test"))
    );
    assert!(matches!(&blocks[2], Block::List { start: None, items } if items.len() == 2));
    assert!(matches!(&blocks[3], Block::List { start: Some(1), items } if items.len() == 1));
    assert!(matches!(&blocks[4], Block::Rule));
    assert!(matches!(&blocks[5], Block::Code(code) if code == "let x;"));
}

/// Untrusted input that yields no block must give an empty document.
#[test]
fn input_without_blocks_is_empty() {
    for md in [
        "",
        " ",
        "\n\n  \n",
        "[a]: http://x",
        "<!-- note -->",
        "[a]: http://x\n[b]: http://y\n",
    ] {
        let _ = markdown::parse(md);
        let _ = markdown::plain_text(md);
    }
    assert_eq!(markdown::parse(""), []);
    assert_eq!(markdown::plain_text(""), "");
    assert_eq!(markdown::parse("  \n \n"), []);
    assert_eq!(markdown::parse("[a]: http://x"), []);
}

/// Paints one markdown source.
struct Probe(&'static str);

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        markdown::render(self.0, cx)
    }
}

/// Builds and paints the element of every block kind, and of input with no
/// block, so a renderer that cannot handle one of them fails here.
#[gpui_kit::test]
fn render_smoke_every_block_kind(cx: &mut TestAppContext) {
    for md in [
        "",
        "   ",
        "[a]: http://x",
        "# H1\n\n## H2\n\n### H3",
        "plain",
        "*em* **strong** `code` ***both***",
        "[link](https://a.test) and [bad](file:///x)",
        "![alt](https://a.test/i.png)",
        "```\nlet x;\n\nlet y;\n```",
        "- a\n- b\n  - nested\n\n1. one\n2. two",
        "- [x](https://a.test) in a list",
        "---",
        "<b>html</b>\n\n<div>\nblock\n</div>",
        "> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |",
    ] {
        cx.update(tomakast_ui::init);
        let window = cx.add_window(|_, _| Probe(md));
        cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
}
