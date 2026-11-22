//! Colours: sRGB channels read from hex, `rgb()`, `hsl()`, `oklch()` and CSS names, and
//! written back in any of those forms.

/// A colour: sRGB channels and alpha, each in 0–1, and the form it is shown in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
    pub form: Form,
}

/// How a colour is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Hex,
    Rgb,
    Hsl,
    Oklch,
}

/// `#` and 3, 4, 6 or 8 hex digits (any case), without the `#`. Shown as `rgb()` when
/// written alone (echoing the hex would say nothing).
pub fn from_hex(digits: &str) -> Option<Color> {
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&digits[i..=i], 16).ok().map(|n| n * 17);
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let [r, g, b, a] = match digits.len() {
        3 => [nibble(0)?, nibble(1)?, nibble(2)?, 255],
        4 => [nibble(0)?, nibble(1)?, nibble(2)?, nibble(3)?],
        6 => [byte(0)?, byte(2)?, byte(4)?, 255],
        8 => [byte(0)?, byte(2)?, byte(4)?, byte(6)?],
        _ => return None,
    };
    let mut c = from_rgb(r.into(), g.into(), b.into(), f64::from(a) / 255.0);
    c.form = Form::Rgb;
    Some(c)
}

/// A CSS named colour (any case), or `transparent`.
pub fn named(name: &str) -> Option<Color> {
    if name.eq_ignore_ascii_case("transparent") {
        return Some(from_rgb(0.0, 0.0, 0.0, 0.0));
    }
    let lower = name.to_ascii_lowercase();
    let i = NAMES.binary_search_by(|(n, _)| (*n).cmp(&lower)).ok()?;
    let rgb = NAMES[i].1;
    Some(from_rgb(
        f64::from(rgb >> 16 & 0xff),
        f64::from(rgb >> 8 & 0xff),
        f64::from(rgb & 0xff),
        1.0,
    ))
}

/// Channels in 0–255 and alpha in 0–1, each clamped into range.
pub fn from_rgb(r: f64, g: f64, b: f64, a: f64) -> Color {
    let unit = |x: f64| x.clamp(0.0, 1.0);
    Color {
        r: unit(r / 255.0),
        g: unit(g / 255.0),
        b: unit(b / 255.0),
        a: unit(a),
        form: Form::Hex,
    }
}

