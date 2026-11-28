//! The starter template spells out every execution option with its default value.

use necrassrs::ExecutionOptions;

#[path = "../templates/shared/options.rs"]
mod options;

#[test]
fn template_options_match_the_defaults() {
    assert_eq!(options::execution_options(), ExecutionOptions::default());
}
