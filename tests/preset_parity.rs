// SPDX-License-Identifier: MPL-2.0

use runyte::{
    command::Mode,
    keymap::{Keymap, actions, keymap_for},
};
mod preset_support;

// Each CI OS checks its actual build-target selection against independently
// frozen pre-YAML data. Loader unit tests additionally simulate every platform.
#[test]
fn runyte_preserves_the_complete_pre_yaml_registry() {
    for fast in [false, true] {
        preset_support::assert_registry(&keymap_for(fast), fast, std::env::consts::OS);
    }
}

#[test]
fn preferred_global_spellings_preserve_pre_yaml_ordering() {
    let rows = [false, true]
        .into_iter()
        .flat_map(|fast| preset_support::preferred_rows(&keymap_for(fast), fast))
        .collect();
    preset_support::assert_preferred(rows);
}
