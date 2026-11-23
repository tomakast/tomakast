# Dev tasks. `just` lists them. Needs bash (on Windows, Git Bash).

set shell := ["bash", "-euo", "pipefail", "-c"]

root := justfile_directory()
# The cargo profile of every build and test here: `dev`, the everyday build (the default), or
# `release`, the optimized build, slow to build: only on request, given before the task:
# `just profile=release build` (or `try`, `check`). `reload`, `toggle` and `run` follow the
# profile the running `just try` was given.
profile := "dev"
profile_flag := if profile == "dev" { "" } else if profile == "release" { "--release" } else { error("profile is dev or release, not " + profile) }
profile_dir := if profile == "dev" { "debug" } else { "release" }
tk := root / "target" / profile_dir / "tomakast"
wasm := root / "target/plugins/wasm32-wasip2" / profile_dir
# Cargo through tools/cargo-live.py, which records the builds each task uses; after `build`,
# `reload` and `check`, `_sweep` deletes the old builds no task uses any more.
cargo := if os() == "windows" { "cargo" } else { "tools/cargo-live.py cargo" }
# The home of `just try`, kept between runs (and reboots); `just try fresh` starts it over.
# Never your real one: `try fresh` deletes it.
home := env_var_or_default("XDG_STATE_HOME", home_directory() / ".local/state") / "tk-try"
# The `tomakast` of the running `just try`, of the profile it was given (shell code).
try_tk := "$(if [ \"$(cat '" + home + "/.just-profile' 2>/dev/null)\" = release ]; then echo '" + root / "target/release/tomakast'; else echo '" + root / "target/debug/tomakast'; fi)"

# List the tasks.
default:
    @just --list --unsorted

# Build the app, its raster helper and the plugins `just try` installs: calc, weather and translator. The dev build; `just profile=release build` for the optimized one. kitchen-sink, the host tests' plugin, is built by those tests.
build: && _sweep
    {{ cargo }} build {{ profile_flag }} -p tomakast-app
    # The raster helper the launcher runs for plugin images; found next to `tomakast`.
    {{ cargo }} build {{ profile_flag }} -p tomakast-ui --bin tomakast-raster
    for p in calc weather translator; do \
        CARGO_TARGET_DIR=target/plugins {{ cargo }} build {{ profile_flag }} --target wasm32-wasip2 \
            --manifest-path plugins/$p/Cargo.toml; \
    done

# Run the daemon in its own home (not your real one) with fresh builds of calc, weather and translator, the demo module on and the keyboard grabbed. The home is kept between runs: settings, approvals, widgets and caches stay. A new home gets the weather on Home (a quarter `now` and a single `today`). Words, in any order: `fresh` deletes the home first; `approve` approves the plugins up front, skipping the review; `nokeys` keeps `TOMAKAST_NO_KEYBOARD=1`: no global hotkey, nothing grabbed, for runs that must not take the keyboard. E.g. `just try fresh approve nokeys`. It logs at `RUST_LOG`, else `info,tomakast_ui=debug`, in colour unless `NO_COLOR` is set. Ctrl+C (or killing the daemon) stops it. The dev build; `just profile=release try` runs the optimized one.
try *words: build
    #!/usr/bin/env bash
    set -euo pipefail
    home='{{ home }}'
    case "$home" in */tk-try) ;; *) echo "refusing to use $home" >&2; exit 1 ;; esac
    fresh='' approve='' keys=yes
    for w in {{ words }}; do
        case "$w" in
            fresh) fresh=yes ;;
            approve) approve=yes ;;
            nokeys) keys='' ;;
            *) echo "just try: unknown word '$w' (fresh, approve, nokeys)" >&2; exit 1 ;;
        esac
    done
    if [ -n "$fresh" ]; then rm -rf "$home"; fi
    mkdir -p "$home/plugins/calc" "$home/plugins/weather" "$home/plugins/translator"
    # `reload`, `toggle` and `run` build and run with the profile of this run.
    echo '{{ profile }}' > "$home/.just-profile"
    cp plugins/calc/plugin.toml "$home/plugins/calc/"
    cp '{{ wasm }}/calc.wasm' "$home/plugins/calc/plugin.wasm"
    cp plugins/weather/plugin.toml "$home/plugins/weather/"
    cp '{{ wasm }}/weather.wasm' "$home/plugins/weather/plugin.wasm"
    cp plugins/translator/plugin.toml "$home/plugins/translator/"
    cp -r plugins/translator/flags "$home/plugins/translator/"
    cp '{{ wasm }}/translator.wasm' "$home/plugins/translator/plugin.wasm"
    if [ ! -e "$home/config.toml" ]; then
        printf '%s\n' '[[home.widgets]]' 'widget = "weather/now"' 'size = "quarter"' '' \
            '[[home.widgets]]' 'widget = "weather/today"' 'size = "single"' > "$home/config.toml"
    fi
    # Approvals given before stay; a plugin whose build changed is approved again.
    if [ -n "$approve" ]; then
        {{ cargo }} build -q {{ profile_flag }} -p tomakast-plugin-host --example approve_plugin
        for p in calc weather translator; do
            'target/{{ profile_dir }}/examples/approve_plugin' "$home/tomakast.db" "$home/plugins/$p"
        done
    fi
    # KDE keeps hotkeys grabbed after the daemon exits: release them.
    # Without the keyboard nothing was bound, and releasing would touch the desktop's own.
    if [ -n "$keys" ]; then
        trap '"{{ tk }}" release-hotkeys' EXIT
        unset TOMAKAST_NO_KEYBOARD
    else
        export TOMAKAST_NO_KEYBOARD=1
    fi
    echo "home: $home (kept; just try fresh starts over)"
    echo "toggle from another terminal: just toggle    log: just log"
    # NO_COLOR, when set, is passed through as it is.
    TOMAKAST_HOME="$home" TOMAKAST_DEMO=1 RUST_LOG="${RUST_LOG:-info,tomakast_ui=debug}" \
        '{{ tk }}' daemon 2> "$home/daemon.log"

