#!/usr/bin/env python3
"""Generate crates/core/src/layout/tables.rs from xkb and CLDR keyboard data.

Usage: gen_layouts.py <out.rs> --cldr-cache <dir>

Sources:
  * xkb: `xkbcli compile-keymap` through libxkbcommon keysym tables (xkbcommon standard).
  * CLDR release-43 keyboards/{osx,windows}/*-t-k0-*.xml (LDML keyboard data), downloaded
    into the cache dir on first use.

Every table maps a character typed on a layout to the character on the same physical key
of the US layout (base and Shift levels). See README.md.
"""
import argparse
import ctypes
import re
import subprocess
import sys
import unicodedata
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

CLDR_BASE = "https://raw.githubusercontent.com/unicode-org/cldr/release-43/keyboards"
KEY_RE = re.compile(
    r"key <(\w+)>\s*\{(?:[^\[}]*?symbols\[1\]=)?\s*\[\s*([^\]]*)\]"
)

xkb = ctypes.CDLL("libxkbcommon.so.0")
xkb.xkb_keysym_from_name.argtypes = [ctypes.c_char_p, ctypes.c_int]
xkb.xkb_keysym_from_name.restype = ctypes.c_uint32
xkb.xkb_keysym_to_utf32.argtypes = [ctypes.c_uint32]
xkb.xkb_keysym_to_utf32.restype = ctypes.c_uint32


def printable(ch):
    return len(ch) == 1 and not ch.isspace() and unicodedata.category(ch)[0] in "LMNPS"


def keysym_char(name):
    ks = xkb.xkb_keysym_from_name(name.encode(), 0)
    if ks == 0:
        return None
    cp = xkb.xkb_keysym_to_utf32(ks)
    return chr(cp) if cp else None


def xkb_keys(layout, variant=None):
    cmd = ["xkbcli", "compile-keymap", "--layout", layout]
    if variant:
        cmd += ["--variant", variant]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    keys = {}
    for key, body in KEY_RE.findall(out):
        levels = [s.strip() for s in body.split(",")]
        if len(levels) < 2:
            continue
        keys[key] = (keysym_char(levels[0]), keysym_char(levels[1]))
    return keys


def cldr_file(cache, platform, locale):
    name = f"{locale}-t-k0-{platform}.xml"
    path = cache / f"{platform}-{name}"
    if not path.exists():
        url = f"{CLDR_BASE}/{platform}/{name}"
        with urllib.request.urlopen(url) as r:
            path.write_bytes(r.read())
    return path


def unescape(s):
    return re.sub(r"\\u\{([0-9A-Fa-f]+)\}", lambda m: chr(int(m.group(1), 16)), s)


def cldr_keys(path):
    root = ET.parse(path).getroot()
    keys = {}
    for km in root.findall("keyMap"):
        mods = km.get("modifiers")
        level = 0 if mods is None else 1 if mods in ("shift", "shift+caps?") else None
        if level is None:
            continue
        for m in km.findall("map"):
            iso, to = m.get("iso"), unescape(m.get("to", ""))
            pair = list(keys.get(iso, (None, None)))
            pair[level] = to if len(to) == 1 else None
            keys[iso] = tuple(pair)
    return keys


def pairs(layout_keys, us_keys):
    table = {}
    for k in layout_keys:
        if k not in us_keys:
            continue
        for lvl in (0, 1):
            a, b = layout_keys[k][lvl], us_keys[k][lvl]
            if a and b and printable(a) and printable(b) and a != b:
                table.setdefault(a, b)
    # A non-letter must not share its US char with a letter (the ISO extra key repeats
    # punctuation of the main block): the letter wins in both directions.
    letter_targets = {b for a, b in table.items() if a.isalpha()}
    return {a: b for a, b in table.items() if a.isalpha() or b not in letter_targets}


