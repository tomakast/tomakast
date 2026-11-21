use std::path::Path;

use tomakast_core::{
    HomeEdit, HomeEntry, HomeWriteError, Settings, WidgetKey, WidgetSize, add_home, edit_home,
    params,
};

fn entry(widget: &str, size: Option<WidgetSize>, pairs: &[(&str, &str)]) -> HomeEntry {
    HomeEntry {
        widget: WidgetKey::parse(widget).unwrap(),
        size,
        params: params(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string()))),
    }
}

fn plain(widget: &str) -> HomeEntry {
    entry(widget, None, &[])
}

/// A `config.toml` holding `text` in a new temporary folder.
fn config(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    (tmp, path)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The files of `dir`: no temporary file is left behind.
fn files(dir: &Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

/// Whether `text` holds `part` once and before `next`.
fn before(text: &str, part: &str, next: &str) -> bool {
    match (text.find(part), text.find(next)) {
        (Some(a), Some(b)) => a < b && text.matches(part).count() == 1,
        _ => false,
    }
}

// --- add_home ---

/// The reproduced data loss: a file `Settings` cannot load (here a wrong `autostart`) gave
/// the launcher an empty Home, and one Add to Home replaced the user's entries.
#[test]
fn adding_to_a_file_settings_cannot_load_keeps_its_entries() {
    let old = "[launcher]\nautostart = \"yes\"\n\n[[home.widgets]]\nwidget = \"a/one\"\n\n[[home.widgets]]\nwidget = \"a/two\"\ncity = \"Oslo\"\n";
    let (_tmp, path) = config(old);
    assert!(Settings::load(&path).is_err());

    let home = add_home(&path, &entry("b/new", Some(WidgetSize::Quarter), &[])).unwrap();

    assert_eq!(
        home,
        [
            plain("a/one"),
            entry("a/two", None, &[("city", "Oslo")]),
            entry("b/new", Some(WidgetSize::Quarter), &[]),
        ]
    );
    let text = read(&path);
    assert_eq!(
        text,
        format!("{old}\n[[home.widgets]]\nwidget = \"b/new\"\nsize = \"quarter\"\n")
    );
}

#[test]
fn adding_keeps_everything_else_byte_for_byte() {
    let old = "# my launcher\n[launcher]\nautostart = false\n\n# my Home\n[[home.widgets]]\n# the weather here\nwidget = \"weather/now\"  # mine\nsize = \"huge\"\nflag = true\n\n[location]\ncity = \"Москва\"  # mine\n\n[plugins.weather]\nunits = \"metric\"\n# the end\n";
    let (_tmp, path) = config(old);

    let added = entry(
        "calc/answer",
        Some(WidgetSize::Single),
        &[("expression", "a \"quoted\" one")],
    );
    let home = add_home(&path, &added).unwrap();

    let text = read(&path);
    assert!(text.starts_with(old), "{text}");
    assert_eq!(
        home,
        [
            entry("weather/now", None, &[("flag", "true")]),
            added.clone()
        ]
    );
    assert_eq!(Settings::load(&path).unwrap().home, home);
}

#[test]
fn adding_to_a_file_of_comments_puts_the_entry_after_them() {
    let (_tmp, path) = config("# mine\n");

    add_home(&path, &plain("a/b")).unwrap();

    assert_eq!(
        read(&path),
        "# mine\n\n[[home.widgets]]\nwidget = \"a/b\"\n"
    );
}

#[test]
fn adding_to_an_empty_file_writes_only_the_entry() {
    let (_tmp, path) = config("");

    add_home(&path, &entry("a/b", None, &[("city", "Oslo")])).unwrap();

    assert_eq!(
        read(&path),
        "[[home.widgets]]\nwidget = \"a/b\"\ncity = \"Oslo\"\n"
    );
}

#[test]
fn adding_to_a_file_without_a_last_newline_starts_a_line() {
    let (_tmp, path) = config("[launcher]\nautostart = false");

    let home = add_home(&path, &plain("a/b")).unwrap();

    assert_eq!(home, [plain("a/b")]);
    assert_eq!(
        read(&path),
        "[launcher]\nautostart = false\n\n[[home.widgets]]\nwidget = \"a/b\"\n"
    );
}

#[test]
fn adding_creates_a_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("missing").join("config.toml");
    let added = entry("weather/now", None, &[("city", "Oslo")]);

    assert_eq!(
        add_home(&path, &added).unwrap(),
        std::slice::from_ref(&added)
    );

    assert_eq!(Settings::load(&path).unwrap().home, [added]);
}

#[test]
fn adding_to_a_broken_file_writes_nothing() {
    let (tmp, path) = config("[location");

    let err = add_home(&path, &plain("a/b")).unwrap_err();

    assert!(matches!(err, HomeWriteError::Broken(_)), "{err:?}");
    assert_eq!(read(&path), "[location");
    assert_eq!(files(tmp.path()), 1);
}

#[test]
fn adding_appends_after_the_entries_followed_by_other_tables() {
    let old = "[[home.widgets]]\nwidget = \"a/one\"\n\n[location]\ncity = \"Oslo\"\n";
    let (_tmp, path) = config(old);

    let home = add_home(&path, &plain("a/two")).unwrap();

    assert_eq!(home, [plain("a/one"), plain("a/two")]);
    assert!(read(&path).starts_with(old));
    let settings = Settings::load(&path).unwrap();
    assert_eq!(settings.home, home);
    assert_eq!(settings.location.city, "Oslo");
}

#[test]
fn adding_keeps_a_malformed_entry_where_it_is() {
    let old = "[[home.widgets]]\nwidget = \"a/b\"\n\n[[home.widgets]]\nwidget = \"nowidget\"\n\n[[home.widgets]]\nwidget = \"a/c\"\ncity = 5\n";
    let (_tmp, path) = config(old);

    let home = add_home(&path, &plain("x/y")).unwrap();

    assert!(read(&path).starts_with(old));
    assert_eq!(home, [plain("a/b"), plain("x/y")]);
    assert_eq!(Settings::load(&path).unwrap().home, home);
}

/// An explicit `[home]` whose `widgets` is an inline array is read and written in its own
/// form: the new entry is an inline table at the end of it, the comments stay.
#[test]
fn adding_to_an_inline_list_appends_an_inline_table() {
    let old = "[home]\n# my tiles\nwidgets = [\n    { widget = \"a/b\" },  # first\n    { widget = \"a/c\", city = \"Oslo\" },\n]\n\n[location]\ncity = \"Oslo\"\n";
    let (_tmp, path) = config(old);

    let home = add_home(&path, &entry("x/y", Some(WidgetSize::Double), &[])).unwrap();

    let expected = [
        plain("a/b"),
        entry("a/c", None, &[("city", "Oslo")]),
        entry("x/y", Some(WidgetSize::Double), &[]),
    ];
    assert_eq!(home, expected);
    assert_eq!(Settings::load(&path).unwrap().home, expected);
    let text = read(&path);
    assert_eq!(text.matches("# my tiles").count(), 1, "{text}");
    assert_eq!(text.matches("# first").count(), 1, "{text}");
    assert!(
        text.contains("    { widget = \"x/y\", size = \"double\" },\n]"),
        "{text}"
    );
}

#[test]
fn adding_to_an_inline_home_table_appends_to_its_list() {
    let (_tmp, path) = config("home = { widgets = [{ widget = \"a/b\" }] }\n");

    let home = add_home(&path, &plain("x/y")).unwrap();

    assert_eq!(home, [plain("a/b"), plain("x/y")]);
    assert_eq!(Settings::load(&path).unwrap().home, home);
}

#[test]
fn adding_to_a_home_table_without_entries_starts_the_list() {
    let (_tmp, path) = config("[home]\n# nothing yet\n");

    let home = add_home(&path, &plain("x/y")).unwrap();

    assert_eq!(home, [plain("x/y")]);
    assert_eq!(Settings::load(&path).unwrap().home, home);
    assert!(read(&path).contains("# nothing yet"));
}

#[test]
fn a_home_that_is_not_a_table_is_not_written() {
    for text in ["home = 3\n", "[home]\nwidgets = 3\n", "[[home]]\nx = 1\n"] {
        let (_tmp, path) = config(text);
        let err = add_home(&path, &plain("x/y")).unwrap_err();
        assert!(
            matches!(&err, HomeWriteError::Broken(m) if m.contains("home")),
            "{text}: {err:?}"
        );
        assert_eq!(read(&path), text);
        let err = edit_home(&path, &[], HomeEdit::Insert(0, plain("x/y"))).unwrap_err();
        assert!(matches!(&err, HomeWriteError::Broken(_)), "{text}: {err:?}");
        assert_eq!(read(&path), text);
    }
}

// --- edit_home ---

/// Three entries with comments above and inside, a malformed one between the first two, and
/// other tables around.
const LIST: &str = "\
# my launcher
[launcher]
autostart = false

# My Home
[[home.widgets]]
widget = \"a/one\"

# broken on purpose
[[home.widgets]]
widget = \"nowidget\"

# the weather
[[home.widgets]]
# in Oslo
widget = \"w/now\"
city = \"Oslo\"
size = \"single\"  # small

[location]
city = \"Oslo\"

# third
[[home.widgets]]
widget = \"a/three\"
flag = true
";

fn list() -> Vec<HomeEntry> {
    vec![
        plain("a/one"),
        entry("w/now", Some(WidgetSize::Single), &[("city", "Oslo")]),
        entry("a/three", None, &[("flag", "true")]),
    ]
}

/// Applies `edit` to [`LIST`]; checks the returned list against the file's and `expected`.
fn edited(edit: HomeEdit, expected: &[HomeEntry]) -> String {
    let (tmp, path) = config(LIST);
    let home = edit_home(&path, &list(), edit).unwrap();
    assert_eq!(home, expected);
    assert_eq!(Settings::load(&path).unwrap().home, expected);
    assert_eq!(files(tmp.path()), 1);
    let text = read(&path);
    for kept in [
        "# my launcher\n[launcher]\nautostart = false\n",
        "# broken on purpose\n[[home.widgets]]\nwidget = \"nowidget\"\n",
        "[location]\ncity = \"Oslo\"\n",
    ] {
        assert!(text.contains(kept), "{kept:?} is gone: {text}");
    }
    text
}

#[test]
fn a_stale_list_is_not_edited() {
    let (_tmp, path) = config(LIST);
    let mut stale = list();
    stale.pop();

    for edit in [
        HomeEdit::Remove(0),
        HomeEdit::Move { from: 0, to: 1 },
        HomeEdit::Resize(0, WidgetSize::Quarter),
        HomeEdit::Insert(0, plain("x/y")),
    ] {
        let err = edit_home(&path, &stale, edit).unwrap_err();
        assert!(matches!(err, HomeWriteError::Changed), "{err:?}");
        assert_eq!(err.to_string(), "config.toml changed: try again");
        assert_eq!(read(&path), LIST);
    }
}

#[test]
fn an_index_past_the_list_is_refused() {
    let (_tmp, path) = config(LIST);

    for edit in [
        HomeEdit::Remove(3),
        HomeEdit::Move { from: 3, to: 0 },
        HomeEdit::Move { from: 0, to: 3 },
        HomeEdit::Resize(3, WidgetSize::Quarter),
        HomeEdit::Insert(4, plain("x/y")),
    ] {
        let err = edit_home(&path, &list(), edit).unwrap_err();
        assert!(matches!(err, HomeWriteError::NoEntry(_)), "{err:?}");
        assert_eq!(read(&path), LIST);
    }
}

#[test]
fn removing_takes_out_that_table_alone() {
    let l = list();
    let text = edited(HomeEdit::Remove(1), &[l[0].clone(), l[2].clone()]);

    assert!(!text.contains("w/now"), "{text}");
    assert!(
        !text.contains("# the weather"),
        "its comment goes with it: {text}"
    );
    assert!(text.contains("# My Home\n[[home.widgets]]\nwidget = \"a/one\"\n"));
    assert!(text.contains("# third\n[[home.widgets]]\nwidget = \"a/three\"\nflag = true\n"));
}

/// A table owns only the comment lines right above its header: a comment parted from it by a
/// blank line belongs to the place, and stays when the entry is removed.
#[test]
fn removing_the_entry_added_to_a_file_of_comments_keeps_them() {
    let (_tmp, path) = config("# mine\n");
    add_home(&path, &plain("a/b")).unwrap();

    edit_home(&path, &[plain("a/b")], HomeEdit::Remove(0)).unwrap();

    assert_eq!(read(&path), "# mine\n");
}

#[test]
fn a_comment_of_the_file_above_the_list_stays_first() {
    let old = "# header\n\n[[home.widgets]]\nwidget = \"a/one\"\n\n[[home.widgets]]\nwidget = \"a/two\"\n";
    let start = [plain("a/one"), plain("a/two")];
    for (edit, expected) in [
        (HomeEdit::Remove(0), vec![plain("a/two")]),
        (
            HomeEdit::Move { from: 0, to: 1 },
            vec![plain("a/two"), plain("a/one")],
        ),
        (
            HomeEdit::Insert(0, plain("x/y")),
            vec![plain("x/y"), plain("a/one"), plain("a/two")],
        ),
    ] {
        let what = format!("{edit:?}");
        let (_tmp, path) = config(old);
        assert_eq!(edit_home(&path, &start, edit).unwrap(), expected, "{what}");
        let text = read(&path);
        assert!(
            text.starts_with("# header\n\n[[home.widgets]]\n"),
            "{what}: {text}"
        );
        assert_eq!(text.matches("# header").count(), 1, "{what}: {text}");
    }
}

#[test]
fn removing_keeps_the_comment_at_the_end_of_the_table_above() {
    let old = "[launcher]\nautostart = false\n# hotkey = \"ctrl+space\"\n\n[[home.widgets]]\nwidget = \"a/one\"\n";
    let (_tmp, path) = config(old);

    edit_home(&path, &[plain("a/one")], HomeEdit::Remove(0)).unwrap();

    assert_eq!(
        read(&path),
        "[launcher]\nautostart = false\n# hotkey = \"ctrl+space\"\n"
    );
}

#[test]
fn removing_hands_the_comment_above_to_the_table_printed_next() {
    let old = "[[home.widgets]]\nwidget = \"a/one\"\n# the end of a/one\n\n[[home.widgets]]\nwidget = \"a/two\"\n\n[location]\ncity = \"Oslo\"\n";
    let (_tmp, path) = config(old);

    edit_home(
        &path,
        &[plain("a/one"), plain("a/two")],
        HomeEdit::Remove(1),
    )
    .unwrap();

    assert_eq!(
        read(&path),
        "[[home.widgets]]\nwidget = \"a/one\"\n# the end of a/one\n\n[location]\ncity = \"Oslo\"\n"
    );
}

#[test]
fn removing_the_last_inline_element_keeps_the_comment_above_it() {
    let old =
        "[home]\nwidgets = [\n    { widget = \"a/one\" },  # one\n    { widget = \"a/two\" },\n]\n";
    let (_tmp, path) = config(old);

    edit_home(
        &path,
        &[plain("a/one"), plain("a/two")],
        HomeEdit::Remove(1),
    )
    .unwrap();

    let text = read(&path);
    assert!(text.contains("# one"), "{text}");
    assert_eq!(
        Settings::load(&path).unwrap().home,
        [plain("a/one")],
        "{text}"
    );
}

#[test]
fn removing_the_first_entry_keeps_the_malformed_one() {
    let l = list();
    let text = edited(HomeEdit::Remove(0), &l[1..]);

    assert!(!text.contains("a/one"), "{text}");
    assert!(before(&text, "nowidget", "w/now"), "{text}");
}

#[test]
fn moving_carries_the_tables_comments() {
    let l = list();
    let text = edited(
        HomeEdit::Move { from: 1, to: 0 },
        &[l[1].clone(), l[0].clone(), l[2].clone()],
    );

    assert!(
        text.contains("# the weather\n[[home.widgets]]\n# in Oslo\nwidget = \"w/now\"\ncity = \"Oslo\"\nsize = \"single\"  # small\n"),
        "{text}"
    );
    assert!(text.contains("# My Home"), "{text}");
    assert!(before(&text, "w/now", "nowidget"), "{text}");
    assert!(
        before(&text, "nowidget", "a/one"),
        "the malformed entry stays first-but-one: {text}"
    );
    assert!(before(&text, "[location]", "a/three"), "{text}");
}

#[test]
fn moving_to_the_end_puts_the_entry_last() {
    let l = list();
    let text = edited(
        HomeEdit::Move { from: 0, to: 2 },
        &[l[1].clone(), l[2].clone(), l[0].clone()],
    );

    assert!(before(&text, "w/now", "nowidget"), "{text}");
    assert!(before(&text, "a/three", "a/one"), "{text}");
    assert!(
        before(&text, "[location]", "# My Home"),
        "its comment goes with it: {text}"
    );
    assert!(text.trim_end().ends_with("widget = \"a/one\""), "{text}");
}

#[test]
fn moving_in_place_changes_nothing() {
    let text = edited(HomeEdit::Move { from: 1, to: 1 }, &list());

    assert_eq!(text, LIST);
}

#[test]
fn resizing_changes_only_the_size_value() {
    let mut l = list();
    l[1].size = Some(WidgetSize::Double);
    let text = edited(HomeEdit::Resize(1, WidgetSize::Double), &l);

    assert_eq!(
        text,
        LIST.replace("size = \"single\"  # small", "size = \"double\"  # small")
    );
}

#[test]
fn resizing_an_entry_without_a_size_adds_one() {
    let mut l = list();
    l[2].size = Some(WidgetSize::Quarter);
    let text = edited(HomeEdit::Resize(2, WidgetSize::Quarter), &l);

    assert!(
        text.starts_with(&LIST[..LIST.find("# third").unwrap()]),
        "{text}"
    );
    assert!(text.contains("size = \"quarter\""), "{text}");
}

#[test]
fn inserting_puts_the_entry_before_the_one_at_the_index() {
    let l = list();
    let new = entry("x/y", Some(WidgetSize::Quarter), &[("city", "Rome")]);
    let text = edited(
        HomeEdit::Insert(1, new.clone()),
        &[l[0].clone(), new, l[1].clone(), l[2].clone()],
    );

    assert!(before(&text, "nowidget", "x/y"), "{text}");
    assert!(before(&text, "x/y", "# the weather"), "{text}");
}

#[test]
fn inserting_at_the_start_and_the_end() {
    let l = list();
    let new = plain("x/y");

    let text = edited(
        HomeEdit::Insert(0, new.clone()),
        &[new.clone(), l[0].clone(), l[1].clone(), l[2].clone()],
    );
    // The comment right above the first entry is that entry's: the new one goes above it.
    assert!(
        before(
            &text,
            "x/y",
            "# My Home\n[[home.widgets]]\nwidget = \"a/one\""
        ),
        "{text}"
    );

    let mut all = l.clone();
    all.push(new.clone());
    let text = edited(HomeEdit::Insert(3, new), &all);
    assert!(text.starts_with(LIST), "{text}");
}

#[test]
fn inserting_into_a_missing_file_creates_it() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");

    let home = edit_home(&path, &[], HomeEdit::Insert(0, plain("a/b"))).unwrap();

    assert_eq!(home, [plain("a/b")]);
    assert_eq!(Settings::load(&path).unwrap().home, home);
}

