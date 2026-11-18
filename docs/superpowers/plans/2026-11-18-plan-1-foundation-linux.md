# План 1: ядро, окно лаунчера и поиск приложений на Linux

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Демон на KDE Plasma (Wayland): по глобальному хоткею мгновенно открывает лаунчер, который ищет и запускает приложения из `.desktop`-файлов и ранжирует их по использованию (за всё время, за сегодня, по времени суток). Плюс переносимый бинарник-эксперимент окна, который пользователь прогоняет на macOS и Windows.

**Architecture:** Cargo workspace. В `tomakast-core` живут индекс, сопоставление, ранжирование, хранилище, трейт модулей и настройки, без зависимостей от GPUI. `tomakast-ui` — единственный крейт, который знает о GPUI: окно и представление списка. В `tomakast-platform` собраны ОС-шимы: глобальные хоткеи через XDG-портал и автозапуск. `tomakast-module-apps` — встроенный модуль приложений. `tomakast-app` — бинарник `tomakast` с CLI и IPC для единственного экземпляра.

**Tech Stack:** Rust 1.98 (edition 2024), gpui-kit `=0.10.0` (тянет gpui-pre `=0.3.12`), nucleo-matcher 0.3, rusqlite 0.43 (feature `bundled`), chrono 0.4, uuid 1 (`v4`), async-channel 2.5, interprocess 2.4, ashpd 0.14 (без tokio, на async-io), freedesktop-desktop-entry 0.8, freedesktop-icons 0.4, notify 8, directories 6, toml 1, serde 1, clap 4, bitflags 2, anyhow/thiserror, tracing + tracing-subscriber, tempfile (dev).

**Spec:** `docs/superpowers/specs/2026-11-18-tomakast-core-design.md`; дорожная карта — `docs/superpowers/plans/2026-11-18-roadmap.md`.

## Global Constraints

- Версии GPUI запинены точно: `gpui-kit = { version = "=0.10.0" }`. Других GPUI-крейтов напрямую не подключать: всё GPUI берётся через `gpui_kit::*`.
- `tomakast-core` не зависит от GPUI и не содержит платформенного кода. Типы GPUI не выходят за пределы `tomakast-ui`.
- GPUI на Linux по умолчанию завершает процесс при закрытии последнего окна. Демон обязан вызвать `cx.set_quit_mode(QuitMode::Explicit)`.
- Бюджеты из спецификации: тёплое открытие (хоткей → первый кадр) ~50 мс; нажатие клавиши → выдача на 10k элементов ≤ 16 мс, из них сопоставление и ранжирование ≤ 5 мс; со скрытым окном ~0% CPU и никаких перерисовок; RSS со скрытым окном ≤ ~200 МБ (замерять и не ухудшать).
- Целевая система этого плана — Linux, KDE Plasma на Wayland (layer-shell, портал `org.freedesktop.portal.GlobalShortcuts`). Остальные крейты должны собираться на macOS и Windows: платформенный код под `cfg`, на других ОС — заглушки, которые возвращают `Unsupported`.
- Каждая запись в хранилище имеет стабильный `id` и `modified_at` (задел под синхронизацию).
- Идентификатор элемента: `<provider-id>/<item-id>`, у приложений — `apps/<desktop-file-id>`.
- Логи пишутся через `tracing`, уровень задаётся `RUST_LOG`, по умолчанию `info`.

## Review Focus

1. **Битые или странные `.desktop`-файлы** (невалидный UTF-8, нет `Name`, кривые кавычки в `Exec`). Такую запись пропускаем и пишем в лог, остальное сканирование продолжается. Тест: Task 12 `scan_skips_broken_entries`.
2. **Второй запуск демона, пока первый работает.** Второй демон не стартует и сообщает, что уже запущен. Тест: Task 8 `second_server_reports_already_running`.
3. **Битый или нечитаемый `config.toml`.** Демон стартует с настройками по умолчанию и пишет ошибку в лог. Тест: Task 7 `load_or_default_survives_invalid_toml`.
4. **Кириллица и регистр в запросе.** Запрос «фай» находит «Файлы», «FIRE» находит «Firefox». Тест: Task 2 `matches_cyrillic_and_case_insensitive`.
5. **Приложение, которое удалили после сканирования.** Запуск возвращает ошибку, использование не записывается, окно остаётся открытым и показывает ошибку. Тест: Task 6 `failed_action_does_not_record_usage`.

---

