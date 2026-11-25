//! The settings window's frame: its title area (the search field over the sidebar, the page
//! title over the page), which window controls it draws, in which shape and on which side,
//! and how its empty parts move the window.
//!
//! On Linux the window asks for client-side decorations; the kit's Root draws the border,
//! shadow and resize edges when it gets them, and the title area moves the window. When the
//! compositor keeps server-side decorations, the title area is just the top of the window.
//! On Windows the system's title bar is hidden: the controls are marked as its caption
//! buttons and the empty parts as its caption, so the system moves the window and offers
//! snap layouts. On macOS the system draws the traffic lights at the top of the sidebar, in
//! a strip of their own level with the page title, the search field in the row below (as in
//! the system's own settings); the title area moves the window itself, as on Linux (AppKit's
//! own title-bar drag is off).

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::label::Label;
use gpui_kit::{
    AnyElement, App, Decorations, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement as _,
    MouseButton, ParentElement as _, Role, Stateful, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, WindowButton, WindowButtonLayout, WindowControlArea,
    WindowControls, div, prelude::FluentBuilder as _, px, rgb, svg, white,
};
use tomakast_platform::FrameButton;

use crate::settings::layout::page_column;
use crate::theme::{self, Palette};

/// The OS the window runs on, as far as the frame cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Buttons on the left and on the right of the title area.
type ButtonLayout = (Vec<FrameButton>, Vec<FrameButton>);

/// What the window learns of the desktop once, when it opens, and the title area's state.
pub(crate) struct Frame {
    platform: Platform,
    layout: ButtonLayout,
    style: ControlStyle,
    /// A left press on an empty part of the title area: the next drag moves the window.
    armed: Rc<Cell<bool>>,
}

impl Frame {
    pub(crate) fn new(cx: &App) -> Frame {
        let platform = Platform::current();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Frame {
            platform,
            layout: button_layout(platform, cx),
            style: control_style(platform, &desktop),
            armed: Rc::default(),
        }
    }

    /// Whether the window draws its own frame now: on Linux when the compositor gave it
    /// client-side decorations, elsewhere always (the system's title bar is hidden).
    fn client_decorated(&self, window: &Window) -> bool {
        match self.platform {
            Platform::Linux => matches!(window.window_decorations(), Decorations::Client { .. }),
            Platform::MacOs | Platform::Windows => true,
        }
    }

    /// The controls to draw now. A window of fixed size has no maximize.
    pub(crate) fn controls(&self, window: &Window) -> FrameControls {
        let mut supported = window.window_controls();
        supported.maximize &= window.is_resizable();
        frame_controls(
            self.client_decorated(window),
            self.layout.clone(),
            supported,
            self.style,
        )
    }

