//! Test-only: what crossed the WIT boundary of a plugin, named as in the WIT
//! ([`Recorder`]), and the entries the host notes for each call.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use crate::bindings::tomakast::plugin::types as wit;

/// Test-only: a set of what crossed the WIT boundary of the plugins it is attached to (with
/// [`WasmPlugin::with_recorder`](crate::WasmPlugin::with_recorder)), named as in the WIT:
/// - `import <interface>.<function>`: an import the guest called, allowed or not;
/// - `export <name>`: an export the host called;
/// - `event <case>`, `view-event <case>`, `widget-event <case>` and `pointer-kind <case>`: the
///   `event` (and view or widget event) cases the host tried to deliver;
/// - `effect <case>`: the `effect` cases the guest returned, before any check, `view <case>`
///   for the view a `push-view` or `update-view` carries, and `image-format <case>` for the
///   image of a `set-widget-image`;
/// - `node-kind`, `draw`, `path-segment`, `paint`, `color` and `image-format <case>`: what a
///   frame `render-widget` answered holds.
///
/// Cheap to clone; clones share the set.
#[derive(Clone, Default)]
pub struct Recorder(Arc<Mutex<BTreeSet<String>>>);

impl Recorder {
    pub(crate) fn note(&self, entry: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(entry);
    }