### Task 1: Каркас workspace и CI

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`, `.gitignore`, `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/ui/Cargo.toml`, `crates/ui/src/lib.rs`, `crates/platform/Cargo.toml`, `crates/platform/src/lib.rs`, `modules/apps/Cargo.toml`, `modules/apps/src/lib.rs`, `crates/app/Cargo.toml`, `crates/app/src/main.rs`, `.github/workflows/ci.yml`

**Interfaces:**
- Produces: имена крейтов `tomakast-core` (lib `tomakast_core`), `tomakast-ui`, `tomakast-platform`, `tomakast-module-apps`, `tomakast-app` (bin `tomakast`). Общие версии зависимостей задаются в `[workspace.dependencies]`.

- [ ] **Step 1:** Создай workspace с пятью крейтами-пустышками. `rust-toolchain.toml`: `channel = "1.98"`. Все версии зависимостей из Tech Stack пропиши в `[workspace.dependencies]`, gpui-kit — как `"=0.10.0"`. `.gitignore`: `/target`.
- [ ] **Step 2:** В `tomakast-ui` подключи `gpui-kit`. Убедись, что `cargo build --workspace` проходит локально.
  Run: `cargo build --workspace` → Expected: успешная сборка.
- [ ] **Step 3:** Напиши `.github/workflows/ci.yml`: матрица `ubuntu-latest`, `macos-latest`, `windows-latest`; шаги `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. На Ubuntu до сборки ставь системные библиотеки для GPUI: начни с `libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig-dev libx11-xcb-dev libxcb1-dev clang`. Если сборка падает из-за отсутствующей `-sys`-библиотеки, добавь нужный пакет.
- [ ] **Step 4:** Run: `cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace` → Expected: всё зелёное.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "chore: workspace skeleton and CI"`

### Task 2: Индекс и сопоставление

**Files:**
- Create: `crates/core/src/index.rs`, `crates/core/src/matcher.rs`
- Modify: `crates/core/src/lib.rs` (pub mod + реэкспорт)
- Test: `crates/core/tests/matcher.rs`, `crates/core/tests/index.rs`

**Interfaces:**
- Produces:
  - `pub struct ItemId(String)` (Clone, Eq, Hash, Debug, Ord) с методами `ItemId::new(provider: &str, local: &str) -> ItemId`, `ItemId::parse(s: &str) -> Option<ItemId>` (требует `/`), `provider(&self) -> &str`, `as_str(&self) -> &str`.
  - `pub enum ItemKind { Application, Command }`
  - `pub enum Icon { File(PathBuf), Name(String) }`
  - `pub struct Action { pub id: String, pub title: String }`
  - `pub struct Item { pub id: ItemId, pub title: String, pub subtitle: Option<String>, pub keywords: Vec<String>, pub icon: Option<Icon>, pub kind: ItemKind, pub actions: Vec<Action> }`. Первое действие — основное, `actions` не пустой.
  - `pub struct Index` с методами `new() -> Index`, `replace_provider(&mut self, provider: &str, items: Vec<Item>)`, `get(&self, id: &ItemId) -> Option<&Arc<Item>>`, `len(&self) -> usize`, `iter(&self) -> impl Iterator<Item = &Arc<Item>>`.
  - `pub struct Pattern`, `Pattern::parse(query: &str) -> Pattern`, `Pattern::is_empty(&self) -> bool`.
  - `pub struct Matcher`, `Matcher::new() -> Matcher`, `score(&mut self, pattern: &Pattern, item: &Item) -> Option<u32>`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/index.rs
#[test] fn replace_provider_swaps_only_that_provider() {
    // index: apps/a, apps/b, calc/x; replace_provider("apps", [apps/c])
    // => len()==2, get(apps/c).is_some(), get(apps/a).is_none(), get(calc/x).is_some()
}
#[test] fn item_id_parse_requires_slash() {
    assert!(ItemId::parse("apps/firefox.desktop").is_some());
    assert!(ItemId::parse("firefox").is_none());
    assert_eq!(ItemId::new("apps", "x.desktop").provider(), "apps");
}
// tests/matcher.rs
#[test] fn title_match_scores_and_non_match_is_none() {
    // "fire" vs Item{title:"Firefox"} => Some(_); "zzz" => None
}
#[test] fn keyword_match_found_but_below_equal_title_match() {
    // A{title:"Browser"}, B{title:"Firefox", keywords:["browser"]}; query "browser"
    // => both Some, score(A) > score(B)
}
#[test] fn matches_cyrillic_and_case_insensitive() {
    // "фай" vs "Файлы" => Some; "FIRE" vs "Firefox" => Some
}
#[test] fn empty_pattern_matches_everything_with_zero() {
    // Pattern::parse("").is_empty(); score == Some(0)
}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core` → Expected: FAIL (типов ещё нет).
- [ ] **Step 3:** Реализуй `index.rs` и `matcher.rs`. `Index` хранит `HashMap<ItemId, Arc<Item>>`. `Matcher` — обёртка над `nucleo_matcher::Matcher` с `Config::DEFAULT`; `Pattern` — над `nucleo_matcher::pattern::Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)`. Score = максимум из score заголовка и (лучший score по ключевым словам × 3 / 4).
- [ ] **Step 4:** Run: `cargo test -p tomakast-core` → Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -am "feat(core): item index and fuzzy matcher"` (новые файлы добавь через `git add`).

### Task 3: Модель использования и ранжирование

**Files:**
- Create: `crates/core/src/usage.rs`
- Test: `crates/core/tests/usage.rs`

**Interfaces:**
- Consumes: `ItemId` (Task 2).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub struct LocalNow { pub unix: i64, pub local_day: i32, pub local_minute: u16 }`. Здесь `local_day` — число дней с 1970-01-01 в локальной зоне, `local_minute` — 0..1440. Конструкторы `LocalNow::now()` (через `chrono::Local`) и `LocalNow::from_unix_with_offset(unix: i64, offset_secs: i32) -> LocalNow`.
  - `pub struct UsageRecord { pub id: String, pub item: ItemId, pub query: String, pub at: LocalNow }`. `query` хранится в нижнем регистре; `id` — uuid v4 строкой. Конструктор `UsageRecord::new(item: ItemId, query: &str, at: LocalNow) -> UsageRecord`.
  - `pub struct UsageStats { pub frecency: f64, pub today: u32, pub time_of_day: u32, pub query_hits: u32 }`
  - `pub struct RankWeights { pub half_life_days: f64, pub time_window_minutes: u16, pub w_frecency: f64, pub w_today: f64, pub w_time_of_day: f64, pub w_query: f64, pub usage_scale: f64 }`. `Default`: 14.0, 90, 1.0, 2.0, 1.5, 4.0, 30.0.
  - `pub fn usage_value(s: &UsageStats, w: &RankWeights) -> f64` = `w_frecency*frecency + w_today*today + w_time_of_day*time_of_day + w_query*query_hits`.
  - `pub fn combined_score(match_score: u32, usage: f64, w: &RankWeights) -> f64` = `match_score as f64 + usage_scale * ln(1 + usage)`.
  - `pub struct UsageModel` с методами `new(records: Vec<UsageRecord>) -> UsageModel`, `push(&mut self, r: UsageRecord)`, `stats(&self, query: &str, now: LocalNow, w: &RankWeights) -> HashMap<ItemId, UsageStats>` (один проход по записям).

