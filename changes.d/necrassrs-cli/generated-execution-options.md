 -  Added editable `src/options.rs` to new Axum and Actix projects. Both
    generated handlers use its `ExecutionOptions` for request validation and
    execution. Existing projects can adopt the same `execute_with_options` call.
