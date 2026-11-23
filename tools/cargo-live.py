#!/usr/bin/env python3
"""Keeps cargo's target dirs from filling the disk without costing the `just` builds a rebuild.

Cargo never deletes a build it no longer uses: a changed setting, feature set or dependency
leaves the old copy next to the new one, hundreds of MB per test binary. On this machine the
disk is mounted `noatime` and cargo leaves the files of a fresh (not rebuilt) unit untouched, so
nothing on disk tells a used build from a stale one. Cargo's JSON messages do: they name every
unit a command uses, fresh ones included.

    cargo-live.py cargo <cargo args>   run cargo; on success, record the units it used
    cargo-live.py sweep <target dir>…  delete the units no recorded command uses

`cargo` records into `.live/` of each profile dir (`target/debug`, …), one list per command (its arguments, the working
directory and CARGO_TARGET_DIR); a later run of the same command replaces its list. For
`cargo test` it builds first with `--no-run`, so that the tests' own output stays out of the JSON.

`sweep` runs at most once a day per profile dir, and skips a profile dir while cargo holds its
lock. It deletes a hashed unit (in `deps`, `build`, `.fingerprint`, `examples`) only when no list
of the last two weeks names it and it was built more than three days ago; an `incremental` folder
only when a newer one of the same crate exists and it is as old. Without a list it deletes
nothing. A unit built only by a plain `cargo` command (not through this script) and not rebuilt
for three days is deleted too: that command then rebuilds it once.
"""

import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

LIVE = ".live"
KEEP_LISTS = 14 * 24 * 3600
SWEEP_EVERY = 20 * 3600
# Whatever was built in the last days stays, listed or not: work outside the recipes.
MIN_AGE = 3 * 24 * 3600
# CARGO_LIVE_FORCE=1 sweeps now and spares nothing for its age (for testing this script).
FORCE = os.environ.get("CARGO_LIVE_FORCE") == "1"
HASHED = re.compile(r"^(.+)-([0-9a-f]{16})(\..+)?$")
INCREMENTAL = re.compile(r"^(.+)-([0-9a-z]{13})$")
UNIT_DIRS = ("deps", "build", ".fingerprint", "examples")


def split(path):
    """`(profile dir, unit dir, name)` of a cargo output path: `target/debug/deps/libx-<hash>.rlib`,
    `target/debug/build/x-<hash>/out`; an uplifted copy (`target/debug/tomakast`) has no unit dir."""
    parts = path.split(os.sep)
    for back in (2, 3):
        if len(parts) > back and parts[-back] in UNIT_DIRS:
            return os.sep.join(parts[:-back]), parts[-back], parts[-back + 1]
    return os.path.dirname(path), None, os.path.basename(path)


def unit_hash(path):
    """`(profile dir, unit hash)` of a cargo output path. An uplifted copy is a hard link of a
    hashed file: in `deps` (binaries, libraries) or next to it (examples)."""
    profile, sub, name = split(path)
    m = HASHED.match(name)
    if m:
        return profile, m.group(2)
    folder = os.path.join(profile, sub or "deps")
    # `target/debug/tomakast-raster` is a link of `deps/tomakast_raster-<hash>`.
    stem = name.split(".")[0].replace("-", "_") + "_"
    try:
        st = os.stat(path)
        names = os.listdir(folder)
    except OSError:
        return None
    for other in names:
        m = HASHED.match(other)
        if not m or not other.replace("-", "_").startswith(stem):
            continue
        try:
            o = os.stat(os.path.join(folder, other))
        except OSError:
            continue
        if (o.st_ino, o.st_dev) == (st.st_ino, st.st_dev):
            return profile, m.group(2)
    return None


def crate_names(name):
    """Spellings of the crate a file or unit folder is named after: `libkitchen_sink.rlib`,
    `kitchen-sink-<hash>` and `kitchen_sink.wasm` all give `kitchen_sink`."""
    base = name.split(".")[0].replace("-", "_")
    names = {base}
    if base.startswith("lib"):
        names.add(base[3:])
    return names


def with_json(args):
    """`args` with JSON messages asked for, before a `--` that starts the tool's own args."""
    extra = ["--message-format=json-render-diagnostics"]
    if "--" in args:
        i = args.index("--")
        return args[:i] + extra + args[i:]
    return args + extra


def run_recorded(args):
    """Runs cargo with JSON messages; returns its exit code and the units it named."""
    units = set()
    proc = subprocess.Popen(["cargo"] + with_json(args), stdout=subprocess.PIPE, text=True)
    for line in proc.stdout:
        if not line.startswith('{"reason":'):
            sys.stdout.write(line)
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            sys.stdout.write(line)
            continue
        paths = []
        if msg["reason"] == "compiler-artifact":
            paths = list(msg.get("filenames") or [])
            if msg.get("executable"):
                paths.append(msg["executable"])
        elif msg["reason"] == "build-script-executed":
            paths = [msg["out_dir"]]
        for path in paths:
            unit = unit_hash(path)
            if unit:
                units.add(unit)
            else:
                # The package's own outputs carry no hash (`libcalc.rlib`, `calc.wasm`): its
                # unit folders are kept by name.
                profile, _, name = split(path)
                units.update((profile, "=" + n) for n in crate_names(name))
    return proc.wait(), units