Как считать сигналы (одна запись `r`, текущее `now`, запрос `q` в нижнем регистре):
- frecency += `0.5^((now.unix - r.at.unix) / (half_life_days * 86400))`;
- today += 1, если `r.at.local_day == now.local_day`;
- time_of_day += 1, если круговая разница минут `min(|a-b|, 1440-|a-b|) <= time_window_minutes`;
- query_hits += 1, если `q` не пуст и `r.query.starts_with(q)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn frecency_halves_after_half_life() { /* record 14 days before now => stats.frecency ≈ 0.5 (eps 1e-9) */ }
#[test] fn today_counts_only_same_local_day() { /* records day=now.day and day-1 => today==1 */ }
#[test] fn time_of_day_window_wraps_midnight() { /* now minute 5, record minute 1430 => time_of_day==1; record minute 200 => 0 */ }
#[test] fn query_hits_use_prefix_of_recorded_query() { /* recorded "fir": q "fi" => 1, q "fo" => 0, q "" => 0 */ }
#[test] fn zero_usage_keeps_match_score() { assert_eq!(combined_score(120, 0.0, &RankWeights::default()), 120.0); }
#[test] fn from_unix_with_offset_computes_local_minute_and_day() {
    // unix 0 with offset +3h => local_minute 180, local_day 0; unix 0 with offset -1h => local_minute 1380, local_day -1
}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test usage` → Expected: FAIL.
- [ ] **Step 3:** Реализуй `usage.rs` по формулам выше.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test usage` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): usage model and ranking weights`.

### Task 4: Хранилище (SQLite)

**Files:**
- Create: `crates/core/src/store.rs`
- Test: `crates/core/tests/store.rs`

**Interfaces:**
- Consumes: `UsageRecord`, `LocalNow`, `ItemId`.
- Produces: `pub struct Store` с методами `Store::open(path: &Path) -> anyhow::Result<Store>`, `Store::open_in_memory() -> anyhow::Result<Store>`, `record_usage(&self, r: &UsageRecord) -> anyhow::Result<()>`, `load_usage(&self, since_unix: i64) -> anyhow::Result<Vec<UsageRecord>>` (по возрастанию `at_unix`).

Схема v1 (`PRAGMA user_version = 1`, `journal_mode = WAL`):
```sql
CREATE TABLE usage(
  id TEXT PRIMARY KEY, item_id TEXT NOT NULL, query TEXT NOT NULL,
  at_unix INTEGER NOT NULL, local_day INTEGER NOT NULL, local_minute INTEGER NOT NULL,
  modified_at INTEGER NOT NULL);
CREATE INDEX usage_at ON usage(at_unix);
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn roundtrip_usage_record() { /* in-memory: record, load_usage(0) == [same record] */ }
#[test] fn load_usage_filters_by_since() { /* records at 100 and 200; load_usage(150) -> only 200 */ }
#[test] fn file_store_persists_across_reopen() { /* tempfile dir; open, record, drop, open again, load -> 1 */ }
#[test] fn reopening_does_not_rerun_migration() { /* open twice on same file — no error, user_version == 1 */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test store` → Expected: FAIL.
- [ ] **Step 3:** Реализуй `store.rs` на rusqlite (`bundled`). Миграция: если `user_version == 0`, выполнить DDL и выставить 1. `modified_at` = `at.unix` на момент записи.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test store` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): sqlite store for usage history`.

### Task 5: Поисковый движок

**Files:**
- Create: `crates/core/src/engine.rs`
- Test: `crates/core/tests/engine.rs`

**Interfaces:**
- Consumes: Task 2 (`Index`, `Matcher`, `Pattern`, `Item`), Task 3 (`UsageModel`, `RankWeights`, `LocalNow`, `UsageRecord`, `usage_value`, `combined_score`).
- Produces:
  - `#[derive(Clone)] pub struct Hit { pub item: Arc<Item>, pub score: f64 }`
  - `pub struct SearchEngine` с методами `new(usage: UsageModel, weights: RankWeights) -> SearchEngine`, `index(&self) -> &Index`, `index_mut(&mut self) -> &mut Index`, `search(&mut self, query: &str, now: LocalNow) -> Vec<Hit>`, `record_launch(&mut self, item: &ItemId, query: &str, now: LocalNow) -> UsageRecord` (добавляет запись в модель и возвращает её для сохранения).

