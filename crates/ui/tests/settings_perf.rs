//! How long a frame of the settings window's Hotkeys page takes with 130 applications
//! indexed and Applications open: a measurement, run on request
//! (`cargo test -p tomakast-ui --test settings_perf -- --ignored --nocapture`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestQueryExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, BorrowAppContext as _, InputEvent as _, QuitMode, ScrollDelta, ScrollEvent,
    TestAppContext, point, px,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{Core, Icon, Item, ItemId, ItemKind, LocalNow, RankWeights};
use tomakast_ui::{SettingsController, SettingsPage};

/// Up to `n` icon files of the system's icon theme, PNG and SVG.
fn system_icons(n: usize) -> Vec<PathBuf> {
    let mut icons = Vec::new();
    for dir in [
        "/usr/share/icons/hicolor/48x48/apps",
        "/usr/share/icons/hicolor/scalable/apps",
    ] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            icons.extend(paths);
        }
    }
    icons.truncate(n);
    icons
}

fn apps(n: usize) -> Vec<Item> {
    // `TOMAKAST_PERF_NO_ICONS=1`: monograms instead, to tell the icons' share.
    let icons = match std::env::var_os("TOMAKAST_PERF_NO_ICONS") {
        Some(_) => Vec::new(),
        None => system_icons(n),
    };
    (0..n)
        .map(|ix| Item {
            id: ItemId::new("apps", &format!("org.example.app{ix:03}.desktop")),
            title: format!("Application {ix:03}"),
            subtitle: None,
            keywords: Vec::new(),
            icon: icons.get(ix).cloned().map(Icon::File),
            kind: ItemKind::Application,
            source: None,
            actions: vec![Action {
                id: "open".into(),
                title: "Open".into(),
                shortcut: None,
            }],
        })
        .collect()
}

#[gpui_kit::test]
#[ignore = "a measurement; run with --ignored --nocapture"]
fn hotkeys_page_frame_time(cx: &mut TestAppContext) {
    measure(cx, "test text system");
}

/// As [`hotkeys_page_frame_time`], with text shaped and measured in the bundled Inter, as in
/// the app.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "a measurement; run with --ignored --nocapture"]
fn hotkeys_page_frame_time_with_inter() {
    let text_system = std::sync::Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        "Inter",
    ));
    let mut cx =
        TestAppContext::build_with_text_system(gpui_kit::TestDispatcher::new(0), None, text_system);
    measure(&mut cx, "Inter");
}

fn measure(cx: &mut TestAppContext, text: &str) {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap();
    core.register(Box::new(
        FakeModule::new(apps(130), || Ok(Vec::new())).with_id("apps"),
    ));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "[hotkeys]\n\"Ctrl+Alt+A\" = \"apps/org.example.app042.desktop\"\n",
    )
    .unwrap();
    let core = Rc::new(RefCell::new(core));
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(SettingsController::new(config.clone(), core.clone()));
    });
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| c.open(Some(SettingsPage::Hotkeys), cx))
    });
    cx.run_until_parked();
    let window = cx
        .update(|cx| cx.global::<SettingsController>().window())
        .unwrap();
    // A frame, after a wheel scroll of the page by `dy` pixels when it is not 0: the
    // scroll's handling and the one frame it calls for.
    let frame = |cx: &mut TestAppContext, dy: f32| {
        cx.update_window(window, |_, window, cx| {
            let position = window.find("settings-page").bounds().center();
            let start = Instant::now();
            if dy != 0. {
                let scroll = ScrollEvent {
                    position,
                    delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                    ..Default::default()
                };
                window.dispatch_event(scroll.to_platform_input(), cx);
            }
            window.render_frame(cx);
            start.elapsed()
        })
        .unwrap()
    };
    // The pointer over the page, as for a wheel.
    cx.update_window(window, |_, window, cx| window.hover("settings-page", cx))
        .unwrap();
    // Warm up: fields made, icons loaded.
    for _ in 0..5 {
        frame(cx, 0.);
        cx.run_until_parked();
    }
    let fields = cx.update(|cx| {
        cx.global::<SettingsController>()
            .hotkey_field_items(cx)
            .len()
    });
    assert!(fields >= 130, "{fields} fields");
    let report = |what: &str, mut times: Vec<Duration>| {
        times.sort();
        let mean = times.iter().sum::<Duration>() / times.len() as u32;
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        eprintln!(
            "hotkeys page, 130 apps, {text}, {what}: frame mean {:.2} ms, median {:.2} ms, min {:.2} ms, max {:.2} ms",
            ms(mean),
            ms(times[times.len() / 2]),
            ms(times[0]),
            ms(times[times.len() - 1]),
        );
    };
    let still = (0..50).map(|_| frame(cx, 0.)).collect();
    report("redrawn", still);
    // Down the whole table and back, 40 px a step.
    let mut scrolling: Vec<Duration> = (0..150).map(|_| frame(cx, -40.)).collect();
    let bottom = cx
        .update_window(window, |_, window, _| {
            window.try_find_by_label("Application 129").is_some()
        })
        .unwrap();
    assert!(bottom, "scrolled to the end of the table");
    scrolling.extend((0..150).map(|_| frame(cx, 40.)));
    report("scrolling", scrolling);
}
