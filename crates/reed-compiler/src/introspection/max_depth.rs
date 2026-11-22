use crate::collections::HashMap;
use crate::executable::Selection;
use crate::executable::SelectionSet;
use crate::parser::SourceSpan;
use crate::request::RequestError;
use crate::validation::Valid;
use crate::ExecutableDocument;
use crate::Name;

/// Returns the greatest introspection list depth reached in `selection_set`.
///
/// `depth_so_far` is the list depth at which `selection_set` is entered. It never exceeds
/// `max_list_depth`, because every increment is checked before recursing.
///
/// `fragment_depths` caches the list depth each named fragment adds relative to the depth
/// at which it is entered. Validation guarantees that fragment spreads are acyclic, so this
/// relative depth does not depend on where the fragment is used. A cache hit must apply the
/// same limit check and contribute the same depth to the result as traversing the fragment
/// again; otherwise reusing a fragment deeper in the document, or enclosing it in another
/// fragment, would understate the depth.
pub(super) fn check_selection_set<'doc>(
    document: &'doc Valid<ExecutableDocument>,
    fragment_depths: &mut HashMap<&'doc Name, usize>,
    max_list_depth: usize,
    depth_so_far: usize,
    selection_set: &'doc SelectionSet,
) -> Result<usize, RequestError> {
    let mut max_depth = depth_so_far;
    for selection in &selection_set.selections {
        match selection {
            Selection::InlineFragment(inline) => {
                max_depth = max_depth.max(check_selection_set(
                    document,
                    fragment_depths,
                    max_list_depth,
                    depth_so_far,
                    &inline.selection_set,
                )?)
            }
            Selection::FragmentSpread(spread) => {
                let Some(def) = document.fragments.get(&spread.fragment_name) else {
                    continue;
                };
                // Avoiding the entry API because we may have to modify the map in-between this `.get()`
                // and the `.insert()`.
                if let Some(fragment_depth) = fragment_depths.get(&spread.fragment_name) {
                    let post_fragment_depth = depth_so_far.saturating_add(*fragment_depth);
                    check_list_depth(post_fragment_depth, max_list_depth, spread.location())?;
                    max_depth = max_depth.max(post_fragment_depth);
                } else {
                    // Recursing without marking our fragment spread as used is fine,
                    // because validation guarantees that we do not have a self-referential
                    // fragment chain.
                    let post_fragment_depth = check_selection_set(
                        document,
                        fragment_depths,
                        max_list_depth,
                        depth_so_far,
                        &def.selection_set,
                    )?;
                    fragment_depths
                        .insert(&spread.fragment_name, post_fragment_depth - depth_so_far);
                    max_depth = max_depth.max(post_fragment_depth);
                }
            }
            Selection::Field(field) => {
                let mut depth = depth_so_far;
                if matches!(
                    field.name.as_str(),
                    "fields" | "interfaces" | "possibleTypes" | "inputFields"
                ) {
                    depth = depth.saturating_add(1);
                    check_list_depth(depth, max_list_depth, field.name.location())?;
                }
                max_depth = max_depth.max(check_selection_set(
                    document,
                    fragment_depths,
                    max_list_depth,
                    depth,
                    &field.selection_set,
                )?)
            }
        }
    }
    Ok(max_depth)
}

/// The single limit check shared by direct traversal and cached fragment reuse.
fn check_list_depth(
    list_depth: usize,
    max_list_depth: usize,
    location: Option<SourceSpan>,
) -> Result<(), RequestError> {
    if list_depth > max_list_depth {
        return Err(RequestError {
            message: "Maximum introspection depth exceeded".into(),
            location,
            is_suspected_validation_bug: false,
        });
    }
    Ok(())
}