Порядок выдачи:
- **Пустой запрос:** все элементы; сортировка по `usage_value` по убыванию, при равенстве — по `title` без учёта регистра по возрастанию. `score` = `usage_value`.
- **Непустой запрос:** только совпавшие; сортировка по `combined_score` по убыванию, при равенстве — по `title`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn habit_beats_slightly_better_match() {
    // items "Firefox", "Fire Alarm"; 5× record_launch(Fire Alarm, "fi", now-1h)
    // search("fi", now)[0].item.title == "Fire Alarm"
}
#[test] fn empty_query_orders_by_usage_then_title() {
    // "Zed"(used 2×), "Alacritty"(0), "Btop"(0) => ["Zed","Alacritty","Btop"]
}
#[test] fn time_of_day_prefers_morning_app_in_morning() {
    // A launched at 09:00 on each of past 10 days; B at 21:00 on each of past 10 days
    // empty query at 09:10 => A first; at 21:05 => B first
}
#[test] fn non_matching_items_are_excluded() { /* query "zzz" => empty */ }
#[test] #[ignore] fn search_10k_under_5ms() {
    // 10_000 synthetic items + 1_000 usage records; for q in ["f","fi","fir","fire"]:
    // median of 20 runs of search(q) < 5 ms. Run: cargo test --release -p tomakast-core -- --ignored
}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test engine` → Expected: FAIL.
- [ ] **Step 3:** Реализуй `engine.rs`: на каждый поиск один раз вызови `UsageModel::stats`, затем пройди по индексу.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test engine && cargo test --release -p tomakast-core --test engine -- --ignored` → Expected: PASS, включая бюджет 5 мс.
- [ ] **Step 5:** Commit: `feat(core): search engine with usage-aware ranking`.

### Task 6: Модули и `Core`

**Files:**
- Create: `crates/core/src/module.rs`, `crates/core/src/core.rs`, `crates/core/src/testing.rs` (feature `testing`, используется и в Task 10)
- Test: `crates/core/tests/core.rs`

**Interfaces:**
- Consumes: Tasks 2–5.
- Produces:
  - `pub enum ActionOutcome { HideWindow, KeepOpen }`
  - `pub enum CoreEvent { ReplaceItems { provider: String, items: Vec<Item> } }`
  - `#[derive(Clone)] pub struct ModuleContext`, метод `replace_items(&self, items: Vec<Item>)`. Он отправляет `CoreEvent::ReplaceItems` с провайдером модуля, а элементы с чужим `id.provider()` отбрасывает с `warn!`.
  - `pub trait Module: Send + 'static { fn id(&self) -> &'static str; fn start(&mut self, ctx: ModuleContext); fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<ActionOutcome>; }`
  - `pub struct Core` с методами:
    - `new(store: Store, weights: RankWeights, now: LocalNow) -> anyhow::Result<Core>` — загружает использование за последние 180 дней;
    - `register(&mut self, m: Box<dyn Module>)`, `start_modules(&mut self)`;
    - `events(&self) -> async_channel::Receiver<CoreEvent>`, `apply(&mut self, ev: CoreEvent)`;
    - `search(&mut self, query: &str, now: LocalNow) -> Vec<Hit>`;
    - `execute(&mut self, item: &ItemId, action_id: &str, query: &str, now: LocalNow) -> anyhow::Result<ActionOutcome>`. Сначала `run_action` нужного модуля; только при успехе — `record_launch` в движок и `record_usage` в хранилище. Неизвестный элемент или модуль — `Err`.

- [ ] **Step 1: Write the failing tests** (с `tomakast_core::testing::FakeModule`: `FakeModule::new(items: Vec<Item>, result: fn() -> anyhow::Result<ActionOutcome>)`. В `start` он публикует `items`, а в `run_action` записывает `(item id, action id)` в `calls: Arc<Mutex<Vec<(ItemId, String)>>>` и возвращает `result()`):

```rust
#[test] fn module_items_become_searchable_after_apply() { /* start_modules; recv event; apply; search("") has 2 */ }
#[test] fn execute_records_usage_in_store_and_engine() { /* execute ok => store.load_usage(0).len()==1; empty-query order changes */ }
#[test] fn failed_action_does_not_record_usage() { /* run_action -> Err => execute is Err; load_usage(0) is empty */ }
#[test] fn foreign_provider_items_are_dropped() { /* module "fake" publishes id "other/x" => not in index */ }
#[test] fn execute_unknown_item_is_error() {}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test core` → Expected: FAIL.
- [ ] **Step 3:** Реализуй. Модули хранятся в `HashMap<String, Box<dyn Module>>`, канал — `async_channel::unbounded`.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): module trait and core orchestrator`.

### Task 7: Настройки, хоткеи и пути

**Files:**
- Create: `crates/core/src/settings.rs`, `crates/core/src/paths.rs`
- Test: `crates/core/tests/settings.rs`

**Interfaces:**
- Produces:
  - `bitflags! pub struct Modifiers: u8 { CTRL, ALT, SHIFT, SUPER }`
  - `pub struct Hotkey { pub mods: Modifiers, pub key: String }`. `key` — имя клавиши в нижнем регистре (`"space"`, `"t"`, `"f1"`). Методы: `Hotkey::parse(s: &str) -> Result<Hotkey, HotkeyError>`, `to_xdg_trigger(&self) -> String`, `Display` (`"Ctrl+Alt+T"`).
  - `pub struct Settings { pub launcher_hotkey: Hotkey, pub autostart: bool, pub hotkeys: Vec<(Hotkey, ItemId)> }`. `Default`: `Alt+Space`, `true`, пусто.
  - `Settings::load(path: &Path) -> anyhow::Result<Settings>` — если файла нет, возвращает `Default`.
  - `Settings::load_or_default(path: &Path) -> (Settings, Option<anyhow::Error>)`.
  - `pub struct Paths { pub config_file: PathBuf, pub data_dir: PathBuf, pub db_file: PathBuf }` с методами `Paths::detect() -> anyhow::Result<Paths>` (`directories::ProjectDirs::from("", "", "tomakast")`, `config_dir/config.toml`, `data_dir/tomakast.db`) и `ensure_dirs(&self) -> anyhow::Result<()>`.

Формат `config.toml`:
```toml
[launcher]
hotkey = "Alt+Space"
autostart = true

