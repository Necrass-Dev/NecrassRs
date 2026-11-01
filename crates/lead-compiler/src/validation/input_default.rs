//! InputObjectDefaultValueHasCycle, GraphQL September 2025.
use crate::ast::{InputValueDefinition, Value};
use crate::collections::HashMap;
use crate::schema::ExtendedType;
use crate::validation::diagnostics::DiagnosticData;
use crate::validation::DiagnosticList;
use crate::{Name, Node, Schema};

pub(crate) fn validate_input_defaults(diagnostics: &mut DiagnosticList, schema: &Schema) {
    // Nodes identify defaults, not types: explicit values can terminate recursion.
    let nodes: Vec<(Name, &Node<InputValueDefinition>)> = schema
        .types
        .values()
        .filter_map(|ty| match ty {
            ExtendedType::InputObject(input) => Some(input),
            _ => None,
        })
        .flat_map(|input| {
            input.fields.values().filter_map(|field| {
                if field.default_value.is_some()
                    && schema
                        .get_input_object(field.ty.inner_named_type())
                        .is_some()
                {
                    Some((input.name.clone(), &field.node))
                } else {
                    None
                }
            })
        })
        .collect();
    let indices: HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, (ty, field))| ((ty.clone(), field.name.clone()), i))
        .collect();
    let mut edges = vec![Vec::new(); nodes.len()];
    for (i, (_, field)) in nodes.iter().enumerate() {
        let Some(default) = &field.default_value else {
            continue;
        };
        let mut pending = vec![(field.ty.inner_named_type(), default.as_ref())];
        while let Some((ty, value)) = pending.pop() {
            match value {
                Value::List(items) => {
                    pending.extend(items.iter().rev().map(|item| (ty, item.as_ref())));
                }
                Value::Object(fields) => {
                    let Some(input) = schema.get_input_object(ty) else {
                        continue;
                    };
                    let supplied: HashMap<_, _> = fields.iter().map(|(k, v)| (k, v)).collect();
                    for (name, field) in &input.fields {
                        let target = field.ty.inner_named_type();
                        if schema.get_input_object(target).is_none() {
                            continue;
                        }
                        if let Some(&value) = supplied.get(name) {
                            pending.push((target, value.as_ref()));
                        } else if let Some(&j) = indices.get(&(input.name.clone(), name.clone())) {
                            edges[i].push(j);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Iterative three-state DFS avoids call-stack growth on long default chains.
    // Repeated edge occurrences are harmless and retain O(V + E) traversal.
    let mut state = vec![0u8; nodes.len()];
    for root in 0..nodes.len() {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        let mut path = vec![(root, 0usize)];
        while let Some((node, next_edge)) = path.last_mut() {
            let Some(&next) = edges[*node].get(*next_edge) else {
                state[*node] = 2;
                path.pop();
                continue;
            };
            *next_edge += 1;
            match state[next] {
                0 => {
                    state[next] = 1;
                    path.push((next, 0));
                }
                1 => {
                    let trace = path
                        .iter()
                        .skip_while(|(i, _)| *i != next)
                        .map(|(i, _)| (nodes[*i].0.clone(), nodes[*i].1.clone()))
                        .collect();
                    diagnostics.push(
                        nodes[next]
                            .1
                            .default_value
                            .as_ref()
                            .and_then(|value| value.location()),
                        DiagnosticData::RecursiveInputDefault { trace },
                    );
                    return;
                }
                _ => {}
            }
        }
    }
}
