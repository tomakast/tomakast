# Layout table generator

`gen_layouts.py` writes `src/layout/tables.rs`: for every supported keyboard layout, the map
from the character typed to the character on the same physical key of the US layout
(base and Shift levels). Tests and builds never run it; the output is checked in.

Needs `python3`, `xkbcli` (libxkbcommon-tools) and `libxkbcommon.so.0`, plus network access
on the first run to download the CLDR release-43 keyboard files into the cache dir.

```sh
python3 crates/core/tools/gen_layouts.py crates/core/src/layout/tables.rs \
    --cldr-cache /tmp/cldr-keyboards
```

Run `cargo fmt -p tomakast-core` afterwards: the checked-in file is rustfmt-formatted.

Sources: xkb tables for `us ru ua by kz kg tj uz(cyrillic) ge am`; CLDR release-43
`keyboards/osx/{ru,uk,be,kk,ka,hy}` and `keyboards/windows/{ru,uk,be,kk,ky,tg,uz,ka,hy}`
(`*-t-k0-*.xml`) against the English layouts of the same platform. A variant whose table
equals an already emitted one of the same script is dropped. Each table has a comment with
its source.

The xkb `ru(mac)` and `ua(macOS)` variants are deliberately not used: they differ from CLDR macOS only on non-letter ISO keys.
