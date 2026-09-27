//! Experimental default-cycle validators for issue #23, not production validation.
//! Run: cargo run -p necrassrs --release --locked --example default_cycle_comparison
use apollo_compiler::{Name, Schema, ast::Value, schema::ExtendedType};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    hint::black_box,
    time::Instant,
};

type Key = (Name, Name);

#[derive(Default)]
struct Work {
    fields: Cell<usize>,
    expansions: Cell<usize>,
    peak_depth: Cell<usize>,
}

fn roots(schema: &Schema) -> Vec<Key> {
    schema
        .types
        .values()
        .filter_map(|ty| match ty {
            ExtendedType::InputObject(input) => Some(input),
            _ => None,
        })
        .flat_map(|input| {
            input
                .fields
                .iter()
                .filter(|(_, field)| {
                    field.default_value.is_some()
                        && schema
                            .get_input_object(field.ty.inner_named_type())
                            .is_some()
                })
                .map(|(name, _)| (input.name.clone(), name.clone()))
        })
        .collect()
}

// Walk only explicit literal structure. The callback decides whether to expand
// a missing field's default immediately or record an edge for later traversal.
fn literal(
    schema: &Schema,
    ty: &Name,
    value: &Value,
    work: &Work,
    visit: &mut dyn FnMut(Key) -> bool,
) -> bool {
    match value {
        Value::List(items) => items
            .iter()
            .any(|item| literal(schema, ty, item, work, visit)),
        Value::Object(fields) => {
            let Some(input) = schema.get_input_object(ty) else {
                return false;
            };
            let supplied: HashMap<_, _> = fields.iter().map(|(k, v)| (k, v)).collect();
            input.fields.iter().any(|(name, field)| {
                work.fields.set(work.fields.get() + 1);
                let target = field.ty.inner_named_type();
                if schema.get_input_object(target).is_none() {
                    return false;
                }
                if let Some(value) = supplied.get(name) {
                    literal(schema, target, value, work, visit)
                } else if field.default_value.is_some() {
                    visit((input.name.clone(), name.clone()))
                } else {
                    false
                }
            })
        }
        _ => false,
    }
}

fn expand(schema: &Schema, key: &Key, work: &Work, visit: &mut dyn FnMut(Key) -> bool) -> bool {
    work.expansions.set(work.expansions.get() + 1);
    let field = &schema.get_input_object(&key.0).unwrap().fields[&key.1];
    literal(
        schema,
        field.ty.inner_named_type(),
        field.default_value.as_ref().unwrap(),
        work,
        visit,
    )
}

fn path_dfs(schema: &Schema, keys: &[Key], work: &Work) -> bool {
    fn visit(schema: &Schema, key: Key, active: &mut HashSet<Key>, work: &Work) -> bool {
        if !active.insert(key.clone()) {
            return true;
        }
        work.peak_depth.set(work.peak_depth.get().max(active.len()));
        let cycle = expand(schema, &key, work, &mut |next| {
            visit(schema, next, active, work)
        });
        active.remove(&key);
        cycle
    }
    let mut active = HashSet::new();
    keys.iter()
        .any(|key| visit(schema, key.clone(), &mut active, work))
}

fn graph(schema: &Schema, keys: &[Key], work: &Work) -> (bool, usize) {
    let indices: HashMap<_, _> = keys.iter().enumerate().map(|(i, k)| (k, i)).collect();
    let edges: Vec<Vec<usize>> = keys
        .iter()
        .map(|key| {
            let mut targets = Vec::new();
            expand(schema, key, work, &mut |next| {
                targets.push(indices[&next]);
                false
            });
            // Retain repeated edge occurrences: no sorting or extra hash set.
            targets
        })
        .collect();
    fn visit(i: usize, edges: &[Vec<usize>], state: &mut [u8], depth: usize, work: &Work) -> bool {
        match state[i] {
            1 => return true,
            2 => return false,
            _ => {}
        }
        state[i] = 1;
        work.peak_depth.set(work.peak_depth.get().max(depth));
        if edges[i]
            .iter()
            .any(|&next| visit(next, edges, state, depth + 1, work))
        {
            return true;
        }
        state[i] = 2;
        false
    }
    let mut state = vec![0; keys.len()];
    let cycle = (0..keys.len()).any(|i| visit(i, &edges, &mut state, 1, work));
    (cycle, edges.iter().map(Vec::len).sum())
}

fn schema(input: &str) -> Schema {
    // Deliberately bypass Apollo validation: this compares the missing rule itself.
    Schema::parse(
        format!("{input}\ntype Query {{ hello: String }}"),
        "comparison.graphql",
    )
    .unwrap()
}

