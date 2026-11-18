# WASM tracer bullet (plan 2b-1, Task 1)

Date: 2026-11-18. Code: `experiments/wasm-tracer/` (`wit/tracer.wit`, `guest/`, `host/`, `size-probe/`).
None of these crates is a workspace member.

Status: done, except for the items in [Not measured](#not-measured).

## Answers for plan 2b-2 and Task 3

| Question | Answer | Confidence |
|---|---|---|
| Per-call cost of `search(query) -> list<item>`, reused instance | 1 item: **0.8 µs**. 10 items: **3.3 µs**. 100 items: **31 µs**. These are medians with epoch on; p95 is within 10 % of the median. Through a worker thread that owns the store: **5 / 8 / 37 µs**. | High |
| Cold `Component::new` vs `precompile_component` + `deserialize` | `Component::new`: **~6–7.5 ms**. `deserialize`: **~0.14 ms**, about 50× faster. `precompile_component` costs the same as `Component::new`. Precompiling also cuts load RSS: see the next row. | High (51 KB guest; scales with code size) |
| RSS | Engine + `deserialize` of one component: **+5 MiB** over a bare process. Engine + `Component::new`: **+21.5 MiB**, caused by Cranelift. First instance: **+1.0–1.2 MiB**. 10 instances: **+1.6 MiB** in total, about 45–75 KiB for each instance after the first. | Medium (a single small guest) |
| Binary size of a release host that links wasmtime | **+17.3 MiB stripped** with wasmtime's default features. **+20.6 MiB stripped** with default wasmtime plus `wasmtime-wasi`. **+13.2 MiB** with a trimmed compiler build. **+1.2 MiB** with a runtime-only build (no Cranelift, no WASI). | High; medium for runtime-only (not exercised past `deserialize`) |
| Does epoch interruption stop an infinite loop within 100 ms? | **Yes.** With a 1 ms tick and a 50-tick deadline it traps at 52.6 ms. With a 10 ms tick and a 10-tick deadline it traps at 100.6 ms. The trap is `wasm trap: interrupt`. The time past the target comes from ticker-thread sleep overshoot, not from trap latency. | High |
| Cost of epoch interruption | Calls are **~20 % slower** in guest-heavy code: 25.4 → 30.6 µs for 100 items, 2.8 → 3.3 µs for 10. No measurable difference for 1 item. Compilation is ~1.3 ms slower. Instantiation costs the same. | High |
| Does `StoreLimits` cap guest memory? | **Yes.** At a 64 MiB limit, `hog` (1 GiB) traps. Without a limiter, the same `hog` returns normally in 58 ms. | High |
| Does WIT accept a recursive type? | **No.** `record node { label: string, children: list<node> }` is rejected by `wit-parser` 0.261 (wasmtime 50's version) with: ``type `node` depends on itself``. | High |
| Sync or async? | **Sync is enough:** sync wasmtime, one worker thread per plugin. See the [ruling](#ruling-sync-or-async). | High for 2b-2's scope |

## Setup

- AMD Ryzen 7 7800X3D (8 cores / 16 threads), 30 GiB RAM, Linux 7.3.5-1-cachyos (CachyOS), Rust 1.98.1.
- Host: `wasmtime 50.0.2` (default features, Cranelift 0.137.2), `wasmtime-wasi 50.0.2` (WASI p2, `add_to_linker_sync`), `wit-parser 0.261.0`.
  - Every store has an empty `WasiCtx`: no args, env, preopens or network.
  - The engine uses the default `Config`; `epoch_interruption(true)` is set where noted.
  - No pooling allocator; no compilation cache (the default).
- Guest: `wit-bindgen 0.63.0`, target `wasm32-wasip2`, built with `opt-level = "s"`, LTO, stripped.
  - The component is 51 290 bytes.
  - Its std imports WASI p2 (`wasi:io/poll@0.2.9` and others). This is why the host links `wasmtime-wasi`: without it, instantiation fails.
- WIT (`experiments/wasm-tracer/wit/tracer.wit`): the brief's world, with the record named `item`:
  - `record item { id, title, subtitle: string, score: u32 }`;
  - `search: func(query: string) -> list<item>`, which returns as many items as the leading number of the query;
  - `spin: func()`, an infinite loop;
  - `hog: func()`, which allocates 1 GiB and writes to every byte of it.
  - Each item is about 90 bytes of strings.
- Runs:
  - build the guest: `cargo build --release --target wasm32-wasip2 --manifest-path experiments/wasm-tracer/guest/Cargo.toml`;
  - print all measurements: `cargo run --release --manifest-path experiments/wasm-tracer/host/Cargo.toml`;
  - behaviour tests: `cargo test --release --manifest-path experiments/wasm-tracer/host/Cargo.toml`;
  - binary sizes, which the host does not print:
    - build `experiments/wasm-tracer/size-probe` once per feature set: `cargo build --release` for the baseline, then `cargo build --release --features <set>` for `runtime`, `compiler`, `full` and `wasi`;
    - after each build, copy `target/release/size-probe` and run `strip` on the copy.
- Statistics: medians and p95 (nearest rank) over 1000 calls after 50 warm-up calls. Compile times are over 5 runs, each on a fresh `Engine`.
- Run-to-run variation was within ±10 %. The tables below come from the last run.

## Numbers

### Compilation and loading (fresh `Engine` per run)

| | epoch off median | epoch off p95 | epoch on median | epoch on p95 |
|---|---|---|---|---|
| `Component::new` | 6.13 ms | 8.61 ms | 7.50 ms | 8.10 ms |
| `Engine::precompile_component` | 6.62 ms | 6.85 ms | 7.38 ms | 8.16 ms |
| `Component::deserialize` | 0.136 ms | 0.171 ms | 0.140 ms | 0.182 ms |
| precompiled artifact | 161 080 B | | 189 840 B | |

`Component::new` uses all cores (`parallel-compilation`). A precompiled artifact is tied to the engine config and the wasmtime version:
- an epoch-on `.cwasm` does not load into an epoch-off engine;
- a GC-enabled `.cwasm` does not load into a runtime-only build ("module was compiled with GC however GC is disabled in the host").

### `search` latency, one `Store` per plugin, instance reused

| items | epoch off median | epoch off p95 | epoch on median | epoch on p95 | worker round trip median | worker round trip p95 |
|---|---|---|---|---|---|---|
| 1 | 0.8 µs | 0.8 µs | 0.8 µs | 0.9 µs | 5.2 µs | 5.5 µs |
| 10 | 2.8 µs | 2.8 µs | 3.3 µs | 3.4 µs | 7.8 µs | 9.1 µs |
| 100 | 25.4 µs | 29.4 µs | 30.6 µs | 31.1 µs | 36.6 µs | 38.6 µs |

- Each call includes lowering the query and lifting the result `Vec<Item>` into host strings.
- Epoch-on runs use a 1 ms ticker and a 100-tick deadline that is reset before each call.
- Worker round trip:
  - the main thread sends the query over `std::sync::mpsc` to a thread that owns the `Store` and the instance, and waits for the reply;
  - epoch is on;
  - the ~4–5 µs over the in-thread call is the cost of waking the thread twice.
- Creating a `Store` and instantiating the component (empty WASI context included): median 17 µs, p95 24–26 µs.

### RSS (`VmRSS` from `/proc/self/status`; one clean child process per line, epoch on)

Each instance makes one `search("100 …")` call, so its memory is committed.

| load | instances | bare process | after engine + load | after instances | per instance |
|---|---|---|---|---|---|
| `deserialize` | 1 | 4 736 KiB | 9 876 KiB | 11 088 KiB | 1 212 KiB |
| `deserialize` | 10 | 4 736 KiB | 9 792 KiB | 11 408 KiB | 161 KiB average; about 45–75 KiB after the first |
| `Component::new` | 1 | 4 724 KiB | 26 768 KiB | 27 828 KiB | 1 060 KiB |

The +16.5 MiB of `Component::new` over `deserialize` is Cranelift's working memory, and it is not returned. Runs vary by about ±200 KiB. In an earlier run the instance deltas were 960 / 1 648 KiB.

### Binary size (`size-probe`, release, `debug = false`)

Each variant is the same small program:
- without wasmtime, it reads a file;
- with wasmtime, it also loads a component, instantiates it and calls `search` through the dynamic API.

| variant (cargo features) | unstripped | stripped | stripped delta vs baseline |
|---|---|---|---|
| baseline, no wasmtime | 458 KiB | 351 KiB | — |
| `runtime`: wasmtime with `runtime, component-model, std`, deserialize only (see the caveat below) | 2.19 MiB | 1.49 MiB | +1.15 MiB |
| `compiler`: `runtime` + `cranelift, parallel-compilation` | 18.8 MiB | 13.5 MiB | +13.2 MiB |
| `full`: wasmtime default features | 24.6 MiB | 17.7 MiB | +17.3 MiB |
| `wasi`: `full` + `wasmtime-wasi` (the tracer host's set) | 30.4 MiB | 20.9 MiB | +20.6 MiB |

Caveat for `runtime`, medium confidence:
- the smoke run got only as far as `deserialize`, which rejected the GC-enabled `.cwasm`, so no call was made;
- it links no WASI, which this guest needs. A usable runtime-only host is therefore about +1.2 MiB plus WASI (~3.3 MiB).

Most of the size comes from Cranelift. Three things are optional:
- the default features add 4 MiB over a trimmed compiler build: `gc*`, `async`, `component-model-async`, `cache`, `wat`, `profiling`, `coredump`, `debug` and others;
- WASI p2 adds 3.3 MiB, and it also pulls in `tokio` (only through `wasmtime-wasi`; wasmtime alone does not depend on it);
- a runtime-only daemon would need components compiled elsewhere, for example at install time by a separate tool.

### Epoch interruption of `spin` (fresh `Store` per run, 10 runs)

| tick | deadline | median | p95 | min | max |
|---|---|---|---|---|---|
| 1 ms | 50 ticks | 52.59 ms | 52.85 ms | 52.28 ms | 52.85 ms |
| 10 ms | 10 ticks | 100.57 ms | 100.59 ms | 100.48 ms | 100.59 ms |

The overshoot is ~5 % for a 1 ms tick and ~0.6 % for a 10 ms tick. Both are about 50–60 µs per tick, which matches the ticker's `thread::sleep` overrunning on every tick.

Inferred, not measured separately: the trap itself comes promptly once the epoch passes the deadline.

Each run starts right after a tick, because the ticker has just started or the previous trap has just fired. So the runs are phase-aligned, and the tight spread shows exactly N full ticks. A call that arrives mid-tick traps up to one tick earlier. The real window is therefore `(N − 1) × tick` to `N × tick`, plus drift.

The trap error is `wasm trap: interrupt`. It carries a wasm backtrace with no names, because the guest is stripped.

Pitfall, covered by the test `epoch_engine_without_a_deadline_traps_at_once`: when epoch interruption is on, a new store's deadline is 0. A host that forgets `set_epoch_deadline` before instantiation or before a call gets an immediate trap. Set the deadline before every call.

### `StoreLimits` vs `hog` (1 GiB)

| limiter | result |
|---|---|
| `memory_size(64 MiB)` | trap after 0.4–3.4 ms: `wasm 'unreachable' instruction executed`. `memory.grow` returned −1, and Rust's allocation-failure handler aborted. |
| `memory_size(64 MiB)` + `trap_on_grow_failure(true)` | trap after 0.01 ms: `forcing trap when growing memory to 1074855936 bytes`. The message is clear and can be reported. |
| none | returns normally after 58 ms. This is the control: the guest really can take 1 GiB. |

Use `trap_on_grow_failure(true)`. It turns a guest OOM into a host-visible "memory limit" error instead of a generic `unreachable`.

### Recursive WIT

The parser rejects the recursive WIT below, with ``type `node` depends on itself`` (`wit_parser::Resolve::push_str`):

```wit
interface tree {
  record node { label: string, children: list<node> }
}
```

Task 3 must keep every type in `view.rs` / `effect.rs` non-recursive. A tree has to be flattened into a list with parent indices or ids, which is what "views are flat" already requires.

## Per-keystroke cost vs the 16 ms budget

One keystroke sends one `search` to each dynamic plugin. Take a plugin that returns 100 small rows:
- the cost is ~31 µs with epoch on, or ~37 µs counting the round trip to its worker thread;
- that is **0.2 % of a 16 ms frame**;
- ten such plugins called one after another would take ~0.4 ms. With one worker thread each, they run in parallel, and the wall time stays near that of one plugin.

The WASM boundary itself (lowering, lifting, the epoch checks) is negligible next to the budget. These numbers do not include:
- the guest's real search work;
- the host's merging and fuzzy ranking;
- rendering.

The only WASM costs on the scale of a frame are one-time:
- `Component::new` takes ~7 ms for this tiny guest, and more for real plugins. Never compile on the UI thread or on a keystroke path. Compile or deserialize at daemon start, on the plugin's worker thread.
- Prefer a precompiled `.cwasm` cache. It loads in 0.14 ms and avoids ~17 MiB of RSS.

## Ruling: sync or async

**Sync is enough. Plan 2b-2 should use sync wasmtime, one worker thread per plugin, each owning its `Store` and a reused instance.**

- Latency: the sync call through a worker thread costs 5–37 µs. Async would not make that faster. It would add fiber stacks and `call_async` overhead. In wasmtime 50, `Config::async_support` is deprecated and has no effect, so async is chosen per call (`call_async`), not per engine.
- Deadlines:
  - The 100 ms dynamic-source deadline is a soft one: late results are still merged. It belongs on the host side (the core stops waiting), and it needs nothing from the guest.
  - A hard stop for a runaway guest works with sync: epoch interruption traps an infinite loop on time, and the worker thread gets back an error it can report.
  - Async epoch yielding (`epoch_deadline_async_yield_and_update`) is not needed.
- Isolation: one thread per plugin means a stuck or slow plugin blocks only its own queue. The UI thread never calls into WASM. This matches "the UI thread never waits on a module".
- Threads are cheap at the expected plugin count (tens). Measuring them was out of scope.
- Async would become necessary only if plugins import host functions that must not block their own thread, such as HTTP. Even then, a sync host import can block on its own I/O on that worker thread. Reconsider only if the host must run many concurrent calls into one instance, or if wasmtime's component-model-async (WASI p3) becomes the plugin API.

Notes for 2b-2:
- `workspace.exclude` takes path prefixes, not globs. `"experiments/*"` excludes nothing, so the root `Cargo.toml` has no `exclude` (only a comment). Every crate under `experiments/` or `plugins/` needs its own empty `[workspace]` table. It needs one in nested worktrees in any case, because Cargo keeps walking up to the main checkout's `Cargo.toml`.
- After a trap, drop the `Store`, the instance and any pending reply, and re-instantiate from the cached `Component`. Re-instantiation takes 17 µs. Do not reuse a store after a trap.
- Pick one epoch tick for the engine; 10 ms is plenty. The hard deadline is `ceil(budget / tick)` ticks, reset before every call.

## Not measured

- **A real plugin.** The guest's `search` does no work, so these numbers isolate the boundary cost.
- **macOS and Windows.** Linux only; numbers on slower CPUs will be higher.
- **Pooling allocator.** Not tried, because instantiation is already ~17 µs.
- **The tomakast daemon's own release size.** The table above gives the size added by wasmtime. Building the GPUI daemon in release was out of the time-box.
- **Many threads.** The worker-thread round trip was measured with one plugin. Contention with 10+ worker threads on fewer cores was not measured.
- **Async wasmtime.** Not built. The ruling rests on the sync numbers being far under budget, not on a measured comparison.