[hotkeys]
"Ctrl+Alt+F" = "apps/firefox.desktop"
```
XDG-триггер: модификаторы `CTRL`, `ALT`, `SHIFT`, `LOGO` (это Super) в таком порядке, затем имя клавиши XKB в нижнем регистре, разделитель `+`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn parse_and_format_hotkey() {
    let h = Hotkey::parse("ctrl+alt+t").unwrap();
    assert_eq!(h.to_string(), "Ctrl+Alt+T");
    assert_eq!(h.to_xdg_trigger(), "CTRL+ALT+t");
    assert_eq!(Hotkey::parse("Super+Space").unwrap().to_xdg_trigger(), "LOGO+space");
}
#[test] fn parse_rejects_bad_hotkeys() {
    for s in ["", "Ctrl+", "Ctrl+A+B", "Hyper+A"] { assert!(Hotkey::parse(s).is_err(), "{s}"); }
}
#[test] fn missing_file_gives_defaults() { /* load(nonexistent) == Settings::default() */ }
#[test] fn parses_hotkey_table() { /* sample above => hotkeys == [(Ctrl+Alt+F, apps/firefox.desktop)] */ }
#[test] fn load_or_default_survives_invalid_toml() { /* "[launcher" => (default, Some(err)) */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test settings` → Expected: FAIL.
- [ ] **Step 3:** Реализуй (serde + toml). Невалидный хоткей или id в `[hotkeys]` — ошибка загрузки с именем ключа в сообщении.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test settings` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): settings, hotkey parsing, paths`.

### Task 8: IPC и CLI для единственного экземпляра

**Files:**
- Create: `crates/app/src/ipc.rs`, `crates/app/src/cli.rs`
- Modify: `crates/app/src/main.rs`
- Test: `crates/app/tests/ipc.rs`

**Interfaces:**
- Consumes: `ItemId`.
- Produces:
  - `pub enum Request { Toggle, Run(ItemId), Ping }` с методами `encode(&self) -> String` (`"toggle"`, `"run <id>"`, `"ping"`) и `Request::decode(line: &str) -> anyhow::Result<Request>`.
  - `pub fn serve_on(name: &str, handler: impl Fn(Request) + Send + 'static) -> Result<ServerHandle, ServeError>`, где `ServeError::AlreadyRunning` возвращается, если имя занято живым сервером.
  - `pub fn send_to(name: &str, req: &Request) -> anyhow::Result<()>` — пишет строку, ждёт `"ok\n"`.
  - `pub const SOCKET_NAME: &str = "tomakast.sock"`; `serve`/`send` — обёртки с этим именем.
  - CLI (clap): без подкоманды — `send(Toggle)`, если не вышло, стартует демон; `daemon` — запуск демона, если демон уже работает, выходит с кодом 1; `toggle`; `run <ID>`.

Имя сокета: где поддерживается — `GenericNamespaced` (на Linux это abstract namespace, на Windows named pipe, устаревших файлов не бывает). Иначе (macOS) — `GenericFilePath` в `std::env::temp_dir()`. В этом случае при `AddrInUse` сначала пробуем `Ping`: если ответа нет, удаляем файл и биндимся заново.

- [ ] **Step 1: Write the failing tests** (уникальное имя на тест: `format!("tomakast-test-{}", uuid)`):