# Rebuild the plugins and put them into the running `just try`: the launcher reloads a plugin whose files changed, without a restart. A change of the launcher itself still needs `just try` again. It builds with the profile `just try` was given.
reload: && _sweep
    #!/usr/bin/env bash
    set -euo pipefail
    home='{{ home }}'
    [ -d "$home/plugins" ] || { echo "no home of just try ($home): run just try first" >&2; exit 1; }
    flag='' out=debug
    if [ "$(cat "$home/.just-profile" 2>/dev/null)" = release ]; then flag=--release out=release; fi
    for p in calc weather translator; do
        CARGO_TARGET_DIR=target/plugins {{ cargo }} build -q $flag --target wasm32-wasip2 \
            --manifest-path plugins/$p/Cargo.toml
    done
    for p in calc weather translator; do
        dir="$home/plugins/$p"
        mkdir -p "$dir"
        cp plugins/$p/plugin.toml "$dir/"
        if [ -d plugins/$p/flags ]; then cp -r plugins/$p/flags "$dir/"; fi
        # The wasm last: its change is the one the launcher reloads on.
        cp "target/plugins/wasm32-wasip2/$out/$p.wasm" "$dir/plugin.wasm"
    done
    echo "plugins copied into $home: a running launcher reloads the changed ones"

# Show or hide the launcher of `just try`.
toggle:
    @TOMAKAST_HOME='{{ home }}' "{{ try_tk }}" toggle

# Run an item or command of `just try` by id, e.g. `just run weather/refresh`.
run id:
    @TOMAKAST_HOME='{{ home }}' "{{ try_tk }}" run '{{ id }}'

# Follow the log of `just try`.
log:
    tail -f '{{ home }}/daemon.log'

# Probe the desktop's GlobalShortcuts portal (Linux): binds Ctrl+Alt+Shift+K/L/U/I under a test app id and asks you to answer KDE's dialogs. Report: $XDG_RUNTIME_DIR/tk-hotkey-probe.txt.
hotkey-probe:
    cargo run -q -p tomakast-platform --example portal_probe

# Ask each of the launcher's location sources once (the OS service, the time zone) and print what plugins would get. The desktop may show its permission dialog.
locate:
    cargo run -q -p tomakast-plugin-host --example locate

# Like `just locate`, with the Wi-Fi lookup too: sends the Wi-Fi networks around (and the IP address) to BeaconDB.
locate-wifi:
    cargo run -q -p tomakast-plugin-host --example locate -- --wifi

# Everything CI runs: fmt, clippy and tests for the workspace, the guest SDK and the plugins. The dev build, so the timing tests are skipped; CI runs calc's tests optimized for its timing tests, and `just profile=release check` runs every test optimized, the timing tests too.
check: && _sweep
    cargo fmt --check
    {{ cargo }} clippy --workspace --all-targets -- -D warnings
    {{ cargo }} clippy -p tomakast-plugin-host --lib -- -D warnings
    {{ cargo }} clippy -p tomakast-location --all-targets --target aarch64-apple-darwin -- -D warnings
    {{ cargo }} clippy -p tomakast-location --all-targets --target x86_64-pc-windows-gnu -- -D warnings
    {{ cargo }} test {{ profile_flag }} --workspace
    # The guests the host tests built: recorded for `_sweep` (built already, nothing rebuilds).
    for p in calc weather translator kitchen-sink; do \
        CARGO_TARGET_DIR=target/plugins {{ cargo }} build -q {{ profile_flag }} --target wasm32-wasip2 \
            --manifest-path plugins/$p/Cargo.toml; \
    done
    just _guest crates/plugin-sdk
    CARGO_TARGET_DIR=target/plugins {{ cargo }} test --manifest-path crates/plugin-sdk/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo fmt --check --manifest-path plugins/kitchen-sink/Cargo.toml
    CARGO_TARGET_DIR=target/plugins {{ cargo }} clippy --manifest-path plugins/kitchen-sink/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings
    just _guest plugins/calc
    CARGO_TARGET_DIR=target/plugins {{ cargo }} test {{ profile_flag }} --manifest-path plugins/calc/Cargo.toml
    just _guest plugins/weather
    CARGO_TARGET_DIR=target/plugins {{ cargo }} test --manifest-path plugins/weather/Cargo.toml
    just _guest plugins/translator
    CARGO_TARGET_DIR=target/plugins {{ cargo }} test --manifest-path plugins/translator/Cargo.toml

# fmt and clippy (native and wasm) of a guest crate.
_guest dir:
    CARGO_TARGET_DIR=target/plugins cargo fmt --check --manifest-path {{ dir }}/Cargo.toml
    CARGO_TARGET_DIR=target/plugins {{ cargo }} clippy --manifest-path {{ dir }}/Cargo.toml --all-targets -- -D warnings
    CARGO_TARGET_DIR=target/plugins {{ cargo }} clippy --manifest-path {{ dir }}/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings

# Format everything: the workspace and the guest crates.
fmt:
    cargo fmt --all
    for d in crates/plugin-sdk plugins/kitchen-sink plugins/calc plugins/weather plugins/translator; do cargo fmt --manifest-path $d/Cargo.toml; done

# Delete the old builds no task uses any more, at most once a day (see tools/cargo-live.py).
_sweep:
    @{{ if os() == "windows" { "true" } else { "tools/cargo-live.py sweep target" } }}
