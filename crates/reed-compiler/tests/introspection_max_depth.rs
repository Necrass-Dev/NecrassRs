use expect_test::expect;
use reed_compiler::introspection;
use reed_compiler::ExecutableDocument;
use reed_compiler::Schema;

#[test]
fn test_3_sibling_fields_list() {
    let doc = r#"
      {
        __type(name: "Query") {
          trueFields: fields(includeDeprecated: true) {
            name
          }
          falseFields: fields(includeDeprecated: false) {
            name
          }
          omittedFields: fields {
            name
          }
        }
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_2_nested_fields_lists() {
    let doc = r#"
      {
        __type(name: "Query") {
          fields {
            type {
              fields {
                name
              }
            }
          }
        }
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_3_nested_fields_lists() {
    let doc = r#"
      {
        __type(name: "Query") {
          fields {
            type {
              fields {
                type {
                  fields {
                    name
                  }
                }
              }
            }
          }
        }
      }
    "#;
    let expected = expect![[r#"
        GraphQLError {
            message: "Maximum introspection depth exceeded",
            locations: [
                8:19,
            ],
            path: [],
            extensions: {},
        }
    "#]];
    assert_split(doc, expected);
}

#[test]
fn test_2_nested_input_fields_lists() {
    let doc = r#"
      {
        __type(name: "Query") {
          inputFields {
            type {
              inputFields {
                name
              }
            }
          }
        }
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_3_nested_input_fields_lists() {
    let doc = r#"
      {
        __type(name: "Query") {
          inputFields {
            type {
              inputFields {
                type {
                  inputFields {
                    name
                  }
                }
              }
            }
          }
        }
      }
    "#;
    let expected = expect![[r#"
        GraphQLError {
            message: "Maximum introspection depth exceeded",
            locations: [
                8:19,
            ],
            path: [],
            extensions: {},
        }
    "#]];
    assert_split(doc, expected);
}

#[test]
fn test_2_nested_interfaces_lists() {
    let doc = r#"
      {
        __schema {
          types {
            interfaces {
              interfaces {
                name
              }
            }
          }
        }
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_3_nested_interfaces_lists() {
    let doc = r#"
      {
        __schema {
          types {
            interfaces {
              interfaces {
                interfaces {
                  name
                }
              }
            }
          }
        }
      }
    "#;
    let expected = expect![[r#"
        GraphQLError {
            message: "Maximum introspection depth exceeded",
            locations: [
                7:17,
            ],
            path: [],
            extensions: {},
        }
    "#]];
    assert_split(doc, expected);
}

#[test]
fn test_2_nested_possible_types_lists() {
    let doc = r#"
      {
        __schema {
          types {
            possibleTypes {
              possibleTypes {
                name
              }
            }
          }
        }
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_3_nested_possible_types_lists() {
    let doc = r#"
      {
        __schema {
          types {
            possibleTypes {
              possibleTypes {
                possibleTypes {
                  name
                }
              }
            }
          }
        }
      }
    "#;
    let expected = expect![[r#"
        GraphQLError {
            message: "Maximum introspection depth exceeded",
            locations: [
                7:17,
            ],
            path: [],
            extensions: {},
        }
    "#]];
    assert_split(doc, expected);
}

#[test]
fn test_2_nested_possible_types_lists_with_inline_fragments() {
    let doc = r#"
      {
        ... on Query {
          __schema {
            types {
              ... on __Type {
                possibleTypes {
                  ... on __Type {
                    possibleTypes {
                      ... on __Type {
                        name
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_3_nested_possible_types_lists_with_inline_fragments() {
    let doc = r#"
      {
        ... on Query {
          __schema {
            types {
              ... on __Type {
                possibleTypes {
                  ... on __Type {
                    possibleTypes {
                      ... on __Type {
                        possibleTypes {
                          ... on __Type {
                            name
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    "#;
    let expected = expect![[r#"
        GraphQLError {
            message: "Maximum introspection depth exceeded",
            locations: [
                11:25,
            ],
            path: [],
            extensions: {},
        }
    "#]];
    assert_split(doc, expected);
}

#[test]
fn test_2_nested_possible_types_lists_with_named_fragments() {
    let doc = r#"
      {
        __schema {
          types {
            ...One
          }
        }
      }
      fragment One on __Type {
        possibleTypes {
          ...Two
        }
      }
      fragment Two on __Type {
        possibleTypes {
          ...Three
        }
      }
      fragment Three on __Type {
        name
      }
    "#;
    let expected = expect!["Ok"];
    assert_split(doc, expected);
}

#[test]
fn test_3_nested_possible_types_lists_with_named_fragments() {
    let doc = r#"
      {
        __schema {
          types {
            ...One
          }
        }
      }
      fragment One on __Type {
        possibleTypes {
          ...Two
        }
      }
      fragment Two on __Type {
        possibleTypes {
          ...Three
        }
      }
      fragment Three on __Type {
        possibleTypes {
          ...Four
        }
      }
      fragment Four on __Type {
        name
      }
    "#;
    let expected = expect![[r#"
        GraphQLError {
            message: "Maximum introspection depth exceeded",
            locations: [
                20:9,
            ],
            path: [],
            extensions: {},
        }
    "#]];
    assert_split(doc, expected);
}

#[track_caller]
fn assert_split(doc: &str, expected: expect_test::Expect) {
    let schema = "type Query { f: Int }";
    let schema = Schema::parse_and_validate(schema, "schema.graphql").unwrap();
    let doc = ExecutableDocument::parse_and_validate(&schema, doc, "doc.graphql").unwrap();
    let operation = doc.operations.get(None).unwrap();

    match introspection::check_max_depth(&doc, operation) {
        Ok(_) => expected.assert_eq("Ok"),
        Err(err) => expected.assert_debug_eq(&err.to_graphql_error(&doc.sources)),
    }
}

// Regression cases for named fragments reused at a different list depth than their first
// use. A cache hit must reject at the same limit as direct traversal and contribute its depth
// to any enclosing fragment.

#[test]
fn test_cached_fragment_reused_below_two_lists() {
    let doc = r#"
      {
        shallow: __type(name: "Query") { ...OneList }
        deep: __type(name: "Query") {
          fields { type { fields { type { ...OneList } } } }
        }
      }
      fragment OneList on __Type { fields { name } }
    "#;
    assert_rejected(doc);
}

#[test]
fn test_enclosing_fragment_counts_cached_child() {
    let doc = r#"
      {
        a: __type(name: "Query") { ...Child }
        b: __type(name: "Query") { ...Parent }
        c: __type(name: "Query") { fields { type { fields { type { ...Parent } } } } }
      }
      fragment Child on __Type { fields { name } }
      fragment Parent on __Type { fields { type { ...Child } } }
    "#;
    assert_rejected(doc);
}

#[test]
fn test_enclosing_fragment_counts_child_cached_two_lists_deep() {
    let doc = r#"
      {
        a: __type(name: "Query") { fields { type { ...B } } }
        b: __type(name: "Query") { fields { type { fields { type { ...A } } } } }
      }
      fragment B on __Type { name }
      fragment A on __Type { fields { type { ...B } } }
    "#;
    assert_rejected(doc);
}

#[test]
fn test_fragment_chain_primed_by_shallow_aliases() {
    let doc = r#"
      {
        f: __type(name: "Query") { ...F }
        g: __type(name: "Query") { ...G }
        h: __type(name: "Query") { ...H }
      }
      fragment F on __Type { fields { name } }
      fragment G on __Type { fields { type { ...F } } }
      fragment H on __Type { fields { type { ...G } } }
    "#;
    assert_rejected(doc);
}

#[test]
fn test_fragment_chain_primed_by_skipped_spreads() {
    let doc = r#"
      {
        __type(name: "Query") {
          ...F1 @skip(if: true)
          ...F2 @skip(if: true)
          ...F3 @skip(if: true)
          ...F4 @skip(if: true)
          ...F5
        }
      }
      fragment F1 on __Type { fields { name } }
      fragment F2 on __Type { fields { type { ...F1 } } }
      fragment F3 on __Type { fields { type { ...F2 } } }
      fragment F4 on __Type { fields { type { ...F3 } } }
      fragment F5 on __Type { fields { type { ...F4 } } }
    "#;
    assert_rejected(doc);
}

#[test]
fn test_cached_fragment_reused_within_the_limit() {
    let doc = r#"
      {
        shallow: __type(name: "Query") { ...OneList ...NoList }
        deep: __type(name: "Query") {
          fields { type { ...OneList fields { type { ...NoList } } } }
        }
      }
      fragment OneList on __Type { fields { name } }
      fragment NoList on __Type { name ofType { name } }
    "#;
    assert_eq!(check(doc, introspection::DEFAULT_MAX_LIST_DEPTH), Ok(()));
}

#[test]
fn test_custom_list_depth_limit() {
    let three_lists = r#"
      {
        shallow: __type(name: "Query") { ...OneList }
        deep: __type(name: "Query") {
          fields { type { fields { type { ...OneList } } } }
        }
      }
      fragment OneList on __Type { fields { name } }
    "#;
    assert_eq!(check(three_lists, 3), Ok(()));
    assert!(check(three_lists, 2).is_err());

    let four_lists = r#"
      {
        a: __type(name: "Query") { ...Child }
        b: __type(name: "Query") { ...Parent }
        c: __type(name: "Query") { fields { type { fields { type { ...Parent } } } } }
      }
      fragment Child on __Type { fields { name } }
      fragment Parent on __Type { fields { type { ...Child } } }
    "#;
    assert_eq!(check(four_lists, 4), Ok(()));
    assert!(check(four_lists, 3).is_err());

    assert!(check(r#"{ __type(name: "Query") { fields { name } } }"#, 0).is_err());
    assert_eq!(
        check(r#"{ __type(name: "Query") { name ofType { name } } }"#, 0),
        Ok(())
    );
}

/// Compares the checker with an oracle that expands every fragment spread, over generated
/// documents that reuse fragments at different depths and in different orders.
#[test]
fn test_generated_documents_match_expanded_traversal() {
    let schema = Schema::parse_and_validate("type Query { f: Int }", "schema.graphql").unwrap();
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    let mut checked = 0;
    for _ in 0..3000 {
        let generated = GeneratedDocument::new(&mut rng);
        let text = generated.render();
        let Ok(doc) = ExecutableDocument::parse_and_validate(&schema, &text, "doc.graphql") else {
            continue;
        };
        let operation = doc.operations.get(None).unwrap();
        let expected_depth = generated.expanded_depth();
        for limit in 0..=4 {
            let accepted =
                introspection::check_max_depth_with_limit(&doc, operation, limit).is_ok();
            assert_eq!(
                accepted,
                expected_depth <= limit,
                "limit {limit}, expanded depth {expected_depth}:\n{text}"
            );
        }
        checked += 1;
    }
    assert!(
        checked > 2500,
        "only {checked} generated documents were valid"
    );
}

struct XorShift(u64);

impl XorShift {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

/// A selection on \`__Type\`.
enum TypeSelection {
    Name,
    OfType(Vec<TypeSelection>),
    /// \`fields\`, \`inputFields\`, \`interfaces\`, or \`possibleTypes\`.
    List(usize, Vec<TypeSelection>),
    Inline(Vec<TypeSelection>),
    Spread {
        fragment: usize,
        skipped: bool,
    },
}

struct GeneratedDocument {
    /// Fragment \`i\` spreads only fragments before it, so spreads are acyclic.
    fragments: Vec<Vec<TypeSelection>>,
    roots: Vec<Vec<TypeSelection>>,
}

impl GeneratedDocument {
    fn new(rng: &mut XorShift) -> Self {
        let fragment_count = 1 + rng.below(6);
        let fragments = (0..fragment_count)
            .map(|index| selections(rng, 0, index))
            .collect();
        let mut roots: Vec<_> = (0..1 + rng.below(3))
            .map(|_| selections(rng, 0, fragment_count))
            .collect();
        // Use every fragment so that validation accepts the document, at a random position so
        // that it sometimes primes the cache and sometimes reuses it.
        let all_fragments = (0..fragment_count)
            .map(|fragment| TypeSelection::Spread {
                fragment,
                skipped: rng.below(2) == 0,
            })
            .collect();
        let position = rng.below(roots.len() + 1);
        roots.insert(position, all_fragments);
        Self { fragments, roots }
    }

    fn render(&self) -> String {
        let mut text = String::from("{");
        for (index, root) in self.roots.iter().enumerate() {
            text.push_str(&format!(" r{index}: __type(name: \"Query\") {{"));
            render(&mut text, root);
            text.push_str(" }");
        }
        text.push_str(" }");
        for (index, fragment) in self.fragments.iter().enumerate() {
            text.push_str(&format!(" fragment F{index} on __Type {{"));
            render(&mut text, fragment);
            text.push_str(" }");
        }
        text
    }

    /// The deepest list nesting with every fragment spread expanded, including skipped ones.
    fn expanded_depth(&self) -> usize {
        self.roots
            .iter()
            .map(|root| self.depth(root))
            .max()
            .unwrap_or(0)
    }

    fn depth(&self, selections: &[TypeSelection]) -> usize {
        selections
            .iter()
            .map(|selection| match selection {
                TypeSelection::Name => 0,
                TypeSelection::OfType(nested) | TypeSelection::Inline(nested) => self.depth(nested),
                TypeSelection::List(_, nested) => 1 + self.depth(nested),
                TypeSelection::Spread { fragment, .. } => self.depth(&self.fragments[*fragment]),
            })
            .max()
            .unwrap_or(0)
    }
}

fn selections(rng: &mut XorShift, depth: usize, spreadable: usize) -> Vec<TypeSelection> {
    (0..1 + rng.below(3))
        .map(|_| {
            let choice = rng.below(10);
            if depth >= 5 || choice < 2 {
                TypeSelection::Name
            } else if choice < 5 {
                TypeSelection::List(rng.below(4), selections(rng, depth + 1, spreadable))
            } else if choice < 6 {
                TypeSelection::OfType(selections(rng, depth + 1, spreadable))
            } else if choice < 7 {
                TypeSelection::Inline(selections(rng, depth + 1, spreadable))
            } else if spreadable > 0 {
                TypeSelection::Spread {
                    fragment: rng.below(spreadable),
                    skipped: rng.below(3) == 0,
                }
            } else {
                TypeSelection::Name
            }
        })
        .collect()
}

fn render(text: &mut String, selections: &[TypeSelection]) {
    for selection in selections {
        match selection {
            TypeSelection::Name => text.push_str(" name"),
            TypeSelection::OfType(nested) => {
                text.push_str(" ofType {");
                render(text, nested);
                text.push_str(" }");
            }
            TypeSelection::List(kind, nested) => {
                let (open, close) = match kind {
                    0 => (" fields { type {", " } }"),
                    1 => (" inputFields { type {", " } }"),
                    2 => (" interfaces {", " }"),
                    _ => (" possibleTypes {", " }"),
                };
                text.push_str(open);
                render(text, nested);
                text.push_str(close);
            }
            TypeSelection::Inline(nested) => {
                text.push_str(" ... on __Type {");
                render(text, nested);
                text.push_str(" }");
            }
            TypeSelection::Spread { fragment, skipped } => {
                text.push_str(&format!(" ...F{fragment}"));
                if *skipped {
                    text.push_str(" @skip(if: true)");
                }
            }
        }
    }
}

#[track_caller]
fn assert_rejected(doc: &str) {
    let error = check(doc, introspection::DEFAULT_MAX_LIST_DEPTH).unwrap_err();
    assert_eq!(error, "Maximum introspection depth exceeded");
}

fn check(doc: &str, max_list_depth: usize) -> Result<(), String> {
    let schema = Schema::parse_and_validate("type Query { f: Int }", "schema.graphql").unwrap();
    let doc = ExecutableDocument::parse_and_validate(&schema, doc, "doc.graphql").unwrap();
    let operation = doc.operations.get(None).unwrap();
    introspection::check_max_depth_with_limit(&doc, operation, max_list_depth)
        .map_err(|error| error.message().to_string())
}