def script_of(table):
    for ch in table:
        if ch.isalpha():
            name = unicodedata.name(ch)
            for s in ("CYRILLIC", "GEORGIAN", "ARMENIAN"):
                if name.startswith(s):
                    return s
    raise SystemExit("no script letter in table")


def rust_char(ch):
    if ch in "'\\":
        return "'\\" + ch + "'"
    return "'" + ch + "'"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--cldr-cache", required=True)
    args = ap.parse_args()
    cache = Path(args.cldr_cache)
    cache.mkdir(parents=True, exist_ok=True)

    us_xkb = xkb_keys("us")
    us_osx = cldr_keys(cldr_file(cache, "osx", "en"))
    us_win = cldr_keys(cldr_file(cache, "windows", "en"))

    # (group, base?, name, source comment, table)
    cands = []
    xkb_src = "xkb (xkbcommon) `{}`"
    for name, lay, var in [
        ("ru", "ru", None), ("ua", "ua", None), ("by", "by", None), ("kz", "kz", None),
        ("kg", "kg", None), ("tj", "tj", None), ("uz", "uz", "cyrillic"),
    ]:
        cands.append(("cyr", 0, name, xkb_src.format(lay + (f"({var})" if var else "")),
                      pairs(xkb_keys(lay, var), us_xkb)))
    cldr = [
        ("cyr", "ru", "ru"), ("cyr", "ua", "uk"), ("cyr", "by", "be"), ("cyr", "kz", "kk"),
        ("ge", "ge", "ka"), ("am", "am", "hy"),
    ]
    for grp, name, loc in cldr:
        cands.append((grp, 1, f"{name}-mac", f"CLDR 43 keyboards/osx/{loc}-t-k0-osx.xml",
                      pairs(cldr_keys(cldr_file(cache, "osx", loc)), us_osx)))
    for grp, name, loc in cldr + [("cyr", "kg", "ky"), ("cyr", "tj", "tg"), ("cyr", "uz", "uz")]:
        cands.append((grp, 1, f"{name}-win",
                      f"CLDR 43 keyboards/windows/{loc}-t-k0-windows.xml",
                      pairs(cldr_keys(cldr_file(cache, "windows", loc)), us_win)))
    for name, lay, var in [("ge", "ge", None), ("am", "am", None)]:
        cands.append((name, 0, name, xkb_src.format(lay), pairs(xkb_keys(lay, var), us_xkb)))

    order = {"cyr": 0, "ge": 1, "am": 2}
    cands.sort(key=lambda c: (order[c[0]], c[1]))  # stable: keeps listing order
    emitted, kept = [], []
    for grp, _, name, src, table in cands:
        if not table:
            print(f"warning: {name} empty", file=sys.stderr)
            continue
        script = script_of(table)
        if any(t == table and s == script for _, s, t, _ in emitted):
            print(f"dropped {name}: equal to an emitted table", file=sys.stderr)
            continue
        emitted.append((name, script, table, src))

    lines = [
        "// @generated by crates/core/tools/gen_layouts.py. Do not edit.",
        "// Each table maps a character typed on a layout to the character on the same",
        "// physical key of the US layout (base and Shift levels), sorted by the first char.",
        "",
        "use super::Layout;",
        "use crate::script::Scripts;",
        "",
        "pub(super) const TABLES: &[Layout] = &[",
    ]
    for name, script, table, src in emitted:
        pairs_sorted = sorted(table.items())
        lines.append(f"    // {name}: {src}")
        lines.append(f"    Layout {{ name: \"{name}\", script: Scripts::{script}, to_us: &[")
        for a, b in pairs_sorted:
            lines.append(f"        ({rust_char(a)}, {rust_char(b)}),")
        lines.append("    ] },")
    lines.append("];")
    Path(args.out).write_text("\n".join(lines) + "\n")
    for name, script, table, _ in emitted:
        print(f"{name:12} {script:9} {len(table)} pairs", file=sys.stderr)


if __name__ == "__main__":
    main()