fn check(input: &str, expected: bool) {
    let schema = schema(input);
    let keys = roots(&schema);
    assert_eq!(
        path_dfs(&schema, &keys, &Work::default()),
        expected,
        "{input}"
    );
    assert_eq!(
        graph(&schema, &keys, &Work::default()).0,
        expected,
        "{input}"
    );
}

fn correctness() {
    // Existing request/execution regressions supply these default structures.
    check("input Recursive { next: Recursive = {} }", true);
    check("input A { bs: [B] = [{}] } input B { a: A = {} }", true);
    check("input R { next: R = {next: null} }", false);
    check("input R { next: R }", false);
    check("input R { next: R = {next: {next: null}} }", false);
    check("input A { bs: [B] = [] } input B { a: A = {} }", false);
    check("input A { bs: [B] = {} } input B { a: A = {} }", true);
    check("input A { bs: [[B]] = [[{}]] } input B { a: A = {} }", true);
    check("input A { bs: [B] = [null] } input B { a: A = {} }", false);
    check("input R { next: R = null }", false);
    check("input R { next: R! }", false); // Type-cycle validity is a separate rule.
    check(
        "input R { a: R = {a: null, b: null} b: R = {a: null, b: null} }",
        false,
    );
    check("input R { a: R = {a: null} b: R = {b: null} }", true);

    // Exhaustive small family: absent/null/empty/explicitly terminating defaults.
    let defaults = ["", " = null", " = {}", " = {a: null, b: null}"];
    for a in defaults {
        for b in defaults {
            check(
                &format!("input R {{ a: R{a} b: R{b} }}"),
                a == " = {}" || b == " = {}",
            );
        }
    }
}

fn chain(depth: usize, branching: bool) -> String {
    let mut s = "input Leaf { value: String }\n".to_owned();
    for i in 0..depth {
        let target = if i + 1 == depth {
            "Leaf".to_owned()
        } else {
            format!("T{}", i + 1)
        };
        s.push_str(&format!("input T{i} {{ left: {target} = {{}}"));
        if branching {
            s.push_str(&format!(" right: {target} = {{}}"));
        }
        s.push_str(" }\n");
    }
    s
}

fn dense(width: usize) -> String {
    let mut s = "input Roots { ".to_owned();
    for i in 0..width {
        s.push_str(&format!("r{i}: Wide = {{}} "));
    }
    s.push_str("} input Wide { ");
    for i in 0..width {
        s.push_str(&format!("f{i}: Leaf = {{}} "));
    }
    s.push_str("} input Leaf { value: String }");
    s
}

fn measure(label: &str, input: &str, expected: bool) {
    let schema = schema(input);
    let keys = roots(&schema);
    for algorithm in ["path", "graph"] {
        let mut times = Vec::new();
        let mut fields = 0;
        let mut expansions = 0;
        let mut depth = 0;
        let mut edges = 0;
        // One warm-up and seven measured samples. Include graph construction.
        for sample in 0..8 {
            let work = Work::default();
            let start = Instant::now();
            let cycle = if algorithm == "path" {
                path_dfs(black_box(&schema), black_box(&keys), &work)
            } else {
                let result = graph(black_box(&schema), black_box(&keys), &work);
                edges = result.1;
                result.0
            };
            let elapsed = start.elapsed().as_nanos();
            assert_eq!(black_box(cycle), expected);
            if sample > 0 {
                times.push(elapsed);
            }
            fields = work.fields.get();
            expansions = work.expansions.get();
            depth = work.peak_depth.get();
        }
        times.sort_unstable();
        println!(
            "{label},{algorithm},{},{},{},{fields},{expansions},{edges},{depth}",
            input.len(),
            keys.len(),
            times[3]
        );
    }
}

fn main() {
    correctness();
    println!(
        "case,algorithm,sdl_bytes,default_nodes,median_ns,field_checks,default_expansions,stored_edges,peak_default_depth"
    );
    measure("self_cycle", "input R { next: R = {} }", true);
    measure("finite", "input R { next: R = {next: null} }", false);
    for depth in [8, 12, 16, 20] {
        measure(&format!("branch_{depth}"), &chain(depth, true), false);
    }
    for depth in [16, 64, 256] {
        measure(&format!("chain_{depth}"), &chain(depth, false), false);
    }
    for width in [8, 32, 128, 256] {
        measure(&format!("dense_{width}"), &dense(width), false);
    }
    measure(
        "early_cycle_dense_256",
        &format!("input R {{ next: R = {{}} }} {}", dense(256)),
        true,
    );
}

#[test]
fn candidates_match_expected_default_cycles() {
    correctness();
}