#[test]
fn an_inline_list_is_edited_in_its_own_form() {
    let old = "[home]\nwidgets = [\n    { widget = \"a/one\" },\n    # second\n    { widget = \"a/two\", size = \"single\" },\n    { widget = 5 },\n    { widget = \"a/three\" },\n]\n";
    let one = plain("a/one");
    let two = entry("a/two", Some(WidgetSize::Single), &[]);
    let three = plain("a/three");
    let start = vec![one.clone(), two.clone(), three.clone()];
    let new = plain("x/y");
    let mut resized = two.clone();
    resized.size = Some(WidgetSize::Double);

    let cases: Vec<(HomeEdit, Vec<HomeEntry>)> = vec![
        (HomeEdit::Remove(0), vec![two.clone(), three.clone()]),
        (
            HomeEdit::Move { from: 1, to: 2 },
            vec![one.clone(), three.clone(), two.clone()],
        ),
        (
            HomeEdit::Resize(1, WidgetSize::Double),
            vec![one.clone(), resized, three.clone()],
        ),
        (
            HomeEdit::Insert(1, new.clone()),
            vec![one.clone(), new.clone(), two.clone(), three.clone()],
        ),
        (
            HomeEdit::Insert(3, new.clone()),
            vec![one.clone(), two.clone(), three.clone(), new.clone()],
        ),
    ];
    for (edit, expected) in cases {
        let what = format!("{edit:?}");
        let (_tmp, path) = config(old);
        let home = edit_home(&path, &start, edit).unwrap();
        assert_eq!(home, expected, "{what}");
        assert_eq!(Settings::load(&path).unwrap().home, expected, "{what}");
        let text = read(&path);
        assert!(text.contains("{ widget = 5 }"), "{what}: {text}");
        assert_eq!(text.matches("# second").count(), 1, "{what}: {text}");
    }
}

// --- the file itself ---

#[cfg(unix)]
#[test]
fn a_symlinked_config_stays_a_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let dotfiles = tmp.path().join("dotfiles");
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    let target = dotfiles.join("config.toml");
    std::fs::write(&target, "# mine\n").unwrap();
    let link = config_dir.join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    add_home(&link, &plain("a/b")).unwrap();
    edit_home(&link, &[plain("a/b")], HomeEdit::Insert(0, plain("x/y"))).unwrap();

    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        Settings::load(&target).unwrap().home,
        [plain("x/y"), plain("a/b")]
    );
    assert_eq!(files(&dotfiles), 1);
    assert_eq!(files(&config_dir), 1);
}

#[cfg(unix)]
#[test]
fn the_files_mode_is_kept() {
    use std::os::unix::fs::PermissionsExt;
    let (tmp, path) = config("# mine\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    add_home(&path, &plain("a/b")).unwrap();
    edit_home(&path, &[plain("a/b")], HomeEdit::Remove(0)).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(files(tmp.path()), 1);
}
