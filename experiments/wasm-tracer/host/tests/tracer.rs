//! Behaviour checks for the tracer host against the real guest component.
//! Build the guest first:
//! `cargo build --release --target wasm32-wasip2 --manifest-path experiments/wasm-tracer/guest/Cargo.toml`

use std::time::Duration;

use tracer_host::{
    Plugin, PluginOptions, RECURSIVE_WIT, Ticker, load_component, make_engine, make_linker,
    parse_wit,
};

#[test]
fn search_returns_the_requested_number_of_items() {
    let engine = make_engine(false);
    let component = load_component(&engine).unwrap();
    let linker = make_linker(&engine);
    let mut plugin = Plugin::new(&engine, &component, &linker, PluginOptions::default()).unwrap();

    let items = plugin.search("10 apps").unwrap();

    assert_eq!(items.len(), 10);
    assert_eq!(items[0].id, "tracer/0");
    assert_eq!(items[9].title, "Result 9 for 10 apps");
    assert_eq!(plugin.search("0").unwrap().len(), 0);
}

#[test]
fn epoch_deadline_interrupts_an_infinite_loop_within_100_ms() {
    let engine = make_engine(true);
    let component = load_component(&engine).unwrap();
    let linker = make_linker(&engine);
    let _ticker = Ticker::start(&engine, Duration::from_millis(1));
    let options = PluginOptions {
        epoch_deadline_ticks: Some(50),
        ..PluginOptions::default()
    };
    let mut plugin = Plugin::new(&engine, &component, &linker, options).unwrap();

    let (result, elapsed) = plugin.spin();

    assert!(result.is_err(), "spin must trap");
    assert!(
        elapsed < Duration::from_millis(100),
        "spin ran for {elapsed:?}"
    );
}

#[test]
fn store_limits_trap_a_guest_that_allocates_1_gib() {
    let engine = make_engine(false);
    let component = load_component(&engine).unwrap();
    let linker = make_linker(&engine);
    let options = PluginOptions {
        memory_limit: Some(64 << 20),
        ..PluginOptions::default()
    };
    let mut plugin = Plugin::new(&engine, &component, &linker, options).unwrap();

    assert!(plugin.hog().is_err(), "hog must trap under a 64 MiB limit");
}

#[test]
fn without_store_limits_the_same_guest_allocates_1_gib() {
    let engine = make_engine(false);
    let component = load_component(&engine).unwrap();
    let linker = make_linker(&engine);
    let mut plugin = Plugin::new(&engine, &component, &linker, PluginOptions::default()).unwrap();

    plugin.hog().expect("hog must succeed without a limit");
}

#[test]
fn wit_rejects_a_recursive_record() {
    let error = parse_wit(RECURSIVE_WIT).expect_err("recursive record must be rejected");
    assert!(
        error.contains("type `node` depends on itself"),
        "unexpected error: {error}"
    );
}

#[test]
fn the_same_wit_without_recursion_parses() {
    let control = RECURSIVE_WIT.replace("children: list<node>", "children: list<string>");
    assert_ne!(
        control, RECURSIVE_WIT,
        "the control must differ only in the recursive field"
    );

    parse_wit(&control).expect("the non-recursive control must parse");
}

#[test]
fn epoch_engine_without_a_deadline_traps_at_once() {
    let engine = make_engine(true);
    let component = load_component(&engine).unwrap();
    let linker = make_linker(&engine);

    // No `epoch_deadline_ticks`: the store's deadline stays at its initial value.
    let result = Plugin::new(&engine, &component, &linker, PluginOptions::default())
        .and_then(|mut plugin| plugin.search("1"));

    assert!(result.is_err(), "the default deadline must trap");
}