    /// The title area over the sidebar: the left controls, then `search`. On macOS the traffic
    /// lights' strip, as tall as the page title area, and `search` in the row below it.
    pub(crate) fn sidebar_title(
        &self,
        search: AnyElement,
        controls: &FrameControls,
        window: &Window,
        palette: &Palette,
    ) -> AnyElement {
        // In front of the title area: a press here never moves the window, and on Windows it
        // is not the caption.
        let search = div()
            .id("settings-search-area")
            .test_support()
            .occlude()
            .flex_1()
            .min_w_0()
            .child(search);
        if self.platform == Platform::MacOs {
            let strip = div()
                .id("settings-sidebar-title")
                .flex_none()
                .h(px(theme::SETTINGS_TITLE_HEIGHT));
            return div()
                .flex_none()
                .flex()
                .flex_col()
                .child(self.drag_area(strip, window).test_support())
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .px(px(theme::SETTINGS_EDGE))
                        .child(search),
                )
                .into_any_element();
        }
        let left = control_group(
            "frame-left",
            &controls.left,
            controls.style,
            palette.sidebar.alpha(1.),
            window,
            palette,
        );
        let inset = match &left {
            Some(_) if controls.style == ControlStyle::Breeze => theme::BREEZE_EDGE_MARGIN,
            Some(_) => theme::EDGE_PADDING,
            None => theme::SETTINGS_EDGE,
        };
        let band = div()
            .id("settings-sidebar-title")
            .flex_none()
            .h(px(theme::SETTINGS_TITLE_HEIGHT))
            .pl(px(inset))
            .pr(px(theme::SETTINGS_EDGE))
            .flex()
            .items_center()
            .gap(px(theme::ROW_GUTTER))
            .children(left)
            .child(search);
        self.drag_area(band, window)
            .test_support()
            .into_any_element()
    }

    /// The title area over the page: the page's `title` (none when no page is shown), then
    /// the right controls at the window's edge.
    pub(crate) fn page_title(
        &self,
        title: Option<&str>,
        controls: &FrameControls,
        window: &Window,
        palette: &Palette,
    ) -> AnyElement {
        let right = control_group(
            "frame-right",
            &controls.right,
            controls.style,
            palette.panel.alpha(1.),
            window,
            palette,
        );
        // Windows' controls sit flush with the window's edge; Breeze's keep their margin.
        let flush = controls.style == ControlStyle::Windows && right.is_some();
        let edge = if controls.style == ControlStyle::Breeze && right.is_some() {
            theme::BREEZE_EDGE_MARGIN
        } else {
            theme::EDGE_PADDING
        };
        // The title starts where the page's text does (its groups' headings, its rows'
        // titles), in the page's column, clear of the controls at the right.
        let title = title.map(|title| {
            div().absolute().inset_0().child(
                page_column(
                    div()
                        .h_full()
                        .pl(px(theme::SETTINGS_TEXT_INSET))
                        .pr(px(theme::SETTINGS_TITLE_CONTROLS_ROOM))
                        .flex()
                        .items_center()
                        .child(
                            div().id("page-title").test_support().min_w_0().child(
                                Label::new(title.to_string())
                                    .text_size(theme::text(theme::SETTINGS_PAGE_TITLE_SIZE))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.text)
                                    .truncate(),
                            ),
                        ),
                )
                .h_full(),
            )
        });
        let band = div()
            .id("settings-title")
            .relative()
            .flex_none()
            .h(px(theme::SETTINGS_TITLE_HEIGHT))
            .pl(px(theme::EDGE_PADDING))
            .pr(px(if flush { 0. } else { edge }))
            .flex()
            .items_center()
            .gap(px(theme::LABEL_GAP))
            .border_b_1()
            .border_color(palette.divider)
            .children(title)
            .child(div().flex_1())
            .children(right);
        self.drag_area(band, window).into_any_element()
    }

    /// Makes `band`'s empty parts move the window: the caption on Windows; on macOS, and on
    /// Linux when the window draws its frame, a drag moves it and a double click does what the
    /// system does for a title bar (macOS: its double-click setting; Linux: maximize or
    /// restore, unless the window has a fixed size); on Linux a right click opens the window
    /// menu. Its controls and the search
    /// field are in front of it, so presses on them never reach it.
    fn drag_area(&self, band: Stateful<Div>, window: &Window) -> Stateful<Div> {
        let band = band.window_control_area(WindowControlArea::Drag);
        let platform = self.platform;
        let moves = match platform {
            Platform::Linux => self.client_decorated(window),
            Platform::MacOs => true,
            Platform::Windows => false,
        };
        if !moves {
            return band;
        }
        let window_menu = platform == Platform::Linux && window.window_controls().window_menu;
        let (press, release, outside, drag) = (
            self.armed.clone(),
            self.armed.clone(),
            self.armed.clone(),
            self.armed.clone(),
        );
        // As the kit's title bar: the move starts on the first drag, not on the press, so a
        // double click still arrives; a press a control took is never armed. Both bands share
        // the flag: the other band's press-outside runs in the capture phase, before this
        // band's press arms it.
        band.on_mouse_down(MouseButton::Left, move |_, window, _| {
            press.set(!window.default_prevented());
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| release.set(false))
        .on_mouse_down_out(move |_, _, _| outside.set(false))
        .on_mouse_move(move |event, window, _| {
            if event.pressed_button != Some(MouseButton::Left) {
                drag.set(false);
            } else if drag.replace(false) {
                window.start_window_move();
            }
        })
        .on_click(move |event, window, _| {
            if event.click_count() == 2 {
                match platform {
                    Platform::MacOs => window.titlebar_double_click(),
                    _ if window.is_resizable() => window.zoom_window(),
                    _ => {}
                }
            }
        })
        .when(window_menu, |band| {
            band.on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position);
            })
        })
    }
}