    /// Everything recorded so far.
    pub fn entries(&self) -> BTreeSet<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The recorder entries of a returned `effect`: its case, and the case of the view it
/// carries.
pub(crate) fn effect_entries(effect: &wit::Effect) -> Vec<String> {
    let (case, view) = match effect {
        wit::Effect::PushView(view) => ("push-view", Some(view)),
        wit::Effect::UpdateView(view) => ("update-view", Some(view)),
        wit::Effect::PopView => ("pop-view", None),
        wit::Effect::CloseWindow => ("close-window", None),
        wit::Effect::Toast(_) => ("toast", None),
        wit::Effect::CopyText(_) => ("copy-text", None),
        wit::Effect::ReplaceItems(_) => ("replace-items", None),
        wit::Effect::ShowWidgets(_) => ("show-widgets", None),
        wit::Effect::RedrawWidget(_) => ("redraw-widget", None),
        wit::Effect::SetWidgetImage(_) => ("set-widget-image", None),
    };
    let mut entries = vec![format!("effect {case}")];
    if let wit::Effect::SetWidgetImage(image) = effect {
        entries.push(image_format_entry(image));
    }
    if let Some(view) = view {
        let view_case = match view {
            wit::View::ListView(_) => "list-view",
            wit::View::GridView(_) => "grid-view",
            wit::View::DetailView(_) => "detail-view",
            wit::View::FormView(_) => "form-view",
            wit::View::ScratchpadView(_) => "scratchpad-view",
        };
        entries.push(format!("view {view_case}"));
    }
    entries
}

/// The recorder entries of a delivered `event`: its case, and a view event's case.
pub(crate) fn event_entries(event: &wit::Event) -> Vec<String> {
    let case = match event {
        wit::Event::View(target) => {
            let view_case = match &target.event {
                wit::ViewEvent::QueryChanged(_) => "query-changed",
                wit::ViewEvent::ActionChosen(_) => "action-chosen",
                wit::ViewEvent::FormSubmitted(_) => "form-submitted",
                wit::ViewEvent::TextChanged(_) => "text-changed",
                wit::ViewEvent::Closed => "closed",
            };
            return vec!["event view".into(), format!("view-event {view_case}")];
        }
        wit::Event::TimerFired(_) => "timer-fired",
        wit::Event::HttpDone(_) => "http-done",
        wit::Event::PreferencesChanged => "preferences-changed",
        wit::Event::Location(_) => "location",
        wit::Event::Widget(target) => {
            let mut entries = vec!["event widget".to_string()];
            let widget_case = match &target.event {
                wit::WidgetEvent::Shown => "shown",
                wit::WidgetEvent::Hidden => "hidden",
                wit::WidgetEvent::Action(_) => "action",
                wit::WidgetEvent::Key(_) => "key",
                wit::WidgetEvent::Pointer(pointer) => {
                    let kind = match pointer.kind {
                        wit::PointerKind::Down => "down",
                        wit::PointerKind::Up => "up",
                        wit::PointerKind::Move => "move",
                        wit::PointerKind::Enter => "enter",
                        wit::PointerKind::Leave => "leave",
                    };
                    entries.push(format!("pointer-kind {kind}"));
                    "pointer"
                }
            };
            entries.push(format!("widget-event {widget_case}"));
            return entries;
        }
    };
    vec![format!("event {case}")]
}

fn image_format_entry(image: &wit::WidgetImage) -> String {
    let format = match image.format {
        wit::ImageFormat::Png => "png",
        wit::ImageFormat::Jpeg => "jpeg",
        wit::ImageFormat::Svg => "svg",
    };
    format!("image-format {format}")
}

fn color_entry(color: &wit::Color) -> String {
    match color {
        wit::Color::Token(_) => "color token".into(),
        wit::Color::Rgba(_) => "color rgba".into(),
    }
}

fn style_entries(style: &wit::ShapeStyle, entries: &mut Vec<String>) {
    match &style.paint {
        wit::Paint::Solid(color) => {
            entries.push("paint solid".into());
            entries.push(color_entry(color));
        }
        wit::Paint::Linear(gradient) => {
            entries.push("paint linear".into());
            entries.push(color_entry(&gradient.start.color));
            entries.push(color_entry(&gradient.end.color));
        }
    }
}

/// The recorder entries of a frame: the case of every node kind, canvas command, path
/// segment, paint, colour and image format it holds.
pub(crate) fn frame_entries(frame: &wit::WidgetFrame) -> Vec<String> {
    let mut entries = Vec::new();
    for node in &frame.nodes {
        let kind = match &node.kind {
            wit::NodeKind::Row(_) => "row",
            wit::NodeKind::Column(_) => "column",
            wit::NodeKind::Overlay(_) => "overlay",
            wit::NodeKind::Text(text) => {
                entries.push(color_entry(&text.color));
                "text"
            }
            wit::NodeKind::Icon(_) => "icon",
            wit::NodeKind::Image(_) => "image",
            wit::NodeKind::Progress(_) => "progress",
            wit::NodeKind::Spacer => "spacer",
            wit::NodeKind::Button(_) => "button",
            wit::NodeKind::Canvas(canvas) => {
                canvas
                    .commands
                    .iter()
                    .for_each(|draw| draw_entries(draw, &mut entries));
                "canvas"
            }
        };
        entries.push(format!("node-kind {kind}"));
    }
    entries.extend(frame.images.iter().map(image_format_entry));
    entries
}

fn draw_entries(draw: &wit::Draw, entries: &mut Vec<String>) {
    let case = match draw {
        wit::Draw::Rect(rect) => {
            style_entries(&rect.style, entries);
            "rect"
        }
        wit::Draw::Ellipse(ellipse) => {
            style_entries(&ellipse.style, entries);
            "ellipse"
        }
        wit::Draw::Path(path) => {
            style_entries(&path.style, entries);
            for segment in &path.segments {
                let segment = match segment {
                    wit::PathSegment::MoveTo(_) => "move-to",
                    wit::PathSegment::LineTo(_) => "line-to",
                    wit::PathSegment::QuadTo(_) => "quad-to",
                    wit::PathSegment::CubicTo(_) => "cubic-to",
                    wit::PathSegment::Close => "close",
                };
                entries.push(format!("path-segment {segment}"));
            }
            "path"
        }
        wit::Draw::Text(text) => {
            entries.push(color_entry(&text.color));
            "text"
        }
        wit::Draw::Image(_) => "image",
        wit::Draw::Svg(_) => "svg",
        wit::Draw::PushClip(_) => "push-clip",
        wit::Draw::PopClip => "pop-clip",
        wit::Draw::PushTransform(_) => "push-transform",
        wit::Draw::PopTransform => "pop-transform",
    };
    entries.push(format!("draw {case}"));
}
