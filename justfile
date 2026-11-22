# Dev tasks. `just` lists them. Needs bash (on Windows, Git Bash).

set shell := ["bash", "-euo", "pipefail", "-c"]

root := justfile_directory()
tk := root / "target/release/tomakast"
wasm := root / "target/plugins/wasm32-wasip2/release"
# The throwaway home of `just try`. Never your real one: `try` deletes it.
home := env_var_or_default("XDG_RUNTIME_DIR", "/tmp") / "tk-try"

# List the tasks.
default:
    @just --list --unsorted

# Build the app, its raster helper and the plugins `just try` installs: calc, weather and translator (release). kitchen-sink, the host tests' plugin, is built by those tests.
build:
    cargo build --release -p tomakast-app
    # The raster helper the launcher runs for plugin images; found next to `tomakast`.
    cargo build --release -p tomakast-ui --bin tomakast-raster
    for p in calc weather translator; do \
        CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 \
            --manifest-path plugins/$p/Cargo.toml; \
    done

# Run the daemon in a throwaway home with calc, weather and translator installed, the weather on Home (a quarter `now` and a single `today`), the demo module on and the keyboard grabbed. `just try approve` approves the plugins up front, skipping the review. `just try approve nokeys` (or `just try "" nokeys`) keeps `TOMAKAST_NO_KEYBOARD=1`: no global hotkey, nothing grabbed, for runs that must not take the keyboard. It logs at `RUST_LOG`, else `info,tomakast_ui=debug`, in colour unless `NO_COLOR` is set. Ctrl+C (or killing the daemon) stops it and deletes the home.
try approve="" keys="": build
    #!/usr/bin/env bash
    set -euo pipefail
    home='{{ home }}'
    case "$home" in */tk-try) ;; *) echo "refusing to use $home" >&2; exit 1 ;; esac
    rm -rf "$home"
    mkdir -p "$home/plugins/calc" "$home/plugins/weather" "$home/plugins/translator"
    cp plugins/calc/plugin.toml "$home/plugins/calc/"
    cp '{{ wasm }}/calc.wasm' "$home/plugins/calc/plugin.wasm"
    cp plugins/weather/plugin.toml "$home/plugins/weather/"
    cp '{{ wasm }}/weather.wasm' "$home/plugins/weather/plugin.wasm"
    cp plugins/translator/plugin.toml "$home/plugins/translator/"
    cp -r plugins/translator/flags "$home/plugins/translator/"
    cp '{{ wasm }}/translator.wasm' "$home/plugins/translator/plugin.wasm"
    printf '%s\n' '[[home.widgets]]' 'widget = "weather/now"' 'size = "quarter"' '' \
        '[[home.widgets]]' 'widget = "weather/today"' 'size = "single"' > "$home/config.toml"
    if [ '{{ approve }}' = approve ]; then
        cargo build -q --release -p tomakast-plugin-host --example approve_plugin
        for p in calc weather translator; do
            target/release/examples/approve_plugin "$home/tomakast.db" "$home/plugins/$p"
        done
    fi
    # KDE keeps hotkeys grabbed after the daemon exits: release them, then drop the home.
    # Without the keyboard nothing was bound, and releasing would touch the desktop's own.
    if [ '{{ keys }}' = nokeys ]; then
        trap 'rm -rf "$home"' EXIT
    else
        trap '"{{ tk }}" release-hotkeys; rm -rf "$home"' EXIT
    fi
    echo "home: $home"
    echo "toggle from another terminal: just toggle    log: just log"
    if [ '{{ keys }}' = nokeys ]; then
        export TOMAKAST_NO_KEYBOARD=1
    else
        unset TOMAKAST_NO_KEYBOARD
    fi
    # NO_COLOR, when set, is passed through as it is.
    TOMAKAST_HOME="$home" TOMAKAST_DEMO=1 RUST_LOG="${RUST_LOG:-info,tomakast_ui=debug}" \
        '{{ tk }}' daemon 2> "$home/daemon.log"

# Rebuild the plugins and put them into the running `just try`: the launcher reloads a plugin whose files changed, without a restart. A change of the launcher itself still needs `just try` again.
reload:
    #!/usr/bin/env bash
    set -euo pipefail
    home='{{ home }}'
    [ -d "$home/plugins" ] || { echo "no just try is running ($home)" >&2; exit 1; }
    for p in calc weather translator; do
        CARGO_TARGET_DIR=target/plugins cargo build -q --release --target wasm32-wasip2 \
            --manifest-path plugins/$p/Cargo.toml
    done
    for p in calc weather translator; do
        dir="$home/plugins/$p"
        mkdir -p "$dir"
        cp plugins/$p/plugin.toml "$dir/"
        if [ -d plugins/$p/flags ]; then cp -r plugins/$p/flags "$dir/"; fi
        # The wasm last: its change is the one the launcher reloads on.
        cp '{{ wasm }}'/$p.wasm "$dir/plugin.wasm"
    done
    echo "plugins copied into $home: the launcher reloads the changed ones"

# Show or hide the launcher of `just try`.
toggle:
    TOMAKAST_HOME='{{ home }}' '{{ tk }}' toggle

# Run an item or command of `just try` by id, e.g. `just run weather/refresh`.
run id:
    TOMAKAST_HOME='{{ home }}' '{{ tk }}' run '{{ id }}'

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

# Everything CI runs: fmt, clippy and tests for the workspace, the guest SDK and the plugins.
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p tomakast-plugin-host --lib -- -D warnings
    cargo clippy -p tomakast-location --all-targets --target aarch64-apple-darwin -- -D warnings
    cargo clippy -p tomakast-location --all-targets --target x86_64-pc-windows-gnu -- -D warnings
    cargo test --workspace
    just _guest crates/plugin-sdk
    CARGO_TARGET_DIR=target/plugins cargo test --manifest-path crates/plugin-sdk/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo fmt --check --manifest-path plugins/kitchen-sink/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo clippy --manifest-path plugins/kitchen-sink/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings
    just _guest plugins/calc
    CARGO_TARGET_DIR=target/plugins cargo test --release --manifest-path plugins/calc/Cargo.toml
    just _guest plugins/weather
    CARGO_TARGET_DIR=target/plugins cargo test --manifest-path plugins/weather/Cargo.toml
    just _guest plugins/translator
    CARGO_TARGET_DIR=target/plugins cargo test --manifest-path plugins/translator/Cargo.toml

# fmt and clippy (native and wasm) of a guest crate.
_guest dir:
    CARGO_TARGET_DIR=target/plugins cargo fmt --check --manifest-path {{ dir }}/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo clippy --manifest-path {{ dir }}/Cargo.toml --all-targets -- -D warnings
    CARGO_TARGET_DIR=target/plugins cargo clippy --manifest-path {{ dir }}/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings

# Format everything: the workspace and the guest crates.
fmt:
    cargo fmt --all
    for d in crates/plugin-sdk plugins/kitchen-sink plugins/calc plugins/weather plugins/translator; do cargo fmt --manifest-path $d/Cargo.toml; done
