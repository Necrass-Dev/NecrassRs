use super::FieldSet;
use crate::ast;
use crate::collections::HashMap;
use crate::collections::HashSet;
use crate::executable::Selection;
use crate::executable::SelectionSet;
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
use crate::Name;
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
    if exceeds_selection_depth(document) {
        errors.push(None, Details::RecursionLimitError);
        return false;
    }
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

/// Maximum composed selection depth of an operation or fragment definition.
///
/// The definition's own selection set has depth 1. Each non-empty selection set entered
/// through a field, an inline fragment, or a fragment spread adds one level, so fragment
/// spreads compose the depth of the fragments they reference. Later validation and
/// execution recurse over composed selection sets; rejecting deeper documents up front
/// bounds their stack use. The limit matches the existing selection walks.
const SELECTION_DEPTH_LIMIT: usize = 500;

/// Selection depth facts of one definition, collected without following fragment spreads.
struct LocalDepth {
    /// Depth of the deepest non-empty selection set in the definition itself.
    max_depth: usize,
    /// Referenced fragment index and the depth of the selection set containing the spread.
    spreads: Vec<(usize, usize)>,
}

fn local_depth(
    selection_set: &SelectionSet,
    fragment_indices: &HashMap<&Name, usize>,
) -> LocalDepth {
    let mut local = LocalDepth {
        max_depth: 0,
        spreads: Vec::new(),
    };
    let mut pending = vec![(selection_set, 1usize)];
    while let Some((set, depth)) = pending.pop() {
        if set.selections.is_empty() {
            continue;
        }
        local.max_depth = local.max_depth.max(depth);
        for selection in &set.selections {
            match selection {
                Selection::Field(field) => pending.push((&field.selection_set, depth + 1)),
                Selection::InlineFragment(fragment) => {
                    pending.push((&fragment.selection_set, depth + 1))
                }
                // Unknown fragments are reported by later validation.
                Selection::FragmentSpread(spread) => {
                    if let Some(&index) = fragment_indices.get(&spread.fragment_name) {
                        local.spreads.push((index, depth));
                    }
                }
            }
        }
    }
    local
}

/// Returns whether any operation or fragment definition exceeds the selection depth limit
/// once fragment spreads are composed.
///
/// Runs in time linear in the document size and without recursion. Fragments that form a
/// cycle get a conservative upper bound: the sum of their member depths. Cyclic documents are
/// invalid, and later cycle detection walks them with fragment-level deduplication, so the
/// bound must cover every acyclic path through the cycle rather than a single traversal order.
fn exceeds_selection_depth(document: &ExecutableDocument) -> bool {
    const CAP: usize = SELECTION_DEPTH_LIMIT + 1;

    let fragment_indices: HashMap<&Name, usize> = document
        .fragments
        .keys()
        .enumerate()
        .map(|(index, name)| (name, index))
        .collect();
    let fragments: Vec<LocalDepth> = document
        .fragments
        .values()
        .map(|fragment| local_depth(&fragment.selection_set, &fragment_indices))
        .collect();

    // Composed depth of each fragment, counting its own selection set as depth 1, capped at CAP.
    // Tarjan's algorithm finishes strongly connected components after all components they
    // reference, so referenced depths are final when a component is evaluated.
    let count = fragments.len();
    let mut reach = vec![0usize; count];
    let mut order = vec![usize::MAX; count];
    let mut low = vec![0usize; count];
    let mut component = vec![usize::MAX; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut next_order = 0;
    let mut next_component = 0;
    for root in 0..count {
        if order[root] != usize::MAX {
            continue;
        }
        order[root] = next_order;
        low[root] = next_order;
        next_order += 1;
        stack.push(root);
        on_stack[root] = true;
        let mut calls = vec![(root, 0usize)];
        while let Some(&(node, edge)) = calls.last() {
            if let Some(&(target, _)) = fragments[node].spreads.get(edge) {
                calls.last_mut().unwrap().1 += 1;
                if order[target] == usize::MAX {
                    order[target] = next_order;
                    low[target] = next_order;
                    next_order += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    calls.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(order[target]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] != order[node] {
                continue;
            }
            let mut members = Vec::new();
            loop {
                let member = stack.pop().unwrap();
                on_stack[member] = false;
                component[member] = next_component;
                members.push(member);
                if member == node {
                    break;
                }
            }
            let cyclic = members.len() > 1
                || fragments[node]
                    .spreads
                    .iter()
                    .any(|&(target, _)| target == node);
            let depth = if cyclic {
                let mut total = 0usize;
                let mut exit = 0usize;
                for &member in &members {
                    let local = &fragments[member];
                    let member_depth = local
                        .spreads
                        .iter()
                        .fold(local.max_depth, |depth, &(_, at)| depth.max(at));
                    total = total.saturating_add(member_depth);
                    for &(target, _) in &local.spreads {
                        if component[target] != next_component {
                            exit = exit.max(reach[target]);
                        }
                    }
                }
                total.saturating_add(exit)
            } else {
                let local = &fragments[node];
                local
                    .spreads
                    .iter()
                    .fold(local.max_depth, |depth, &(target, at)| {
                        depth.max(at.saturating_add(reach[target]))
                    })
            };
            for &member in &members {
                reach[member] = depth.min(CAP);
            }
            next_component += 1;
        }
    }

    if reach.iter().any(|&depth| depth > SELECTION_DEPTH_LIMIT) {
        return true;
    }
    document.operations.iter().any(|operation| {
        let local = local_depth(&operation.selection_set, &fragment_indices);
        local.max_depth > SELECTION_DEPTH_LIMIT
            || local
                .spreads
                .iter()
                .any(|&(target, at)| at.saturating_add(reach[target]) > SELECTION_DEPTH_LIMIT)
    })
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
