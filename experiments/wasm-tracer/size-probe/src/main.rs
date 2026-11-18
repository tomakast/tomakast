//! A minimal "host" whose only job is to link wasmtime (or not) so release sizes can be compared.
//! Usage: `size-probe <component.wasm | component.cwasm>`.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: size-probe <component>");
    let bytes = std::fs::read(&path).expect("readable component");
    println!("read {} bytes", bytes.len());
    #[cfg(feature = "runtime")]
    run(&bytes);
}

#[cfg(feature = "runtime")]
fn run(bytes: &[u8]) {
    use wasmtime::component::{Component, Linker, Val};
    use wasmtime::{Engine, Store};

    let engine = Engine::default();
    #[cfg(feature = "compiler")]
    let component = if bytes.starts_with(b"\0asm") {
        Component::new(&engine, bytes).expect("component compiles")
    } else {
        // SAFETY: a probe; the input is a `.cwasm` produced by the same wasmtime version.
        unsafe { Component::deserialize(&engine, bytes).expect("component deserializes") }
    };
    #[cfg(not(feature = "compiler"))]
    // SAFETY: a probe; the input is a `.cwasm` produced by the same wasmtime version.
    let component =
        unsafe { Component::deserialize(&engine, bytes).expect("component deserializes") };

    #[cfg(feature = "wasi")]
    let (mut store, linker) = {
        struct State(wasmtime_wasi::WasiCtx, wasmtime::component::ResourceTable);
        impl wasmtime_wasi::WasiView for State {
            fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
                wasmtime_wasi::WasiCtxView {
                    ctx: &mut self.0,
                    table: &mut self.1,
                }
            }
        }
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).expect("WASI links");
        let state = State(
            wasmtime_wasi::WasiCtx::builder().build(),
            Default::default(),
        );
        (Store::new(&engine, state), linker)
    };
    #[cfg(not(feature = "wasi"))]
    let (mut store, linker) = (Store::new(&engine, ()), Linker::new(&engine));

    match linker.instantiate(&mut store, &component) {
        Ok(instance) => {
            let search = instance
                .get_func(&mut store, "search")
                .expect("search export");
            let mut results = [Val::Bool(false)];
            search
                .call(&mut store, &[Val::String("10".into())], &mut results)
                .expect("search runs");
            println!("search returned {:?}", results[0]);
        }
        Err(error) => println!("instantiate failed (expected without WASI): {error}"),
    }
}
