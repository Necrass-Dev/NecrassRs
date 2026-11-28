//! The starter template spells out every execution option with its default value.
//!
//! This check lives in an unpublished crate so that `necrassrs-cli` keeps no
//! dependency on other workspace crates and can be published first.

use necrassrs::ExecutionOptions;

#[path = "../../../crates/necrassrs-cli/templates/shared/options.rs"]
mod options;

#[test]
fn template_options_match_the_defaults() {
    assert_eq!(options::execution_options(), ExecutionOptions::default());
}