/// Where the desktop puts the title bar buttons: on KDE KWin's setting, elsewhere on Linux
/// the system's button layout as GPUI reports it, else minimize, maximize and close on the
/// right (also on Windows; macOS draws none).
fn button_layout(platform: Platform, cx: &App) -> ButtonLayout {
    if platform == Platform::Linux {
        if let Some(layout) = tomakast_platform::kwin_button_layout() {
            return layout;
        }
        if let Some(layout) = cx.button_layout() {
            return from_gpui(layout);
        }
    }
    (
        Vec::new(),
        vec![
            FrameButton::Minimize,
            FrameButton::Maximize,
            FrameButton::Close,
        ],
    )
}

fn from_gpui(layout: WindowButtonLayout) -> ButtonLayout {
    let side = |buttons: [Option<WindowButton>; 3]| {
        buttons
            .into_iter()
            .flatten()
            .map(|button| match button {
                WindowButton::Minimize => FrameButton::Minimize,
                WindowButton::Maximize => FrameButton::Maximize,
                WindowButton::Close => FrameButton::Close,
            })
            .collect()
    };
    (side(layout.left), side(layout.right))
}

/// The controls `buttons` in `style`, in front of the title area whose fill is `backdrop`;
/// `None` when there are none.
fn control_group(
    id: &'static str,
    buttons: &[FrameButton],
    style: ControlStyle,
    backdrop: Hsla,
    window: &Window,
    palette: &Palette,
) -> Option<AnyElement> {
    if buttons.is_empty() {
        return None;
    }
    let maximized = window.is_maximized();
    let gap = match style {
        ControlStyle::Windows => 0.,
        ControlStyle::Breeze => theme::BREEZE_BUTTON_GAP,
        _ => theme::FRAME_BUTTON_GAP,
    };
    let group = div()
        .id(id)
        .occlude()
        .flex_none()
        .h_full()
        .flex()
        .items_center()
        .gap(px(gap))
        .children(
            buttons
                .iter()
                .map(|&button| control(button, style, maximized, backdrop, palette)),
        );
    Some(group.into_any_element())
}

/// The label of a window control, as the window's controls are named for assistive tools.
fn control_label(button: FrameButton, maximized: bool) -> &'static str {
    match button {
        FrameButton::Minimize => "Minimize",
        FrameButton::Maximize if maximized => "Restore",
        FrameButton::Maximize => "Maximize",
        FrameButton::Close => "Close",
    }
}

/// The glyph of a window control in `style`: as Breeze draws them on KDE (minimize a down
/// chevron, maximize an up chevron, restore a diamond, close a cross), Lucide's elsewhere.
fn control_glyph(button: FrameButton, style: ControlStyle, maximized: bool) -> &'static str {
    match (style, button) {
        (ControlStyle::Breeze, FrameButton::Minimize) => "breeze-minimize",
        (ControlStyle::Breeze, FrameButton::Maximize) if maximized => "breeze-restore",
        (ControlStyle::Breeze, FrameButton::Maximize) => "breeze-maximize",
        (ControlStyle::Breeze, FrameButton::Close) => "breeze-close",
        (_, FrameButton::Minimize) => "minus",
        (_, FrameButton::Maximize) if maximized => "chevrons-down-up",
        (_, FrameButton::Maximize) => "chevron-up",
        (_, FrameButton::Close) => "x",
    }
}