```rust
#[test] fn request_roundtrip() { /* Toggle, Ping, Run(apps/x.desktop) encode→decode */ }
#[test] fn decode_rejects_garbage() { /* "", "run", "jump x" => Err */ }
#[test] fn send_reaches_handler() { /* serve_on(name, push to channel); send_to(name, Toggle); recv == Toggle */ }
#[test] fn second_server_reports_already_running() { /* serve_on twice => Err(AlreadyRunning) */ }
#[test] fn send_without_server_errors() {}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-app --test ipc` → Expected: FAIL.
- [ ] **Step 3:** Реализуй на `interprocess::local_socket` (синхронный API, поток-акцептор).
- [ ] **Step 4:** Run: `cargo test -p tomakast-app --test ipc` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(app): single-instance IPC and CLI`.

### Task 9: Эксперимент с окном (ограничен по времени, с ручным гейтом)

Цель — прежде всех остальных UI-задач закрыть открытые вопросы спецификации про окно. Лимит — один рабочий день. Если за это время не уложились, задача останавливается: собираются замеры и отдаются пользователю.

**Files:**
- Create: `crates/ui/examples/window_probe.rs`, `docs/experiments/2026-11-window-probe.md`

**Interfaces:**
- Produces: выводы, которые записываются в `docs/experiments/2026-11-window-probe.md`. От них зависят Task 10 и Task 13:
  - пересоздавать окно или держать его постоянным;
  - помогает ли прогрев рендерера;
  - причина белой подложки и как её убрать.

Что умеет `window_probe`:
- Флаги:
  - `--strategy recreate|persistent`. `persistent` = окно не уничтожается и прячется средствами ОС. Если на текущей платформе это недоступно, печатаем `unsupported` и выходим.
  - `--prewarm` — при старте открыть и сразу закрыть окно 1×1.
  - `--root kit|plain` — открыть окно через `gpui_kit::open_window` или через голый `cx.open_window` без корня кита. Нужно, чтобы найти причину белой подложки.
  - `--cycles N` (по умолчанию 10), `--interval-ms` (по умолчанию 700).
- Окно 720×460: прозрачный фон, скругление 14 px, полупрозрачная заливка контента.
  - Linux/Wayland: layer-shell, слой `Overlay`, `KeyboardInteractivity::None`, чтобы эксперимент не перехватывал клавиатуру пользователя.
  - macOS и Windows: `WindowKind::PopUp`, без заголовка.
- Сам показывает и прячет окно `N` раз. На каждом показе замеряет время от запроса до первого `render` и до `window.on_next_frame`. В конце печатает таблицу: `first`, медиана, p95 для обеих метрик, RSS до и после.
- Режим `--idle 10`: 10 секунд со скрытым окном. Тики CPU берёт из `/proc/self/stat` на Linux, `getrusage` на macOS, `GetProcessTimes` на Windows.
- Всегда вызывает `cx.set_quit_mode(QuitMode::Explicit)`.

- [ ] **Step 1:** Реализуй `window_probe`. Run: `cargo run --release -p tomakast-ui --example window_probe -- --strategy recreate --prewarm` на KDE → Expected: печатается таблица; процесс не завершается, когда закрывается последнее окно.
- [ ] **Step 2:** Прогони на Linux матрицу `strategy × prewarm × root` и `--idle 10`. Результаты запиши в `docs/experiments/2026-11-window-probe.md`. Критерии:
  - медиана тёплого показа до `on_next_frame` ≤ 50 мс;
  - `first` с `--prewarm` ≤ 50 мс;
  - idle — 0 тиков CPU.
- [ ] **Step 3:** Найди причину белой подложки: сравни `--root kit` и `--root plain`, затем уточни внутри. Запиши причину и исправление. Если исправление требует патча GPUI, подключи его через `[patch.crates-io]` на форк, ссылку зафиксируй в документе.
- [ ] **Step 4 (ручной гейт, делает пользователь):** собрать и запустить `window_probe` на macOS и Windows. Отметить в документе:
  - таблицу замеров;
  - не крадёт ли окно фокус у текущего приложения (macOS);
  - нет ли иконки в таскбаре и Alt-Tab (Windows);
  - прозрачность и скругления без подложки.

  Если критерии не выполнены, действуем по запасному плану из спецификации: форк платформенного бэкенда, затем обсуждение с пользователем. Task 10 можно начинать после шагов 2–3, не дожидаясь этого гейта.
- [ ] **Step 5:** Commit: `chore(ui): window probe experiment and findings`.

### Task 10: Окно и представление лаунчера

**Files:**
- Create: `crates/ui/src/launcher_view.rs`, `crates/ui/src/window.rs`, `crates/ui/src/theme.rs`
- Modify: `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/launcher_view.rs` (headless-тесты из `gpui_kit::test`; как ими пользоваться, описано в `TESTING.md` крейта gpui-kit 0.10.0 в `~/.cargo/registry/src/*/gpui-kit-0.10.0/TESTING.md`)

**Interfaces:**
- Consumes: `Core`, `Hit`, `LocalNow`, `ActionOutcome`, `CoreEvent` (Task 6); выводы Task 9.
- Produces:
  - `pub struct LauncherController` с методами:
    - `new(core: Rc<RefCell<Core>>) -> LauncherController`;
    - `prewarm(&mut self, cx: &mut App)`;
    - `show(&mut self, cx: &mut App)`, `hide(&mut self, cx: &mut App)`, `toggle(&mut self, cx: &mut App)`;
    - `is_visible(&self) -> bool`;
    - `core_changed(&mut self, cx: &mut App)` — пересчитать выдачу открытого окна.

    Хранится в `gpui::Global`.
  - `pub fn init(cx: &mut App)` — биндинги клавиш в контексте `"Launcher"`: `up` → `SelectPrev`, `down` → `SelectNext`, `enter` → `Confirm`, `escape` → `Dismiss`; тема по системной (`window.appearance()`) со слежением за её сменой.
  - `pub struct LauncherView` — поле ввода (placeholder «Search apps and commands…») и виртуализированный список `Hit`. В строке: иконка 24 px (`Icon::File` → `img(path)`, без иконки — пустое место), `title`, `subtitle` приглушённым цветом. Выделенная строка — фон `list_active`. Внизу строка для ошибки.

Поведение:
- Каждый ввод → `core.search(query, LocalNow::now())`. Выделение сбрасывается на 0, список прокручивается к началу.
- `Confirm` → `core.execute(id, actions[0].id, query, now)`. При `Ok(HideWindow)` — `hide`. При `Err(e)` окно остаётся открытым, в нижней строке на 4 с появляется `e.to_string()`.
- `Dismiss` и потеря активности окна (там, где платформа об этом сообщает) → `hide`.
- Каждый `show` начинается с пустого запроса; в поле ввода сразу фокус.
- Окно и прозрачность — по выводам Task 9. В показе ведётся замер «запрос показа → `on_next_frame`», уровень `debug`.

- [ ] **Step 1: Write the failing tests** (headless, `Core` с `tomakast_core::testing::FakeModule` из Task 6):

```rust
// typing_filters_results: ввести "fir" => строк 1, title "Firefox"
// enter_executes_selected_and_hides: down, enter => FakeModule получил run_action для 2-го элемента; controller.is_visible()==false
// escape_hides: escape => is_visible()==false
// error_keeps_window_and_shows_message: run_action -> Err("gone") => is_visible()==true, в выводе есть "gone"
// show_resets_query: ввести "x", hide, show => поле пустое
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-ui` → Expected: FAIL.
- [ ] **Step 3:** Реализуй `window.rs`, `launcher_view.rs`, `theme.rs`.
- [ ] **Step 4:** Run: `cargo test -p tomakast-ui` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(ui): launcher window and list view`.

### Task 11: Глобальные хоткеи (Linux, портал) и автозапуск

**Files:**
- Create: `crates/platform/src/hotkeys.rs`, `crates/platform/src/hotkeys/linux.rs`, `crates/platform/src/autostart.rs`
- Test: `crates/platform/tests/hotkeys.rs`, `crates/platform/tests/autostart.rs`

**Interfaces:**
- Consumes: `Hotkey`, `Hotkey::to_xdg_trigger` (Task 7).
- Produces:
  - `pub struct HotkeyBinding { pub id: String, pub description: String, pub hotkey: Hotkey }`. `id` — `"launcher"` или `ItemId::as_str()`.
  - `pub enum HotkeyBackend { Portal, Unsupported { reason: String } }`
  - `pub fn register_global_hotkeys(bindings: Vec<HotkeyBinding>, on_activate: impl Fn(String) + Send + 'static) -> HotkeyBackend`. Запускает поток с `async_io::block_on`. Через ashpd `GlobalShortcuts`: `create_session` → `bind_shortcuts(NewShortcut::new(id, description).preferred_trigger(trigger))` → поток `receive_activated()` → `on_activate(shortcut_id)`. Если портала нет или привязка не удалась, возвращает `Unsupported` с причиной. На не-Linux — заглушка `Unsupported`.
  - `pub fn portal_shortcuts(bindings: &[HotkeyBinding]) -> Vec<(String, String, String)>` — чистая функция, возвращает (id, description, trigger).
  - `pub fn set_autostart_in(dir: &Path, enabled: bool, exe: &Path) -> anyhow::Result<()>` и `pub fn set_autostart(enabled: bool) -> anyhow::Result<()>`. На Linux пишет или удаляет `$XDG_CONFIG_HOME/autostart/tomakast.desktop` с содержимым `[Desktop Entry]`, `Type=Application`, `Name=tomakast`, `Exec="<exe>" daemon`, `X-GNOME-Autostart-enabled=true`. На остальных ОС — `Ok(())` с `warn!`; реальная реализация будет в планах 3 и 4.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn portal_shortcuts_use_xdg_triggers() { /* launcher Alt+Space => ("launcher", "Open tomakast", "ALT+space") */ }
