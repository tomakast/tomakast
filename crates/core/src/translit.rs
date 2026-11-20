//! Romanization and the phonetic skeleton: a coarse Latin spelling of a text in any supported
//! script, so `хром` and `Chrome` (or `fajly` and `Файлы`) end up as near-identical strings.
//!
//! Tables follow GOST-like romanization for Cyrillic, the Georgian national system (2002) and
//! ISO 9985 for Armenian, with diacritics and apostrophes dropped.

/// Lowercase, transliterated, not folded. Latin text and non-letters pass through (lowercased).
pub fn romanize(s: &str) -> String {
    romanize_with(s, false)
}

/// Romanization followed by the phonetic folds: spellings that sound alike become one string.
pub fn skeleton(s: &str) -> String {
    fold(&romanize_with(s, false))
}

/// A second skeleton for texts where one letter has two common Latin spellings: `й` as `j`
/// (`nastrojki`) and Georgian `ფ` as `f` (loanwords). `Some` only when it differs from
/// [`skeleton`].
pub fn skeleton_alt(s: &str) -> Option<String> {
    if !s
        .chars()
        .flat_map(char::to_lowercase)
        .any(|c| matches!(c, 'й' | 'ფ'))
    {
        return None;
    }
    let alt = fold(&romanize_with(s, true));
    (alt != skeleton(s)).then_some(alt)
}

fn romanize_with(s: &str, alt: bool) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 2);
    let mut chars = s.chars().flat_map(char::to_lowercase).peekable();
    while let Some(ch) = chars.next() {
        // Armenian `ու` is one sound, `u`.
        if ch == 'ո' && chars.peek() == Some(&'ւ') {
            chars.next();
            out.push('u');
            continue;
        }
        if alt {
            match ch {
                'й' => {
                    out.push('j');
                    continue;
                }
                'ფ' => {
                    out.push('f');
                    continue;
                }
                _ => {}
            }
        }
        match letter(ch) {
            Some(latin) => out.push_str(latin),
            None => out.push(ch),
        }
    }
    out
}

/// The Latin spelling of one lowercase letter, or `None` when it is not in the tables.
fn letter(ch: char) -> Option<&'static str> {
    Some(match ch {
        // Russian.
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "yo",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' | '\u{2BC}' => "",
        'ы' => "y",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        // Ukrainian, Belarusian.
        'і' => "i",
        'ї' => "yi",
        'є' => "ye",
        'ґ' => "g",
        'ў' => "u",
        // Kazakh, Kyrgyz, Tajik, Uzbek national letters.
        'ә' => "a",
        'ғ' => "g",
        'қ' => "k",
        'ң' => "n",
        'ө' => "o",
        'ү' | 'ұ' | 'ӯ' => "u",
        'ҳ' | 'һ' => "h",
        'ӣ' => "i",
        'ҷ' => "j",
        // Georgian (Mkhedruli; Mtavruli capitals are lowercased before they get here).
        'ა' => "a",
        'ბ' => "b",
        'გ' => "g",
        'დ' => "d",
        'ე' => "e",
        'ვ' => "v",
        'ზ' => "z",
        'თ' => "t",
        'ი' => "i",
        'კ' => "k",
        'ლ' => "l",
        'მ' => "m",
        'ნ' => "n",
        'ო' => "o",
        'პ' => "p",
        'ჟ' => "zh",
        'რ' => "r",
        'ს' => "s",
        'ტ' => "t",
        'უ' => "u",
        'ფ' => "p",
        'ქ' => "k",
        'ღ' => "gh",
        'ყ' => "q",
        'შ' => "sh",
        'ჩ' => "ch",
        'ც' => "ts",
        'ძ' => "dz",
        'წ' => "ts",
        'ჭ' => "ch",
        'ხ' => "kh",
        'ჯ' => "j",
        'ჰ' => "h",
        // Armenian.
        'ա' => "a",
        'բ' => "b",
        'գ' => "g",
        'դ' => "d",
        'ե' => "e",
        'զ' => "z",
        'է' => "e",
        'ը' => "e",
        'թ' => "t",
        'ժ' => "zh",
        'ի' => "i",
        'լ' => "l",
        'խ' => "kh",
        'ծ' => "ts",
        'կ' => "k",
        'հ' => "h",
        'ձ' => "dz",
        'ղ' => "gh",
        'ճ' => "ch",
        'մ' => "m",
        'յ' => "y",
        'ն' => "n",
        'շ' => "sh",
        'ո' => "o",
        'չ' => "ch",
        'պ' => "p",
        'ջ' => "j",
        'ռ' | 'ր' => "r",
        'ս' => "s",
        'վ' | 'ւ' => "v",
        'տ' => "t",
        'ց' => "ts",
        'փ' => "p",
        'ք' => "k",
        'օ' => "o",
        'ֆ' => "f",
        'և' => "ev",
        _ => return None,
    })
}

/// The phonetic folds on a romanized string.
fn fold(s: &str) -> String {
    let s = fold_c(s);
    let s = fold_spellings(&s);
    drop_glide_i(&s)
}

/// `c` spellings: `shch sch → sh`, `tch ch → h`, `ck → k`, `c` before `e i y` → `s`, else `k`.
fn fold_c(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let starts = |at: usize, pat: &str| {
        pat.chars()
            .enumerate()
            .all(|(k, p)| chars.get(at + k) == Some(&p))
    };
    while i < chars.len() {
        if starts(i, "shch") {
            out.push_str("sh");
            i += 4;
        } else if starts(i, "sch") {
            out.push_str("sh");
            i += 3;
        } else if starts(i, "tch") {
            out.push('h');
            i += 3;
        } else if starts(i, "ch") {
            out.push('h');
            i += 2;
        } else if starts(i, "ck") {
            out.push('k');
            i += 2;
        } else if chars[i] == 'c' {
            let soft = matches!(chars.get(i + 1), Some('e' | 'i' | 'y'));
            out.push(if soft { 's' } else { 'k' });
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The remaining digraphs and single-letter folds.
fn fold_spellings(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let next = chars.get(i + 1).copied();
        let (folded, len): (&str, usize) = match (chars[i], next) {
            ('d', Some('z')) if chars.get(i + 2) == Some(&'h') => ("j", 3),
            ('d', Some('j')) | ('z', Some('h')) => ("j", 2),
            ('k', Some('h')) => ("h", 2),
            ('g', Some('h')) => ("g", 2),
            ('p', Some('h')) => ("f", 2),
            ('t', Some('h')) => ("t", 2),
            ('t', Some('s')) => ("s", 2),
            ('o', Some('o' | 'u')) => ("u", 2),
            ('e', Some('e')) => ("i", 2),
            ('x', _) => ("ks", 1),
            ('q', _) => ("k", 1),
            ('w', _) => ("v", 1),
            ('y', _) => ("i", 1),
            _ => {
                out.push(chars[i]);
                i += 1;
                continue;
            }
        };
        out.push_str(folded);
        i += len;
    }
    out
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

fn is_consonant(c: char) -> bool {
    c.is_ascii_alphabetic() && !is_vowel(c)
}

/// Drops an `i` between a consonant and a vowel other than `i` (`ля` → `la`), then collapses
/// doubled letters.
fn drop_glide_i(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut last: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        if c == 'i'
            && i > 0
            && is_consonant(chars[i - 1])
            && matches!(chars.get(i + 1), Some('a' | 'e' | 'o' | 'u'))
        {
            continue;
        }
        if c.is_alphabetic() && last == Some(c) {
            continue;
        }
        out.push(c);
        last = Some(c);
    }
    out
}
