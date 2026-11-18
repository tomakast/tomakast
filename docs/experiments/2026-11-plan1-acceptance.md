# Plan 1 acceptance (Task 13)

Date: 2026-11-18. Binary: `target/release/tomakast` built from branch `plan-1-foundation` (`cargo build --release -p tomakast-app`).
Machine: the Task 9 setup (KDE Plasma 6.8.3, Wayland, one 3840×2160 output at 240 Hz, scale 1.5, NVIDIA RTX 5070 Ti on Vulkan).

This document has two parts:
1. [Agent measurements](#agent-measurements): what the implementing agent ran itself, with `TOMAKAST_HOME=<temp dir>` and `TOMAKAST_NO_KEYBOARD=1`. With these settings nothing grabs the keyboard, no portal shortcut is bound, and autostart is not written.
2. [User checklist](#user-checklist): the interactive checks (typing, hotkeys, focus, re-login) for the user. Its result cells are blank.

## Agent measurements

All runs: `TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk13.XXXX)` (socket path 38 bytes), `TOMAKAST_NO_KEYBOARD=1`, `RUST_LOG=info,tomakast_ui=debug`, release build.
The real app directories were scanned (read-only). Driver script: `tomakast daemon &`, wait for the socket and 3 s more, idle 10 s, then a loop:

```bash
for i in $(seq 1 3000); do
  tomakast toggle; sleep 0.05   # show
  tomakast toggle; sleep 0.05   # hide
  # every 100 shows: sleep 0.3, then sample /proc/$PID/status (window hidden)
done
```

Then idle 10 s, a second `tomakast daemon`, `tomakast run apps/does-not-exist.desktop`, and kill.
CPU ticks are `utime+stime` from `/proc/$PID/stat` (100 ticks/s). Show latency is the `launcher show request -> on_next_frame` debug line: show request → first frame callback, the same metric as Task 9.

### Long-run RSS (3000 shows, 6000 `tomakast toggle` calls, 0 failures)

All samples are taken with the window hidden.

| Shows | RSS | of which anon | file-backed | shmem |
|---|---|---|---|---|
| 0 (after start + prewarm) | 187.7 | 32.8 | 141.6 | 13.2 |
| 1 | 196.2 | 36.6 | 146.3 | 13.2 |
| 100 | 212.1 | 38.7 | 153.2 | 20.1 |
| 200 | 220.3 | 38.9 | 157.2 | 24.1 |
| 500 | 220.6 | 39.2 | 157.2 | 24.1 |
| 1000 | 220.9 | 39.6 | 157.2 | 24.1 |
| 1500 | 221.5 | 40.2 | 157.2 | 24.1 |
| 2000 | 221.7 | 40.3 | 157.2 | 24.1 |
| 2500 | 222.5 | 41.2 | 157.2 | 24.1 |
| 3000 | 222.8 | 41.4 | 157.2 | 24.1 |

All values are in MiB: KiB from `/proc/$PID/status` (`VmRSS`, `RssAnon`, `RssFile`, `RssShmem`) divided by 1024. This is the unit the Task 9 probe prints as "MB".

Verdict against the Task 9 threshold ("RSS hidden ≤ 200 MB, **or** a plateau < +5 MB over the last 1000 shows"):
- **≤ 200 MiB: ❌ not met.** Hidden RSS is 220–223 MiB from show 200 on.
- **Plateau: ✅ met.** The last 1000 shows added +1.1 MiB (221.7 → 222.8), and shows 200–3000 added +2.5 MiB in all. The threshold is passed on this criterion, so no GPUI layer-surface patch is triggered.
- About 157 MiB is file-backed and 24 MiB is shmem. Both stop growing by show 200. Anonymous memory is 41 MiB at the end.
  - Hypothesis, not checked against `smaps`: the file-backed part is mostly mapped GPU driver and Vulkan libraries, and shmem is GPU and Wayland buffers.
- ⚠️ Anonymous memory still grows slowly and steadily: about +0.9 MiB per 1000 shows between shows 1000 and 3000, i.e. ~1 KiB per show. It does not flatten within 3000 shows. At a few dozen shows a day that is a few MiB per year, so it is harmless for now. Its cause was not profiled.
- The spec budget "RSS hidden ≤ ~200 MB" is exceeded by ~11%.
  - Most of the memory comes with the first window, as the Task 9 probe showed (184–189 MiB after prewarm, 211 MiB after 400 shows).
  - The real daemon tracks the probe: 187.7 MiB after prewarm, 220.6–220.9 MiB after 500–1000 shows.
- A first short run right after the build (20 shows) showed 247 MiB right after start and a 606 ms prewarm. This matches the probe's one-off first-run 244 MB; the cause is not verified, possibly a driver or pipeline cache being created. The long run, the second run after the build, started at 187.7 MiB with a 186 ms prewarm.

### Latency

| Metric | Target | Result |
|---|---|---|
| Prewarm (main thread, at startup) | — | 186 ms (606 ms on the first run after the build) |
| First open after startup → `on_next_frame` | ≤ ~50 ms | 33.9 ms (31.8 ms in the short run) ✅ |
| Warm open, median over 2999 shows | ≤ ~50 ms | 17.8 ms ✅ (reproduced with the A11 command on the raw log) |
| Warm open, p95 / p99 / max | — | 19.4 / 20.7 / 25.7 ms; no show over 50 ms |
| Warm median by thousand (1–999, 1000–1999, 2000–2999) | no drift | 17.9 / 17.7 / 17.7 ms ✅ |

The first open is ~10 ms slower than the probe's (22–26 ms) and ~16 ms slower than a warm open. The cause was not profiled. Warm opens also rebuild the view and run the search, so that alone does not explain it; first-use costs such as glyph rasterisation, icon decoding and shader or pipeline setup are candidates. It is within budget.

### Other checks run by the agent

| Check | Result |
|---|---|
| CPU over 10 s, window hidden, before the shows | 0 ticks ✅ |
| CPU over 10 s, window hidden, after 3000 shows | 0 ticks ✅ |
| Second `tomakast daemon` while one runs | prints `tomakast: another tomakast daemon is already running`, exit code 1 ✅ |
| `tomakast run apps/does-not-exist.desktop` | client exit 0 (the request was delivered); the daemon logs `ERROR … cannot run item: unknown item apps/does-not-exist.desktop` ✅ |
| Broken `config.toml` (`launcher = [`) | The daemon starts and `tomakast toggle` works (exit 0). The log has `ERROR … loading …/config.toml: invalid TOML: TOML parse error at line 1, column 13 …` ✅ |
| `TOMAKAST_HOME` set | Logs `autostart is left untouched`; `~/.config/autostart` has no tomakast entry ✅ |
| `TOMAKAST_NO_KEYBOARD=1` | Logs `global hotkeys are not registered`; no portal call, no KDE dialog ✅ |

Log noise seen on every start, not ours:
- `ERROR freedesktop_desktop_entry::exec: Unknown field code: '%%F'`: an installed `.desktop` file has a literal `%%F`;
- `WARN gpui::text_system: failed to load font ".SystemUIFont"`, followed by gpui-component resolving it to Adwaita Sans.


## User checklist

The commands are bash. If your shell is fish, run `bash` first. Run them from the worktree root.

### 0. Setup

```bash
cargo build --release -p tomakast-app
export TK="$PWD/target/release/tomakast"
```

Part A uses a throwaway home: `TOMAKAST_HOME` moves the config, the database and the socket there and disables autostart.
The keyboard is grabbed normally, because `TOMAKAST_NO_KEYBOARD` is *not* set.
Global hotkeys are registered in Part A too, but they are only checked in Part B.

```bash
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-accept.XXXX)
unset TOMAKAST_NO_KEYBOARD
RUST_LOG=info,tomakast_ui=debug "$TK" daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!
```

If KDE shows the shortcut confirmation dialog now, accept it.

### Part A: with `TOMAKAST_HOME`

| # | Check | How | Expected | Result |
|---|---|---|---|---|
| A1 | `tomakast toggle` | `"$TK" toggle`, then `"$TK" toggle` again | The first command opens the launcher with an empty focused query; the second closes it. Both exit 0. | |
| A2 | Esc closes | `"$TK" toggle`, then press Esc | The launcher closes. | |
| A3 | Click outside / focus loss closes | `"$TK" toggle`, then click another window | The launcher closes. If it does not, note it: deactivation may be undetectable on a KDE `Exclusive` layer surface (see the Task 9 notes). | |
| A4 | Second daemon | `"$TK" daemon; echo "exit=$?"` | It prints `tomakast: another tomakast daemon is already running` and `exit=1`. The first daemon keeps running. | |
| A5 | "fir" → Firefox | `"$TK" toggle`, type `fir` | Firefox is the first row. | |
| A6 | Enter launches | Press Enter on Firefox | Firefox starts, the launcher closes, and Firefox has the keyboard focus. | |
| A7 | No zombies | Close Firefox, wait one minute, run `ps -o pid,stat,comm --ppid "$DAEMON"` | No line with `Z` in `STAT`. | |
| A8 | Usage ranking | Launch the same app (e.g. KCalc: type `kcalc`, Enter) 3–4 times, then `"$TK" toggle` with an empty query | That app is at or near the top of the empty-query list. | |
| A9 | `tomakast run` | `"$TK" run apps/org.kde.kcalc.desktop` | KCalc starts without the launcher opening. (`tomakast run` exits 0 once the daemon has accepted the request. A failure such as an unknown id is only logged by the daemon.) | |
| A10 | New `.desktop` without restart | `printf '[Desktop Entry]\nType=Application\nName=Tomakast Probe Entry\nExec=true\n' > ~/.local/share/applications/tomakast-probe.desktop`, wait ~2 s, `"$TK" toggle`, type `probe entry` | "Tomakast Probe Entry" is found. Afterwards: `rm ~/.local/share/applications/tomakast-probe.desktop`. | |
| A11 | Warm open (median) | Open and close the launcher ~20 times with `"$TK" toggle` (or Esc), then run the median command below | The median is ≤ ~50 ms. | |
| A12 | First open after startup | The first `on_next_frame` line after daemon start (command below) | ≤ ~50 ms is the target. Record the value. | |
| A13 | Idle CPU, window hidden | With the launcher closed: `a=$(awk '{print $14+$15}' /proc/$DAEMON/stat); sleep 10; b=$(awk '{print $14+$15}' /proc/$DAEMON/stat); echo $((b-a))` | `0` ticks. | |
| A14 | RSS, window hidden | `grep VmRSS /proc/$DAEMON/status` with the launcher closed (the value is in KiB; divide by 1024 for MiB) | Budget ≤ ~200 MiB. Record the value; see the agent measurements. | |

Median and first-open commands for A11/A12:

```bash
# The log has ANSI colour codes even when redirected; LC_ALL=C keeps `sort -n` from
# misreading the decimal point under a non-English LC_NUMERIC.
sed 's/\x1b\[[0-9;]*m//g' "$TOMAKAST_HOME/daemon.log" \
  | sed -n 's/.*on_next_frame ms=\([0-9.]*\).*/\1/p' > /tmp/tk-frames.txt
echo "first: $(head -1 /tmp/tk-frames.txt) ms"
tail -n +2 /tmp/tk-frames.txt | LC_ALL=C sort -n \
  | awk '{a[NR]=$1} END{print "warm median: " a[int((NR+1)/2)] " ms over " NR " shows"}'
```

Broken config (A15) uses a second throwaway home, so it can run while the first daemon is alive:

| # | Check | How | Expected | Result |
|---|---|---|---|---|
| A15 | Broken `config.toml` | `H=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-bad.XXXX); echo 'launcher = [' > "$H/config.toml"; TOMAKAST_HOME="$H" TOMAKAST_NO_KEYBOARD=1 "$TK" daemon 2> "$H/daemon.log" & P=$!; sleep 3; TOMAKAST_HOME="$H" "$TK" toggle; echo "toggle exit=$?"; grep ERROR "$H/daemon.log"; kill $P; rm -rf "$H"` | The daemon starts. `toggle exit=0`, and a window flashes up without taking the keyboard. The log has an `ERROR` line starting `loading …/config.toml: invalid TOML: TOML parse error at line 1`; the parser's message continues on the following lines and ends with `; using the default settings`. | |

Stop the Part A daemon:

```bash
kill "$DAEMON"; rm -rf "$TOMAKAST_HOME"; unset TOMAKAST_HOME
```

### Part B: real hotkeys and autostart (no `TOMAKAST_HOME`)

Part B uses the real config `~/.config/tomakast/config.toml` and the real database under `~/.local/share/tomakast/`.
The daemon also writes `~/.config/autostart/tomakast.desktop`, because `autostart` defaults to `true`.

Back up an existing config and autostart entry first. The config step below overwrites the config, and the daemon overwrites the autostart entry. A missing file is simply not backed up.

```bash
BK=$(mktemp -d -p ~ tk-backup.XXXX); echo "backup dir: $BK"
[ -f ~/.config/tomakast/config.toml ] && cp -p ~/.config/tomakast/config.toml "$BK/"
[ -f ~/.config/autostart/tomakast.desktop ] && cp -p ~/.config/autostart/tomakast.desktop "$BK/"
ls -A "$BK"
```

Create the config with one app hotkey. Ctrl+Alt+C is free in `~/.config/kglobalshortcutsrc`; Alt+Space is free as well.

```bash
mkdir -p ~/.config/tomakast
cat > ~/.config/tomakast/config.toml <<'EOF'
[launcher]
hotkey = "Alt+Space"
autostart = true

[hotkeys]
"Ctrl+Alt+C" = "apps/org.kde.kcalc.desktop"
EOF
unset TOMAKAST_HOME TOMAKAST_NO_KEYBOARD
"$TK" daemon 2> /tmp/tk-real.log &
DAEMON=$!
```

| # | Check | How | Expected | Result |
|---|---|---|---|---|
| B1 | Portal registration | Accept the KDE dialog if it appears, then `grep -E 'global hotkeys' /tmp/tk-real.log` | `global hotkeys registered backend=Portal`. If you see `unavailable`, record the reason. | |
| B2 | Alt+Space opens | Press Alt+Space | The launcher opens with a focused, empty query. | |
| B3 | Alt+Space again closes | Press Alt+Space while it is open | The launcher closes. | |
| B4 | Esc closes | Alt+Space, then Esc | The launcher closes. | |
| B5 | App hotkey | Press Ctrl+Alt+C with the launcher closed | KCalc starts and the launcher does not open. | |
| B6 | Autostart file | `cat ~/.config/autostart/tomakast.desktop` | `Exec="<path to target/release/tomakast>" daemon`. | |
| B7 | Starts after re-login | Log out and back in, then `pgrep -a tomakast` and press Alt+Space | A `tomakast daemon` process runs and Alt+Space opens the launcher. | |

Clean up after Part B, if wanted: `kill "$DAEMON"`. To remove the autostart entry, set `autostart = false` in the config and start the daemon once, or `rm ~/.config/autostart/tomakast.desktop`.

Restore the backup with the daemon stopped. A file that had no backup did not exist before Part B, so it is removed. After the B7 re-login the shell variables are gone: set `BK` to the backup dir printed above (`ls -d ~/tk-backup.*`) and stop the autostarted daemon with `pkill -x tomakast`.

```bash
: "${BK:?set BK to the backup dir first}"
kill "$DAEMON" 2>/dev/null || pkill -x tomakast
if [ -f "$BK/config.toml" ]; then cp -p "$BK/config.toml" ~/.config/tomakast/config.toml; else rm -f ~/.config/tomakast/config.toml; fi
if [ -f "$BK/tomakast.desktop" ]; then cp -p "$BK/tomakast.desktop" ~/.config/autostart/tomakast.desktop; else rm -f ~/.config/autostart/tomakast.desktop; fi
rm -rf "$BK"
```

Skip the restore if you want to keep tomakast autostarting with the Part B config.
