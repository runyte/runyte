// SPDX-License-Identifier: MPL-2.0
//! Fresh-process configured compilation and real semantic config-reload timings.
use runyte::{
    app::App,
    command::parse_colon_command,
    config::Config,
    keymap::{configured, default_keymap},
    test_support::isolated_startup_config,
};
use std::{fs, time::Instant};

fn main() {
    let root = runyte::test_support::OwnedTempDir::new().unwrap();
    let path = root.path().join("config.yaml");
    let preset = Instant::now();
    let defaults = default_keymap();
    let preset_ns = preset.elapsed().as_nanos();
    for (case, source) in [
        ("empty", "mode: ide\nlsp:\n  enable: false\n"),
        (
            "preview",
            "mode: ide\nlsp:\n  enable: false\neditor:\n  fast_pane_keys: true\n  preview_theme_colors: false\nkeys:\n  bind:\n    normal:\n      Ctrl-Left: focus-window-left\n      Ctrl-Right: focus-window-right\n      Ctrl-Up: focus-window-up\n      Ctrl-Down: focus-window-down\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let config: Config = serde_yaml::from_str(source).unwrap();
        configured::take_mode_flow_time_ns();
        let start = Instant::now();
        let compiled = configured::compile_configuration(&config, defaults);
        let compile_ns = start.elapsed().as_nanos();
        let compile_flow_ns = configured::take_mode_flow_time_ns();
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let mut app = App::new(isolated_startup_config(config.clone()), None).unwrap();
        // Measure an edit to a running fixture configuration, after startup
        // has validated both variants. Cold compilation remains separate above.
        if case == "preview" {
            fs::write(&path, format!("{source}      F6: move-right\n")).unwrap();
        }
        app.note_loaded_config(&path);
        configured::take_mode_flow_time_ns();
        let start = Instant::now();
        let outcome = app
            .execute(parse_colon_command("config-reload").unwrap())
            .unwrap();
        let reload_ns = start.elapsed().as_nanos();
        let reload_flow_ns = configured::take_mode_flow_time_ns();
        let start = Instant::now();
        app.execute(parse_colon_command("config-reload").unwrap())
            .unwrap();
        let unchanged_reload_ns = start.elapsed().as_nanos();
        let unchanged_reload_flow_ns = configured::take_mode_flow_time_ns();
        println!(
            "{{\"case\":\"{case}\",\"preset_ns\":{preset_ns},\"compile_ns\":{compile_ns},\"compile_flow_ns\":{compile_flow_ns},\"reload_ns\":{reload_ns},\"reload_flow_ns\":{reload_flow_ns},\"unchanged_reload_ns\":{unchanged_reload_ns},\"unchanged_reload_flow_ns\":{unchanged_reload_flow_ns}}}"
        );
        std::hint::black_box(outcome);
    }
}