#[test] fn autostart_writes_and_removes_desktop_file() { /* tempdir; enabled=true => file contains `Exec="/x/tomakast" daemon`; enabled=false => file gone */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-platform` → Expected: FAIL.
- [ ] **Step 3:** Реализуй. ashpd подключи с `default-features = false, features = ["async-io"]` и нужной фичей `global_shortcuts`, если в 0.14 она отделена (проверь в `Cargo.toml` крейта).
- [ ] **Step 4:** Run: `cargo test -p tomakast-platform` → Expected: PASS. Ручная проверка на KDE: временный `examples/hotkey_probe.rs` регистрирует `Alt+Space`. KDE показывает диалог подтверждения, после него нажатие печатает `activated launcher`.
- [ ] **Step 5:** Commit: `feat(platform): portal global shortcuts and XDG autostart`.

### Task 12: Модуль `apps` (Linux)

**Files:**
- Create: `modules/apps/src/lib.rs`, `modules/apps/src/linux.rs`, `modules/apps/tests/fixtures/{user,system}/…*.desktop`
- Test: `modules/apps/tests/scan.rs`

**Interfaces:**
- Consumes: `Module`, `ModuleContext`, `Item`, `ItemId`, `Icon`, `Action`, `ActionOutcome` (Tasks 2, 6).
- Produces:
  - `pub struct AppsModule`, `AppsModule::new() -> AppsModule`, `impl Module` с `id() == "apps"`.
  - `pub struct ScanOptions { pub dirs: Vec<PathBuf>, pub locales: Vec<String>, pub current_desktops: Vec<String>, pub path_env: Option<OsString> }`, `ScanOptions::from_env() -> ScanOptions`.
  - `pub struct AppEntry { pub desktop_file: PathBuf, pub exec: Vec<String>, pub terminal: bool, pub working_dir: Option<PathBuf> }`
  - `pub fn scan(opts: &ScanOptions) -> Vec<(Item, AppEntry)>`
  - `pub fn launch(entry: &AppEntry) -> anyhow::Result<()>`

Правила сканирования:
- Каталоги в порядке приоритета:
  1. `$XDG_DATA_HOME/applications` (по умолчанию `~/.local/share/applications`);
  2. `<каждый из $XDG_DATA_DIRS>/applications` (по умолчанию `/usr/local/share:/usr/share`);
  3. затем, если существуют и ещё не включены: `~/.local/share/flatpak/exports/share/applications`, `/var/lib/flatpak/exports/share/applications`, `/var/lib/snapd/desktop/applications`.

  Рекурсивно.
- Desktop file id — путь относительно каталога, `/` заменяется на `-`. При дубликатах побеждает первый каталог.
- Пропускаем записи:
  - с `Type` ≠ `Application`, `NoDisplay=true`, `Hidden=true`;
  - с `OnlyShowIn`, где нет ни одного из `current_desktops` (из `$XDG_CURRENT_DESKTOP`, разделитель `:`);
  - с `NotShowIn`, где есть любой из них;
  - с `TryExec`, который не находится в `path_env`;
  - без `Name`, без `Exec`, а также если `parse_exec()` вернул ошибку (запись в лог).
- Элемент:
  - `id = apps/<desktop-file-id>`, `title` = локализованный `Name`;
  - `subtitle` = `GenericName`, иначе `Comment`;
  - `keywords` = `Keywords` + `GenericName` + basename первого аргумента `Exec`;
  - `kind = Application`, `actions = [Action { id: "open", title: "Open" }]`;
  - `icon`: абсолютный путь → `Icon::File`; имя → поиск через `freedesktop-icons` (`lookup(name).with_size(48).find()`), найдено → `Icon::File(path)`, иначе `None`.