/// A window control: its label, glyph and shape; on Windows the system's caption button, on
/// Linux a button that minimizes, maximizes or restores, or closes the window. `backdrop` is
/// the fill of the title area behind it: a hovered Breeze button is filled and its glyph takes
/// this colour.
fn control(
    button: FrameButton,
    style: ControlStyle,
    maximized: bool,
    backdrop: Hsla,
    palette: &Palette,
) -> AnyElement {
    let label = control_label(button, maximized);
    let icon = control_glyph(button, style, maximized);
    let close = button == FrameButton::Close;
    // The hovered close button is red with a white glyph, except on GNOME and on KDE.
    let red_close = close && !matches!(style, ControlStyle::Adwaita | ControlStyle::Breeze);
    let group = format!("frame-{icon}");
    let breeze = style == ControlStyle::Breeze;
    let glyph = svg()
        .path(format!("icons/{icon}.svg"))
        .id(format!("{label}-glyph"))
        .test_support()
        .size(px(if breeze {
            theme::BREEZE_BUTTON_SIZE * theme::BREEZE_GLYPH_SHARE
        } else {
            theme::FRAME_GLYPH_SIZE
        }))
        .text_color(palette.text)
        .when(red_close, |glyph| {
            glyph.group_hover(group.clone(), |style| style.text_color(white()))
        })
        .when(breeze, |glyph| {
            glyph.group_hover(group.clone(), |style| style.text_color(backdrop))
        });
    let element = div()
        .id(label)
        .role(Role::Button)
        .aria_label(label)
        .test_support()
        .group(group.clone())
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .relative();
    let (selected, error) = (palette.selected, palette.error);
    let element = match style {
        // The hover is a circle behind the glyph, as large as the glyph's box: no fill at rest.
        ControlStyle::Breeze => {
            let (size, fill) = (
                theme::BREEZE_BUTTON_SIZE,
                if close { error } else { palette.text },
            );
            let inset = size * (1. - theme::BREEZE_GLYPH_SHARE) / 2.;
            element
                .size(px(size))
                .child(
                    div()
                        .id(format!("{label}-hover"))
                        .absolute()
                        .top(px(inset))
                        .left(px(inset))
                        .size(px(size * theme::BREEZE_GLYPH_SHARE))
                        .rounded_full()
                        .group_hover(group.clone(), move |style| style.bg(fill)),
                )
                .child(glyph)
        }
        ControlStyle::Adwaita => element
            .size(px(theme::ADWAITA_BUTTON_SIZE))
            .rounded_full()
            .bg(palette.chip)
            .hover(|style| style.bg(palette.border))
            .child(glyph),
        ControlStyle::Windows => element
            .child(glyph)
            .w(px(theme::WINDOWS_BUTTON_WIDTH))
            .h_full()
            .hover(move |style| {
                style.bg(if close {
                    rgb(theme::WINDOWS_CLOSE_HOVER).into()
                } else {
                    selected
                })
            }),
        ControlStyle::None => element.child(glyph),
    };
    if style == ControlStyle::Windows {
        let area = match button {
            FrameButton::Minimize => WindowControlArea::Min,
            FrameButton::Maximize => WindowControlArea::Max,
            FrameButton::Close => WindowControlArea::Close,
        };
        return element.window_control_area(area).into_any_element();
    }
    element
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            match button {
                FrameButton::Minimize => window.minimize_window(),
                FrameButton::Maximize => window.zoom_window(),
                // Through the window's own close, so the controller forgets it.
                FrameButton::Close => window.remove_window(),
            }
        })
        .into_any_element()
}

/// The shape of the drawn window controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlStyle {
    /// Round icon buttons; close turns red on hover (KDE, and unknown desktops).
    Breeze,
    /// Small round filled buttons (GNOME and GTK desktops).
    Adwaita,
    /// Wide rectangles as tall as the title area; close turns red on hover.
    Windows,
    /// None drawn: macOS draws its traffic lights.
    None,
}

/// The window controls to draw at the left and right edges of the title area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameControls {
    pub(crate) left: Vec<FrameButton>,
    pub(crate) right: Vec<FrameButton>,
    pub(crate) style: ControlStyle,
}

/// The controls' shape on `platform`, on Linux by the desktops of `current_desktop` (the
/// colon-separated `XDG_CURRENT_DESKTOP`).
fn control_style(platform: Platform, current_desktop: &str) -> ControlStyle {
    match platform {
        Platform::MacOs => ControlStyle::None,
        Platform::Windows => ControlStyle::Windows,
        Platform::Linux => {
            let desktops: Vec<&str> = current_desktop.split(':').collect();
            if desktops.contains(&"KDE") {
                ControlStyle::Breeze
            } else if desktops
                .iter()
                .any(|desktop| GTK_DESKTOPS.contains(desktop))
            {
                ControlStyle::Adwaita
            } else {
                ControlStyle::Breeze
            }
        }
    }
}

