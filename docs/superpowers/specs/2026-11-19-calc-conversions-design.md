# calc: more conversions (short design)

Date: 2026-11-19. Status: approved by the user (2026-11-19). Source: the calc feedback session.

**Why.** The user wants calc to convert "about as much as Raycast": time, currencies, the current time in other time zones (a `now` variable), and so on.

**Goals.** Everything below works in the search answer and in sheets. Expressions are English only (the user's call). Search rules stay: a bare value (`10 km`, `100 usd`) gets no answer, a conversion or an operation does.

- **More units**, each with its full names and plurals as aliases (`km`, `kilometer`, `kilometers`):
  - temperature, `C F K`, which is affine: an offset as well as a factor;
  - area, `mm2 cm2 m2 km2 ha acre ft2 in2 mi2`;
  - volume, `ml l m3 tsp tbsp floz cup pt qt gal`, US measures;
  - speed, `m/s km/h mph kn`;
  - data, `bit B KB MB GB TB` (decimal) and `KiB MiB GiB TiB` (binary);
  - angle, `deg rad`;
  - time, the existing units plus `month` and `year`, calendar-aware when added to a date.
- **Dates and times:**
  - `now`, `today`, `tomorrow`, `yesterday`;
  - date arithmetic: `now + 2 weeks`, `today - 3 days`;
  - differences: `days until 31 dec`, `dec 31 - today`;
  - time zones: `now in Tokyo`, `time in London`, `5pm London in Moscow`, `15:00 MSK in PST`;
  - unix time: `1700000000 to date`, `now to unix`.
- **Currencies:**
  - fiat by ISO code or symbol: `100 usd in rub`, `$100 to €`, `50 eur + 20 usd in gbp`; an ambiguous symbol takes its most common meaning (`¥` is JPY);
  - crypto by ticker: `0.5 btc in usd`, `1 eth in btc`.
- **Number bases:** `0xff`, `0b1010`, `0o17`, `255 in hex`, `255 in bin`, `255 in oct`.

**Non-goals.**
- Other languages.
- Stocks.
- Historical rates.
- Natural-language dates beyond the shapes above.

**Constraints and calls (WIT 0.1, no contract change).**

*Your time zone* (my call):
- WASI gives only UTC, and no import tells the user's time zone (WIT 0.2 candidate G6).
- Until then, calc gets a text preference `timezone` (IANA, default `UTC`) in its `plugin.toml`. `now` uses it.
- The answer shows the zone it used, so a wrong default is visible.
- Once the contract is edited in place for widgets, a `local-time-zone` import is the natural companion change; that is up to the widgets plan.

*Time zone data* (my call):
- A tz database is bundled in the wasm (for example `jiff` with its bundled tzdb).
- Cities resolve through a bundled table: IANA zone names, the common abbreviations, and a list of the largest cities and capitals.
- The builder measures the wasm size and keeps it reasonable (a few hundred KiB more is acceptable).

*Rates* (user: `open.er-api.com` for fiat, crypto too; CoinGecko is my call for crypto):
- Fiat comes from `open.er-api.com/v6/latest/USD` (free, no key, daily, has RUB).
- Crypto comes from `api.coingecko.com/api/v3/simple/price`, for a bundled ticker → CoinGecko id table of the top coins.
- Calc fetches at `init`, then on `timers`: fiat hourly, crypto every 10 minutes while recently used.
- It keeps the last rates and their time in `storage`.
- `search` never waits for the network: it converts with the stored rates. With none yet, a currency query gets no answer.
- An answer with rates older than a day says so in its subtitle.

*Permissions:*
- calc adds `http = ["open.er-api.com", "api.coingecko.com"]` and `timers`;
- the permissions grow, so the user is asked again once (expected).

*Formatting:*
- dates like `Thu, 19 Nov 2026, 17:42 MSK`;
- durations like `41 days`;
- money with two decimals (crypto up to 8 significant digits) and the code (`9 312.50 RUB`).

**Approaches.**
- *Chosen:* extend calc's own engine (`plugins/calc/src/engine/`). It already has the lexer, parser, units and the search heuristics, and stays pure Rust tested natively.
- *Rejected:* an embedded third-party calculator crate (fend, numbat). It is larger, its syntax and errors are not ours, its heuristics are a poor fit for the "clearly math" search rule, and it would duplicate the existing tests.

**Left to the builder.**
- The exact alias lists.
- The city table's size.
- Rate refresh details within the above.
- Rounding rules.
- How unit dimensions are represented (a new `Dim` per family, with temperature special-cased).