- Локали — из `LC_ALL`, иначе `LC_MESSAGES`, иначе `LANG`: `ru_RU.UTF-8` → `["ru_RU", "ru"]`.
- `exec` — из `DesktopEntry::parse_exec()` (field codes вроде `%u`/`%f` удаляет крейт).

Запуск:
- `terminal == true` → `gio launch <desktop_file>`; если `gio` нет, возвращаем ошибку «terminal apps need gio».
- Иначе `Command::new(exec[0]).args(&exec[1..])`: `current_dir(working_dir)`, stdio = null, `process_group(0)`, без ожидания завершения. Ошибка spawn → `Err` с именем приложения.
- `run_action("open")` → `launch` → `Ok(HideWindow)`. Неизвестный action → `Err`.

Обновление: `notify`-watcher на все существующие каталоги из списка, debounce 500 мс → `scan` → `ctx.replace_items`. Первое сканирование и watcher работают в фоновом потоке, созданном в `start`.

- [ ] **Step 1: Write the failing tests** на фикстурах (два каталога: `user` и `system`):

```rust
#[test] fn scan_basic_entry() { /* firefox.desktop: Exec="firefox %u" => title "Firefox", exec == ["firefox"], id "apps/firefox.desktop" */ }
#[test] fn scan_uses_locale_name() { /* Name[ru]=Файлы, locales ["ru_RU","ru"] => title "Файлы" */ }
#[test] fn scan_filters_hidden_nodisplay_onlyshowin_tryexec() { /* 4 fixtures with current_desktops ["KDE"], path_env empty dir => none present */ }
#[test] fn user_dir_overrides_system_duplicate() { /* same id in both => title from user */ }
#[test] fn subdir_desktop_id_uses_dashes() { /* system/kde/konsole.desktop => id "apps/kde-konsole.desktop" */ }
#[test] fn scan_skips_broken_entries() { /* invalid UTF-8 file, file without Name, Exec with unbalanced quote => skipped, others still returned */ }
#[test] fn launch_missing_binary_errors() { /* exec ["/nonexistent/bin"] => Err */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-module-apps` → Expected: FAIL.
- [ ] **Step 3:** Реализуй `linux.rs` (под `cfg(target_os = "linux")`). В `lib.rs` на других ОС `scan` возвращает пустой список; это временно, до планов 3 и 4.
- [ ] **Step 4:** Run: `cargo test -p tomakast-module-apps` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(apps): desktop entry scanning and launching on Linux`.

### Task 13: Сборка демона и приёмка

**Files:**
- Create: `crates/app/src/daemon.rs`, `docs/experiments/2026-11-plan1-acceptance.md`
- Modify: `crates/app/src/main.rs`

**Interfaces:**
- Consumes: всё выше.

`daemon::run(paths: Paths) -> anyhow::Result<()>`, порядок запуска:
1. Инициализировать `tracing`.
2. `Paths::ensure_dirs`.
3. `Settings::load_or_default`; если есть ошибка — `error!`, работаем дальше.
4. `Store::open(db_file)` → `Core::new`.
5. `register(AppsModule)` → `start_modules`.
6. `serve(handler)` — при `AlreadyRunning` выход с кодом 1 и сообщением. Обработчик пересылает `Request` в `async_channel`.
7. `gpui_kit::application().run`. Внутри:
   - `gpui_kit::init`, `cx.set_quit_mode(QuitMode::Explicit)`, `tomakast_ui::init`;
   - `LauncherController::new` + `prewarm`;
   - две задачи `cx.spawn`. Первая читает `core.events()` → `core.apply` → `controller.core_changed`. Вторая читает IPC-запросы: `Toggle` → `toggle`; `Run(id)` → `core.execute(id, primary, "", now)`, ошибка — в лог;
   - `register_global_hotkeys`: `launcher` + все `settings.hotkeys`. Активация пересылается в тот же канал: `"launcher"` → `Toggle`, остальное → `Run`. Описание биндинга лаунчера — `"Open tomakast"`, у хоткея приложения — `title` элемента. Если вернулся `Unsupported`, пишем `warn!` с подсказкой: «назначь в настройках системы команду `tomakast toggle`».
8. `set_autostart(settings.autostart)`; ошибка — в лог.

- [ ] **Step 1:** Реализуй `daemon.rs` и подключи к CLI из Task 8.
  Run: `cargo build --release -p tomakast-app` → Expected: успешная сборка.
- [ ] **Step 2:** Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings` → Expected: PASS.
- [ ] **Step 3 (ручная приёмка на KDE, вместе с пользователем):** запусти `target/release/tomakast daemon`. Результаты запиши в `docs/experiments/2026-11-plan1-acceptance.md`.
  - Alt+Space после подтверждения в диалоге KDE открывает лаунчер; повторное нажатие или Esc закрывает его.
  - «fir» → Firefox первым; Enter запускает и закрывает окно.
  - После нескольких запусков одного приложения оно поднимается в пустом запросе.
  - Хоткей из `[hotkeys]` запускает приложение, не открывая лаунчер.
  - `tomakast toggle` работает. Второй `tomakast daemon` выходит с кодом 1.
  - Новый `.desktop` в `~/.local/share/applications` появляется в поиске без перезапуска.
  - Замеры: медиана тёплого открытия по логу `debug` (≤ ~50 мс); первое открытие после старта; CPU за 10 с со скрытым окном (0 тиков); RSS со скрытым окном (≤ ~200 МБ).
  - Битый `config.toml` → демон стартует, в логе ошибка.
  - После перелогина демон стартует сам.
- [ ] **Step 4:** Commit: `feat(app): daemon wiring and plan-1 acceptance notes`.