/// Hue in degrees (any turn), saturation and lightness in 0–1 (clamped).
pub fn from_hsl(h: f64, s: f64, l: f64, a: f64) -> Color {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let h = h.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    from_rgb((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0, a)
}

/// OKLCH: lightness 0–1, chroma, hue in degrees. A colour outside sRGB is clipped per
/// channel.
pub fn from_oklch(l: f64, c: f64, h: f64, a: f64) -> Color {
    let (l, c) = (l.clamp(0.0, 1.0), c.max(0.0));
    let (sin, cos) = h.to_radians().sin_cos();
    let (oa, ob) = (c * cos, c * sin);
    let l_ = l + 0.396_337_777_4 * oa + 0.215_803_757_3 * ob;
    let m_ = l - 0.105_561_345_8 * oa - 0.063_854_172_8 * ob;
    let s_ = l - 0.089_484_177_5 * oa - 1.291_485_548 * ob;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let b = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3;
    from_rgb(encode(r) * 255.0, encode(g) * 255.0, encode(b) * 255.0, a)
}

/// sRGB transfer: linear light to the encoded channel.
fn encode(x: f64) -> f64 {
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB transfer: the encoded channel to linear light.
fn decode(x: f64) -> f64 {
    if x <= 0.040_45 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Lightness, chroma and hue (degrees) in OKLCH.
fn oklch(c: &Color) -> (f64, f64, f64) {
    let (r, g, b) = (decode(c.r), decode(c.g), decode(c.b));
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    let ll = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let oa = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let ob = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    let chroma = oa.hypot(ob);
    let hue = ob.atan2(oa).to_degrees().rem_euclid(360.0);
    (ll, chroma, hue)
}

/// Hue (degrees), saturation and lightness (0–1).
fn hsl(c: &Color) -> (f64, f64, f64) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == c.r {
        (c.g - c.b) / d
    } else if max == c.g {
        (c.b - c.r) / d + 2.0
    } else {
        (c.r - c.g) / d + 4.0
    };
    ((h * 60.0).rem_euclid(360.0), s, l)
}

/// Chroma below this is a grey: its hue shows as 0.
const GREY_CHROMA: f64 = 0.000_5;

/// The colour written in `form`.
pub fn format(c: &Color, form: Form) -> String {
    let [r, g, b, a8] = rgba8(c);
    // Alpha that rounds to opaque is opaque.
    let alpha = a8 < 255;
    let a = fixed(c.a, 3);
    match form {
        Form::Hex if alpha => format!("#{r:02x}{g:02x}{b:02x}{a8:02x}"),
        Form::Hex => format!("#{r:02x}{g:02x}{b:02x}"),
        Form::Rgb if alpha => format!("rgba({r}, {g}, {b}, {a})"),
        Form::Rgb => format!("rgb({r}, {g}, {b})"),
        Form::Hsl => {
            let (h, s, l) = hsl(c);
            let (h, s, l) = (fixed(hue(h), 1), fixed(s * 100.0, 1), fixed(l * 100.0, 1));
            if alpha {
                format!("hsla({h}, {s}%, {l}%, {a})")
            } else {
                format!("hsl({h}, {s}%, {l}%)")
            }
        }
        Form::Oklch => {
            let (l, ch, h) = oklch(c);
            let h = if ch < GREY_CHROMA { 0.0 } else { hue(h) };
            let body = format!("{} {} {}", fixed(l, 3), fixed(ch, 3), fixed(h, 1));
            if alpha {
                format!("oklch({body} / {a})")
            } else {
                format!("oklch({body})")
            }
        }
    }
}

/// A hue rounded to the 1 decimal it is shown with, where 360 is 0.
fn hue(h: f64) -> f64 {
    ((h * 10.0).round() / 10.0).rem_euclid(360.0)
}

/// `x` with at most `decimals` decimals, trailing zeros and point trimmed; never `-0`.
fn fixed(x: f64, decimals: usize) -> String {
    let text = format!("{x:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// The CSS name of an opaque colour that is exactly a named one.
pub fn name_of(c: &Color) -> Option<&'static str> {
    let [r, g, b, a] = rgba8(c);
    if [r, g, b, a] == [0; 4] {
        return Some("transparent");
    }
    if a != 255 {
        return None;
    }
    let rgb = u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
    NAMES.iter().find(|(_, v)| *v == rgb).map(|(n, _)| *n)
}

/// The channels and alpha as bytes.
pub fn rgba8(c: &Color) -> [u8; 4] {
    let byte = |x: f64| (x * 255.0).round() as u8;
    [byte(c.r), byte(c.g), byte(c.b), byte(c.a)]
}

/// The colour as a swatch: `#rrggbb`, or `#rrggbbaa` when it is not opaque.
pub fn swatch_hex(c: &Color) -> String {
    format(c, Form::Hex)
}

/// The CSS named colours (CSS Color Module Level 4), sorted by name, as `0xrrggbb`.
const NAMES: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: &str) -> Color {
        from_hex(d).unwrap()
    }

    #[test]
    fn hex_digits() {
        assert_eq!(rgba8(&hex("f80")), rgba8(&hex("FF8800")));
        assert_eq!(hex("ff880080").a, 128.0 / 255.0);
        assert_eq!(format(&hex("ff88"), Form::Hex), "#ffff8888");
        assert_eq!(hex("f80").form, Form::Rgb);
        for bad in ["ff", "ff8800f", "", "ggg", "ff88001"] {
            assert!(from_hex(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn every_form() {
        let c = hex("ff8800");
        assert_eq!(format(&c, Form::Hex), "#ff8800");
        assert_eq!(format(&c, Form::Rgb), "rgb(255, 136, 0)");
        assert_eq!(format(&c, Form::Hsl), "hsl(32, 100%, 50%)");
        assert_eq!(format(&c, Form::Oklch), "oklch(0.744 0.181 56.5)");
        assert_eq!(
            format(&hex("ff0000"), Form::Oklch),
            "oklch(0.628 0.258 29.2)"
        );
        assert_eq!(format(&hex("ffffff"), Form::Oklch), "oklch(1 0 0)");
        assert_eq!(format(&hex("000000"), Form::Oklch), "oklch(0 0 0)");
        assert_eq!(
            format(&hex("008000"), Form::Oklch),
            "oklch(0.52 0.177 142.5)"
        );
        assert_eq!(format(&hex("808080"), Form::Hsl), "hsl(0, 0%, 50.2%)");
    }

    #[test]
    fn alpha_forms() {
        let c = hex("ff880080");
        assert_eq!(format(&c, Form::Hex), "#ff880080");
        assert_eq!(format(&c, Form::Rgb), "rgba(255, 136, 0, 0.502)");
        assert_eq!(format(&c, Form::Hsl), "hsla(32, 100%, 50%, 0.502)");
        assert_eq!(format(&c, Form::Oklch), "oklch(0.744 0.181 56.5 / 0.502)");
        assert_eq!(swatch_hex(&c), "#ff880080");
        assert_eq!(swatch_hex(&hex("ff8800")), "#ff8800");
    }

    #[test]
    fn names() {
        let orange = named("Orange").unwrap();
        assert_eq!(format(&orange, Form::Hex), "#ffa500");
        assert_eq!(orange.form, Form::Hex);
        assert_eq!(
            format(&named("transparent").unwrap(), Form::Hex),
            "#00000000"
        );
        assert!(named("nope").is_none());
        assert_eq!(NAMES.len(), 148);
        assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0), "sorted");
        assert_eq!(name_of(&orange), Some("orange"));
        assert_eq!(name_of(&hex("ff8801")), None);
        assert_eq!(name_of(&hex("ffa50080")), None);
        // Grey has two spellings; either is a name.
        assert!(matches!(name_of(&hex("808080")), Some("gray" | "grey")));
    }

    #[test]
    fn round_trips() {
        assert_eq!(format(&from_hsl(32.0, 1.0, 0.5, 1.0), Form::Hex), "#ff8800");
        assert_eq!(
            format(&from_hsl(180.0, 1.0, 0.5, 1.0), Form::Hex),
            "#00ffff"
        );
        assert_eq!(
            format(&from_hsl(-180.0, 1.0, 0.5, 1.0), Form::Hex),
            "#00ffff"
        );
        assert_eq!(
            format(&from_hsl(392.0, 1.0, 0.5, 1.0), Form::Hex),
            "#ff8800"
        );
        let back = from_oklch(0.744, 0.181, 56.5, 1.0);
        assert_eq!(format(&back, Form::Hex), "#ff8800");
        assert_eq!(
            format(&from_oklch(0.628, 0.258, 29.2, 1.0), Form::Hex),
            "#ff0000"
        );
        let wild = from_oklch(0.9, 0.4, 150.0, 1.0);
        for ch in [wild.r, wild.g, wild.b] {
            assert!((0.0..=1.0).contains(&ch), "{wild:?}");
        }
        // Precision survives a chain of forms.
        let c = hex("123456");
        let hsl = format(&c, Form::Hsl);
        assert_eq!(hsl, "hsl(210, 65.4%, 20.4%)");
        assert_eq!(
            format(&from_rgb(300.0, -5.0, 0.0, 1.0), Form::Hex),
            "#ff0000"
        );
        assert_eq!(
            format(&from_rgb(255.0, 136.0, 0.0, 2.0), Form::Hex),
            "#ff8800"
        );
    }

    #[test]
    fn review_minors() {
        // Alpha that rounds to opaque prints as opaque.
        let nearly = from_rgb(255.0, 136.0, 0.0, 0.999);
        assert_eq!(format(&nearly, Form::Hex), "#ff8800");
        assert_eq!(
            format(&from_rgb(255.0, 136.0, 0.0, 0.9999), Form::Rgb),
            "rgb(255, 136, 0)"
        );
        // `transparent` has its name.
        assert_eq!(name_of(&named("transparent").unwrap()), Some("transparent"));
        // A hue that rounds to 360 is 0.
        assert_eq!(
            format(&from_hsl(359.97, 1.0, 0.5, 1.0), Form::Hsl),
            "hsl(0, 100%, 50%)"
        );
    }
}
