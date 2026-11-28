 -  The starter's `src/options.rs` now lists every `ExecutionOptions` field
    with its default value instead of `..Default::default()`, so each
    execution limit can be adjusted in place. Upgrading NecrassRs with a new
    option makes the file fail to compile until the new field is added.