/// Desktops whose windows have GTK's title bar buttons.
const GTK_DESKTOPS: [&str; 7] = [
    "GNOME",
    "Unity",
    "Pantheon",
    "Budgie",
    "X-Cinnamon",
    "MATE",
    "XFCE",
];

/// The controls to draw: none when the window manager draws the frame (`client_decorated`
/// false) or the OS draws its own (`style` [`ControlStyle::None`]); otherwise the buttons of
/// `layout` (left, right), less a minimize or maximize the window manager does not
/// `supported`. Close always stays.
fn frame_controls(
    client_decorated: bool,
    layout: (Vec<FrameButton>, Vec<FrameButton>),
    supported: WindowControls,
    style: ControlStyle,
) -> FrameControls {
    if !client_decorated || style == ControlStyle::None {
        return FrameControls {
            left: Vec::new(),
            right: Vec::new(),
            style,
        };
    }
    let kept = |button: &FrameButton| match button {
        FrameButton::Minimize => supported.minimize,
        FrameButton::Maximize => supported.maximize,
        FrameButton::Close => true,
    };
    let (mut left, mut right) = layout;
    left.retain(kept);
    right.retain(kept);
    FrameControls { left, right, style }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FrameButton::{Close, Maximize as Max, Minimize as Min};

    fn all() -> WindowControls {
        WindowControls::default()
    }

    fn right() -> (Vec<FrameButton>, Vec<FrameButton>) {
        (vec![], vec![Min, Max, Close])
    }

    #[test]
    fn no_controls_under_server_decorations() {
        let controls = frame_controls(false, right(), all(), ControlStyle::Breeze);
        assert!(controls.left.is_empty() && controls.right.is_empty());
    }

    #[test]
    fn controls_follow_the_layout() {
        let layout = (vec![Close], vec![Min, Max]);
        let controls = frame_controls(true, layout.clone(), all(), ControlStyle::Adwaita);
        assert_eq!((controls.left, controls.right), layout);
        assert_eq!(controls.style, ControlStyle::Adwaita);
    }

    #[test]
    fn unsupported_minimize_is_dropped() {
        let supported = WindowControls {
            minimize: false,
            ..all()
        };
        let controls = frame_controls(true, right(), supported, ControlStyle::Breeze);
        assert_eq!(controls.right, vec![Max, Close]);
    }

    #[test]
    fn close_is_always_there() {
        let supported = WindowControls {
            fullscreen: false,
            maximize: false,
            minimize: false,
            window_menu: false,
        };
        let controls = frame_controls(true, right(), supported, ControlStyle::Breeze);
        assert_eq!(controls.right, vec![Close]);
        let controls = frame_controls(true, (vec![Close], vec![]), supported, ControlStyle::Breeze);
        assert_eq!((controls.left, controls.right), (vec![Close], vec![]));
    }

    /// Draws `buttons` in `style` as the title area does.
    struct Controls(Vec<FrameButton>, ControlStyle);

    impl gpui_kit::Render for Controls {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            let palette = theme::palette(cx);
            div().size_full().children(control_group(
                "controls",
                &self.0,
                self.1,
                palette.panel,
                window,
                &palette,
            ))
        }
    }

    #[gpui_kit::test]
    fn controls_are_labelled_in_layout_order(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
        use gpui_kit::{AppContext as _, QuitMode};
        for style in [
            ControlStyle::Breeze,
            ControlStyle::Adwaita,
            ControlStyle::Windows,
        ] {
            let (w, _) = cx.update(|cx| {
                cx.set_quit_mode(QuitMode::Explicit);
                crate::init(cx);
                gpui_kit::open_window(Default::default(), cx, |_, cx| {
                    cx.new(|_| Controls(vec![Close, Min, Max], style))
                })
                .unwrap()
            });
            cx.run_until_parked();
            let xs: Vec<_> = cx
                .update_window(w, |_, window, cx| {
                    window.render_frame(cx);
                    ["Close", "Minimize", "Maximize"]
                        .map(|label| window.find_by_label(label).bounds().left())
                })
                .unwrap()
                .into();
            assert!(xs.is_sorted(), "{style:?}: {xs:?}");
        }
    }

    /// Breeze's controls are 22 px with 6 px between them, and the margin at the window's edge
    /// is the gap from the title area's top to the buttons, centred.
    #[gpui_kit::test]
    fn breeze_controls_are_small_and_centred(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
        use gpui_kit::{AppContext as _, QuitMode};
        assert_eq!(theme::BREEZE_BUTTON_SIZE, 22.);
        assert_eq!(theme::BREEZE_BUTTON_GAP, 6.);
        assert_eq!(theme::BREEZE_GLYPH_SHARE, 0.9);
        assert_eq!(
            theme::BREEZE_EDGE_MARGIN,
            (theme::SETTINGS_TITLE_HEIGHT - theme::TITLE_BORDER - theme::BREEZE_BUTTON_SIZE) / 2.
        );
        let (w, _) = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|_| Controls(vec![Min, Max, Close], ControlStyle::Breeze))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let bounds = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                ["Minimize", "Maximize", "Close"].map(|label| window.find_by_label(label).bounds())
            })
            .unwrap();
        for b in bounds {
            assert_eq!(f32::from(b.size.width), theme::BREEZE_BUTTON_SIZE);
            assert_eq!(f32::from(b.size.height), theme::BREEZE_BUTTON_SIZE);
        }
        for pair in bounds.windows(2) {
            let gap = f32::from(pair[1].left() - pair[0].right());
            assert_eq!(gap, theme::BREEZE_BUTTON_GAP);
        }
    }

    /// Breeze draws its glyph box at 0.9 of the button (18 units in 20): 19.8 px in 22.
    #[gpui_kit::test]
    fn breeze_glyph_box_is_nine_tenths_of_the_button(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
        use gpui_kit::{AppContext as _, QuitMode};
        let (w, _) = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|_| Controls(vec![Close], ControlStyle::Breeze))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let (button, glyph) = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                (
                    window.find_by_label("Close").bounds(),
                    window.find("Close-glyph").bounds(),
                )
            })
            .unwrap();
        assert_eq!(f32::from(button.size.width), 22.);
        // Layout rounds to whole pixels: 19.8 is laid out as 20.
        for side in [glyph.size.width, glyph.size.height] {
            assert!((f32::from(side) - 19.8).abs() <= 0.5, "{glyph:?}");
        }
        assert!(f32::from(glyph.size.width) < f32::from(button.size.width));
    }

    /// The sidebar's title area on `platform`, with a stand-in for the search field.
    struct SidebarTitle(Frame);

    impl gpui_kit::Render for SidebarTitle {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            let palette = theme::palette(cx);
            let controls = FrameControls {
                left: Vec::new(),
                right: Vec::new(),
                style: self.0.style,
            };
            let search = div().h(px(28.)).w_full().into_any_element();
            div()
                .w(px(240.))
                .child(self.0.sidebar_title(search, &controls, window, &palette))
        }
    }

    /// The bounds of the strip at the top of the sidebar and of the search field in a window
    /// of `platform`.
    fn sidebar_title_bounds(
        platform: Platform,
        cx: &mut gpui_kit::TestAppContext,
    ) -> (
        gpui_kit::Bounds<gpui_kit::Pixels>,
        gpui_kit::Bounds<gpui_kit::Pixels>,
    ) {
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{AppContext as _, QuitMode};
        let (w, _) = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|_| {
                    SidebarTitle(Frame {
                        platform,
                        layout: (Vec::new(), Vec::new()),
                        style: ControlStyle::Breeze,
                        armed: Rc::default(),
                    })
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("settings-sidebar-title").bounds(),
                window.find("settings-search-area").bounds(),
            )
        })
        .unwrap()
    }

    /// On macOS the traffic lights have the strip at the top of the sidebar to themselves, as
    /// tall as the page title area (so they sit level with the page title, as in the system's
    /// own settings); the search field is in the row below, across the sidebar.
    #[gpui_kit::test]
    fn macos_search_field_sits_below_the_traffic_lights(cx: &mut gpui_kit::TestAppContext) {
        let (strip, search) = sidebar_title_bounds(Platform::MacOs, cx);
        let f = f32::from;
        assert_eq!(f(strip.size.height), theme::SETTINGS_TITLE_HEIGHT);
        // Room for the traffic lights: their position and a 16 pt button, twice their margin.
        let (_, light_top) = theme::SETTINGS_TRAFFIC_LIGHT_POSITION;
        assert!(f(strip.size.height) >= 2. * light_top + 16.);
        assert!(f(search.top()) >= f(strip.bottom()), "{strip:?} {search:?}");
        assert_eq!(f(search.left()), theme::SETTINGS_EDGE);
        assert_eq!(f(search.size.width), 240. - 2. * theme::SETTINGS_EDGE);
    }

    /// Elsewhere the search field is in the title area itself, centred in it.
    #[gpui_kit::test]
    fn linux_search_field_sits_in_the_title_area(cx: &mut gpui_kit::TestAppContext) {
        let (band, search) = sidebar_title_bounds(Platform::Linux, cx);
        let f = f32::from;
        assert_eq!(f(band.size.height), theme::SETTINGS_TITLE_HEIGHT);
        assert!(f(search.top()) > f(band.top()) && f(search.bottom()) < f(band.bottom()));
        assert_eq!(f(search.left()), theme::SETTINGS_EDGE);
    }

    /// Over the page title area itself: the buttons are 22 px, 6 apart, centred vertically in
    /// the area, and the margin from the last button to the window's right edge is the gap
    /// from the area's top to the buttons.
    #[gpui_kit::test]
    fn breeze_controls_sit_in_the_title_area_with_equal_margins(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
        use gpui_kit::{AppContext as _, QuitMode};
        struct Title(Frame);
        impl gpui_kit::Render for Title {
            fn render(
                &mut self,
                window: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl gpui_kit::IntoElement {
                let palette = theme::palette(cx);
                let controls = FrameControls {
                    left: Vec::new(),
                    right: vec![Min, Max, Close],
                    style: ControlStyle::Breeze,
                };
                div().size_full().child(self.0.page_title(
                    Some("Title"),
                    &controls,
                    window,
                    &palette,
                ))
            }
        }
        let (w, _) = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|_| {
                    Title(Frame {
                        platform: Platform::Linux,
                        layout: (Vec::new(), vec![Min, Max, Close]),
                        style: ControlStyle::Breeze,
                        armed: Rc::default(),
                    })
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        let (area, buttons, width) = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                (
                    // The only child of the window: the area is at its top.
                    gpui_kit::Bounds::new(
                        gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)),
                        gpui_kit::size(
                            window.viewport_size().width,
                            gpui_kit::px(theme::SETTINGS_TITLE_HEIGHT),
                        ),
                    ),
                    ["Minimize", "Maximize", "Close"]
                        .map(|label| window.find_by_label(label).bounds()),
                    window.viewport_size().width,
                )
            })
            .unwrap();
        let f = f32::from;
        assert_eq!(f(area.size.height), theme::SETTINGS_TITLE_HEIGHT);
        for b in buttons {
            assert_eq!((f(b.size.width), f(b.size.height)), (22., 22.));
            // Centred vertically in the area.
            let (above, below) = (
                f(b.top() - area.top()),
                f(area.bottom() - b.bottom()) - theme::TITLE_BORDER,
            );
            assert!((above - below).abs() < 0.01, "{above} {below}");
        }
        for pair in buttons.windows(2) {
            assert_eq!(f(pair[1].left() - pair[0].right()), 6.);
        }
        let top_gap = f(buttons[2].top() - area.top());
        let right_margin = f(width - buttons[2].right());
        assert!(
            (right_margin - top_gap).abs() < 0.01,
            "{right_margin} {top_gap}"
        );
    }

    /// On KDE each button has Breeze's glyph: a down chevron to minimize, an up chevron to
    /// maximize, a diamond to restore, a cross to close; other styles keep Lucide's.
    #[test]
    fn breeze_glyphs_match_kde() {
        let glyphs = |style, maximized| {
            [Min, Max, Close].map(|button| control_glyph(button, style, maximized))
        };
        assert_eq!(
            glyphs(ControlStyle::Breeze, false),
            ["breeze-minimize", "breeze-maximize", "breeze-close"]
        );
        assert_eq!(
            glyphs(ControlStyle::Breeze, true),
            ["breeze-minimize", "breeze-restore", "breeze-close"]
        );
        for style in [ControlStyle::Adwaita, ControlStyle::Windows] {
            assert_eq!(glyphs(style, false), ["minus", "chevron-up", "x"]);
            assert_eq!(glyphs(style, true), ["minus", "chevrons-down-up", "x"]);
        }
        assert_eq!(control_label(Max, true), "Restore");
        assert_eq!(control_label(Max, false), "Maximize");
    }

    /// A pointer moving onto a Breeze button tells the window to redraw: the hover circle and
    /// the inverted glyph appear on the next frame, not only when something else repaints.
    #[gpui_kit::test]
    fn breeze_hover_redraws_the_window(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
        use gpui_kit::{AppContext as _, Modifiers, MouseMoveEvent, PlatformInput, QuitMode};
        let made = Rc::new(std::cell::RefCell::new(None));
        let w = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            let (w, _) = gpui_kit::open_window(Default::default(), cx, |_, cx| {
                let controls = cx.new(|_| Controls(vec![Min, Max, Close], ControlStyle::Breeze));
                *made.borrow_mut() = Some(controls.clone());
                controls
            })
            .unwrap();
            w
        });
        let view = made.borrow().clone().unwrap();
        let redraws = Rc::new(Cell::new(0));
        let _sub = cx.update(|cx| {
            let redraws = redraws.clone();
            cx.observe(&view, move |_, _| redraws.set(redraws.get() + 1))
        });
        cx.run_until_parked();
        let center = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find_by_label("Close").bounds().center()
            })
            .unwrap();
        let move_to = |cx: &mut gpui_kit::TestAppContext, position| {
            cx.update_window(w, |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                );
            })
            .unwrap();
            cx.run_until_parked();
        };
        // Empty space of the window: nothing to redraw.
        move_to(cx, gpui_kit::point(gpui_kit::px(400.), gpui_kit::px(300.)));
        let before = redraws.get();
        move_to(cx, center);
        assert!(redraws.get() > before, "hovering Close asked for no redraw");
    }

    #[test]
    fn mac_has_no_drawn_controls() {
        let style = control_style(Platform::MacOs, "");
        assert_eq!(style, ControlStyle::None);
        let controls = frame_controls(true, right(), all(), style);
        assert!(controls.left.is_empty() && controls.right.is_empty());
    }

    #[test]
    fn windows_has_windows_controls() {
        assert_eq!(
            control_style(Platform::Windows, "KDE"),
            ControlStyle::Windows
        );
    }

    #[test]
    fn kde_is_breeze() {
        assert_eq!(control_style(Platform::Linux, "KDE"), ControlStyle::Breeze);
    }

    #[test]
    fn gnome_is_adwaita() {
        for desktop in [
            "GNOME",
            "Unity",
            "Pantheon",
            "Budgie",
            "X-Cinnamon",
            "MATE",
            "XFCE",
        ] {
            assert_eq!(
                control_style(Platform::Linux, desktop),
                ControlStyle::Adwaita,
                "{desktop}"
            );
        }
    }

    #[test]
    fn unknown_desktop_is_breeze() {
        assert_eq!(
            control_style(Platform::Linux, "Hyprland"),
            ControlStyle::Breeze
        );
        assert_eq!(control_style(Platform::Linux, ""), ControlStyle::Breeze);
    }

    #[test]
    fn a_desktop_list_is_split_on_colons() {
        assert_eq!(
            control_style(Platform::Linux, "ubuntu:GNOME"),
            ControlStyle::Adwaita
        );
        // A desktop name is matched whole: "GNOME-Flashback" is not "GNOME".
        assert_eq!(
            control_style(Platform::Linux, "GNOME-Flashback"),
            ControlStyle::Breeze
        );
    }
}
