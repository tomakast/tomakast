//! Test and dev helper: runs the plugin compile child, like the hidden
//! `tomakast compile-plugin` subcommand, without the GPUI app.

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Err(e) = tomakast_plugin_host::compile::child_main_from_args(&args) {
        eprintln!("compile-plugin: {e:#}");
        std::process::exit(1);
    }
}
