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

# Build the app, its raster helper and both example plugins (release).
build:
    cargo build --release -p tomakast-app
    # The raster helper the launcher runs for plugin images; found next to `tomakast`.
    cargo build --release -p tomakast-ui --bin tomakast-raster
    for p in calc kitchen-sink; do \
        CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 \
            --manifest-path plugins/$p/Cargo.toml; \
    done

# Run the daemon in a throwaway home with calc and kitchen-sink installed, the demo module on and the keyboard grabbed. `just try approve` approves both plugins up front, skipping the review. It logs at `RUST_LOG`, else `info,tomakast_ui=debug`, in colour unless `NO_COLOR` is set. Ctrl+C (or killing the daemon) stops it and deletes the home.
try approve="": build
    #!/usr/bin/env bash
    set -euo pipefail
    home='{{ home }}'
    case "$home" in */tk-try) ;; *) echo "refusing to use $home" >&2; exit 1 ;; esac
    rm -rf "$home"
    mkdir -p "$home/plugins/calc" "$home/plugins/kitchen-sink"
    cp plugins/calc/plugin.toml "$home/plugins/calc/"
    cp '{{ wasm }}/calc.wasm' "$home/plugins/calc/plugin.wasm"
    cp -r plugins/kitchen-sink/plugin.toml plugins/kitchen-sink/icons "$home/plugins/kitchen-sink/"
    cp '{{ wasm }}/kitchen_sink.wasm' "$home/plugins/kitchen-sink/plugin.wasm"
    if [ '{{ approve }}' = approve ]; then
        cargo build -q --release -p tomakast-plugin-host --example approve_plugin
        for p in calc kitchen-sink; do
            target/release/examples/approve_plugin "$home/tomakast.db" "$home/plugins/$p"
        done
    fi
    trap 'rm -rf "$home"' EXIT
    echo "home: $home"
    echo "toggle from another terminal: just toggle    log: just log"
    unset TOMAKAST_NO_KEYBOARD
    # NO_COLOR, when set, is passed through as it is.
    TOMAKAST_HOME="$home" TOMAKAST_DEMO=1 RUST_LOG="${RUST_LOG:-info,tomakast_ui=debug}" \
        '{{ tk }}' daemon 2> "$home/daemon.log"

# Show or hide the launcher of `just try`.
toggle:
    TOMAKAST_HOME='{{ home }}' '{{ tk }}' toggle

# Run an item or command of `just try` by id, e.g. `just run kitchen-sink/hello`.
run id:
    TOMAKAST_HOME='{{ home }}' '{{ tk }}' run '{{ id }}'

# Follow the log of `just try`.
log:
    tail -f '{{ home }}/daemon.log'

# Everything CI runs: fmt, clippy and tests for the workspace, the guest SDK and both plugins.
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p tomakast-plugin-host --lib -- -D warnings
    cargo test --workspace
    just _guest crates/plugin-sdk
    CARGO_TARGET_DIR=target/plugins cargo test --manifest-path crates/plugin-sdk/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo fmt --check --manifest-path plugins/kitchen-sink/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo clippy --manifest-path plugins/kitchen-sink/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings
    just _guest plugins/calc
    CARGO_TARGET_DIR=target/plugins cargo test --release --manifest-path plugins/calc/Cargo.toml

# fmt and clippy (native and wasm) of a guest crate.
_guest dir:
    CARGO_TARGET_DIR=target/plugins cargo fmt --check --manifest-path {{ dir }}/Cargo.toml
    CARGO_TARGET_DIR=target/plugins cargo clippy --manifest-path {{ dir }}/Cargo.toml --all-targets -- -D warnings
    CARGO_TARGET_DIR=target/plugins cargo clippy --manifest-path {{ dir }}/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings

# Format everything: the workspace and the guest crates.
fmt:
    cargo fmt --all
    for d in crates/plugin-sdk plugins/kitchen-sink plugins/calc; do cargo fmt --manifest-path $d/Cargo.toml; done
