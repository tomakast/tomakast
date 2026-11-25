//! `plugin.toml` lists the languages of [`translator::langs`] as the options of `first` and
//! `second`: the settings window offers what the plugin knows, in its order.

use translator::langs::LANGS;

const MANIFEST: &str = include_str!("../plugin.toml");

/// The `(value, title)` options of preference `key`, read line by line (the plugin has no
/// TOML parser): the `[[preferences]]` block whose key it is, up to the next one.
fn options(key: &str) -> Vec<(String, String)> {
    let block = MANIFEST
        .split("[[preferences]]\n")
        .find(|block| block.starts_with(&format!("key = \"{key}\"\n")))
        .unwrap_or_else(|| panic!("no preference {key}"));
    let quoted = |line: &str, field: &str| {
        line.strip_prefix(&format!("{field} = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .map(str::to_string)
    };
    let mut out = Vec::new();
    let mut lines = block.lines();
    while let Some(line) = lines.next() {
        if line == "[[preferences.options]]" {
            let value = lines
                .next()
                .and_then(|l| quoted(l, "value"))
                .expect("a value");
            let title = lines
                .next()
                .and_then(|l| quoted(l, "title"))
                .expect("a title");
            out.push((value, title));
        } else if line.starts_with('[') {
            break;
        }
    }
    out
}

#[test]
fn manifest_languages_match_langs() {
    let want: Vec<(String, String)> = LANGS
        .iter()
        .map(|l| (l.code.to_string(), l.english.to_string()))
        .collect();
    for key in ["first", "second"] {
        assert_eq!(options(key), want, "{key}");
    }
    assert!(
        MANIFEST.contains("key = \"first\"\ntitle = \"Primary language\"\nkind = \"choice\"\n")
    );
    assert!(
        MANIFEST.contains("key = \"second\"\ntitle = \"Secondary language\"\nkind = \"choice\"\n")
    );
}
