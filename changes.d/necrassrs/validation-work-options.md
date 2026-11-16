 -  Added `ExecutionOptions::validation_work_multiplier` so applications can
    choose the relative executable-validation limit. The default is 8; a value
    of zero applies only the fixed 2,048-work minimum. GraphQL requests cannot
    override this server setting. Existing exhaustive `ExecutionOptions` struct
    literals must add this field or use `..Default::default()`.
