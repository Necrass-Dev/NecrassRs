use super::FieldSet;
use crate::ast;
use crate::collections::HashSet;
use crate::executable::Selection;
use crate::validation::fragment::validate_fragments_used;
use crate::validation::operation::validate_defer;
use crate::validation::operation::validate_operation_definitions;
use crate::validation::selection::FieldsInSetCanMerge;
use crate::validation::variable::walk_selections_with_deduped_fragments;
use crate::validation::Details;
use crate::validation::DiagnosticList;
use crate::validation::ExecutableValidationContext;
use crate::validation::Valid;
use crate::validation::ValidationOptions;
use crate::ExecutableDocument;
use crate::Node;
use crate::Schema;

pub(crate) fn validate_executable_document(
    errors: &mut DiagnosticList,
    schema: &Schema,
    document: &ExecutableDocument,
) {
    validate_executable_document_with_options(
        errors,
        schema,
        document,
        ValidationOptions::default(),
    );
}

pub(crate) fn validate_executable_document_with_options(
    errors: &mut DiagnosticList,
    schema: &Schema,
    document: &ExecutableDocument,
    options: ValidationOptions,
) {
    if validate_with_or_without_schema(errors, Some(schema), document, options) {
        validate_with_schema(errors, schema, document);
    }
}

pub(crate) fn validate_standalone_executable_with_options(
    errors: &mut DiagnosticList,
    document: &ExecutableDocument,
    options: ValidationOptions,
) {
    validate_with_or_without_schema(errors, None, document, options);
}

fn validate_with_schema(
    errors: &mut DiagnosticList,
    schema: &Schema,
    document: &ExecutableDocument,
) {
    let alloc = typed_arena::Arena::new();
    let mut fields_in_set_can_merge = FieldsInSetCanMerge::new(&alloc, schema, document);
    for operation in document.operations.iter() {
        crate::validation::operation::validate_subscription(document, operation, errors);
        fields_in_set_can_merge.validate_operation(operation, errors);
    }
}

pub(crate) fn validate_with_or_without_schema(
    errors: &mut DiagnosticList,
    schema: Option<&Schema>,
    document: &ExecutableDocument,
    options: ValidationOptions,
) -> bool {
    if exceeds_validation_work(document, options) {
        errors.push(None, Details::ValidationWorkLimitExceeded);
        return false;
    }
    let context = ExecutableValidationContext::new(schema);
    validate_operation_definitions(errors, document, &context);
    validate_fragments_used(errors, document);
    validate_defer(document, errors);
    true
}

fn exceeds_validation_work(document: &ExecutableDocument, options: ValidationOptions) -> bool {
    let mut pending: Vec<_> = document
        .operations
        .iter()
        .map(|operation| &operation.selection_set)
        .chain(
            document
                .fragments
                .values()
                .map(|fragment| &fragment.selection_set),
        )
        .collect();
    let mut physical = document.fragments.values().fold(0usize, |work, fragment| {
        work.saturating_add(directive_work(&fragment.directives))
    });
    while let Some(set) = pending.pop() {
        for selection in &set.selections {
            physical = physical.saturating_add(selection_work(selection));
            match selection {
                Selection::Field(field) => pending.push(&field.selection_set),
                Selection::InlineFragment(fragment) => pending.push(&fragment.selection_set),
                Selection::FragmentSpread(_) => {}
            }
        }
    }

    // Permit large documents whose validation work grows with their physical selection count.
    let limit = physical.saturating_mul(options.work_multiplier).max(2048);
    let mut expanded = 0usize;
    for operation in document.operations.iter() {
        let mut seen_fragments = HashSet::default();
        if let Ok(false) = walk_selections_with_deduped_fragments(
            document,
            &operation.selection_set,
            |selection| {
                expanded = expanded.saturating_add(selection_work(selection));
                if let Selection::FragmentSpread(spread) = selection {
                    if seen_fragments.insert(&spread.fragment_name) {
                        if let Some(fragment) = document.fragments.get(&spread.fragment_name) {
                            expanded =
                                expanded.saturating_add(directive_work(&fragment.directives));
                        }
                    }
                }
                expanded <= limit
            },
        ) {
            return true;
        }
    }
    false
}

fn selection_work(selection: &Selection) -> usize {
    match selection {
        Selection::Field(field) => {
            1 + argument_work(&field.arguments) + directive_work(&field.directives)
        }
        Selection::FragmentSpread(spread) => 1 + directive_work(&spread.directives),
        Selection::InlineFragment(fragment) => 1 + directive_work(&fragment.directives),
    }
}

fn directive_work(directives: &ast::DirectiveList) -> usize {
    directives.iter().fold(0usize, |work, directive| {
        work.saturating_add(1 + argument_work(&directive.arguments))
    })
}

fn argument_work(arguments: &[Node<ast::Argument>]) -> usize {
    let mut work = arguments.len();
    let mut values: Vec<_> = arguments.iter().map(|argument| &*argument.value).collect();
    while let Some(value) = values.pop() {
        work = work.saturating_add(1);
        match value {
            ast::Value::List(items) => values.extend(items.iter().map(|item| &**item)),
            ast::Value::Object(fields) => {
                values.extend(fields.iter().map(|(_, value)| &**value));
            }
            _ => {}
        }
    }
    work
}

pub(crate) fn validate_field_set(
    diagnostics: &mut DiagnosticList,
    schema: &Valid<Schema>,
    field_set: &FieldSet,
) {
    let document = &ExecutableDocument::new(); // No fragment definitions
    let context = ExecutableValidationContext::new(Some(schema));
    crate::validation::selection::validate_selection_set(
        diagnostics,
        document,
        Some((schema, &field_set.selection_set.ty)),
        &field_set.selection_set,
        &mut context.operation_context(&[]),
    )
}