def record(args, units):
    """Writes the units a command used into the `.live` folder of each profile dir it built in."""
    by_profile = {}
    for profile, h in units:
        by_profile.setdefault(profile, set()).add(h)
    key = hashlib.sha1(
        json.dumps([os.getcwd(), os.environ.get("CARGO_TARGET_DIR", ""), args]).encode()
    ).hexdigest()[:16]
    for profile, hashes in by_profile.items():
        folder = os.path.join(profile, LIVE)
        os.makedirs(folder, exist_ok=True)
        tmp = os.path.join(folder, f".{key}.tmp")
        with open(tmp, "w") as f:
            json.dump({"args": args, "units": sorted(hashes)}, f)
        os.replace(tmp, os.path.join(folder, f"{key}.json"))


def cargo(args):
    if args and args[0] == "test" and "--no-run" not in args:
        i = args.index("--") if "--" in args else len(args)
        build = args[:i] + ["--no-run"]
        code, units = run_recorded(build)
        if code != 0:
            return code
        code = subprocess.call(["cargo"] + args)
        if code == 0:
            record(build, units)
        return code
    code, units = run_recorded(args)
    if code == 0:
        record(args, units)
    return code


def remove(path):
    if os.path.isdir(path) and not os.path.islink(path):
        shutil.rmtree(path, ignore_errors=True)
    else:
        try:
            os.remove(path)
        except OSError:
            pass


def size(path):
    if os.path.isdir(path) and not os.path.islink(path):
        total = 0
        for root, _, files in os.walk(path):
            for name in files:
                try:
                    total += os.lstat(os.path.join(root, name)).st_size
                except OSError:
                    pass
        return total
    try:
        return os.lstat(path).st_size
    except OSError:
        return 0


def sweep_profile(profile, live, cutoff):
    freed = 0
    for sub in UNIT_DIRS:
        folder = os.path.join(profile, sub)
        try:
            names = os.listdir(folder)
        except OSError:
            continue
        for name in names:
            m = HASHED.match(name)
            if not m or m.group(2) in live:
                continue
            if any("=" + n in live for n in crate_names(m.group(1))):
                continue
            path = os.path.join(folder, name)
            try:
                if os.lstat(path).st_mtime >= cutoff:
                    continue
            except OSError:
                continue
            freed += size(path)
            remove(path)
    folder = os.path.join(profile, "incremental")
    groups = {}
    try:
        names = os.listdir(folder)
    except OSError:
        names = []
    for name in names:
        m = INCREMENTAL.match(name)
        if not m:
            continue
        path = os.path.join(folder, name)
        try:
            groups.setdefault(m.group(1), []).append((os.lstat(path).st_mtime, path))
        except OSError:
            pass
    for entries in groups.values():
        entries.sort()
        for mtime, path in entries[:-1]:
            if mtime < cutoff:
                freed += size(path)
                remove(path)
    return freed


def profile_dirs(target):
    """The profile dirs under a target dir that hold lists: `debug`, `wasm32-wasip2/release`, and
    those of a target dir inside it (`plugins/debug`)."""
    for root, dirs, _ in os.walk(target):
        depth = os.path.relpath(root, target).count(os.sep) + (root != target)
        if LIVE in dirs:
            yield root
        dirs[:] = [d for d in dirs if depth < 3 and d not in UNIT_DIRS + ("incremental", LIVE)]


def sweep_one(profile, now):
    folder = os.path.join(profile, LIVE)
    stamp = os.path.join(folder, ".last-sweep")
    try:
        if not FORCE and now - os.stat(stamp).st_mtime < SWEEP_EVERY:
            return 0
    except OSError:
        pass
    lists = []
    for name in os.listdir(folder):
        if not name.endswith(".json"):
            continue
        path = os.path.join(folder, name)
        try:
            mtime = os.stat(path).st_mtime
            if now - mtime > KEEP_LISTS:
                os.remove(path)
                continue
            with open(path) as f:
                lists.append(set(json.load(f)["units"]))
        except (OSError, ValueError, KeyError, TypeError):
            continue
    if not lists:
        return 0
    cutoff = now if FORCE else now - MIN_AGE
    live = set().union(*lists)
    try:
        lock = open(os.path.join(profile, ".cargo-lock"), "a")
    except OSError:
        return 0
    with lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            return 0  # cargo is building here; next time
        freed = sweep_profile(profile, live, cutoff)
    with open(stamp, "w"):
        pass
    return freed


def sweep(targets):
    now = time.time()
    for target in targets:
        freed = sum(sweep_one(profile, now) for profile in profile_dirs(target))
        if freed:
            print(f"cargo-live: freed {freed / 2**30:.1f} GiB of old builds in {target}")


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("cargo", "sweep"):
        sys.exit(__doc__)
    if sys.argv[1] == "cargo":
        sys.exit(cargo(sys.argv[2:]))
    sweep(sys.argv[2:])


if __name__ == "__main__":
    main()
