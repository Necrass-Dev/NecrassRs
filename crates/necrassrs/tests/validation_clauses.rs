//! Public-boundary checks for the clause inventory in archives/specs.md.
use apollo_compiler::{ExecutableDocument, Schema, ast};
use necrassrs::{Dispatcher, FieldCoordinate, JsonMap, Request, ResolvedValue, ResolverError};

#[test]
fn language_tokens_and_literal_values() {
    let source = "\u{feff}# ignored\r\nquery Q($s: String = \"\\u0041\\uD83D\\uDE00\", $b: String = \"\"\"\n  first\n    second\n\"\"\") { value }";
    let document = ast::Document::parse(source, "request.graphql").unwrap();
    let ast::Definition::OperationDefinition(operation) = &document.definitions[0] else {
        panic!("expected an operation");
    };
    assert_eq!(
        operation.variables[0]
            .default_value
            .as_ref()
            .unwrap()
            .as_str(),
        Some("A😀")
    );
    assert_eq!(
        operation.variables[1]
            .default_value
            .as_ref()
            .unwrap()
            .as_str(),
        Some("first\n  second")
    );
    for literal in [
        "0",
        "-1",
        "1.25e+2",
        "true",
        "false",
        "null",
        "ACTIVE",
        "[1, null, 2]",
        "{nested: [1]}",
    ] {
        ast::Document::parse(format!("{{ field(value: {literal}) }}"), "request.graphql").unwrap();
    }
    for source in [
        "{ field(value: 01) }",
        "{ field(value: 1.) }",
        "{ field(value: 1e) }",
        "{ field(value: 1abc) }",
        "{ field(value: +1) }",
        r#"{ field(value: "\uD800") }"#,
        r#"{ field(value: "\q") }"#,
        "{ field(value: \"line\nbreak\") }",
        "{ café }",
        "{ 1field }",
        "query Q($x: Int = $other) { value }",
        "{ ...on }",
    ] {
        assert!(
            ast::Document::parse(source, "request.graphql").is_err(),
            "{source}"
        );
    }
}

#[test]
fn schema_definition_and_extension_clauses() {
    let valid = r#"
        directive @mark repeatable on SCHEMA | SCALAR | OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT
        directive @meta on DIRECTIVE_DEFINITION
        directive @feature on FIELD
        extend directive @feature @meta
        schema { query: Query }
        extend schema @mark
        scalar Date
        extend scalar Date @mark
        interface Node { id: ID! }
        extend interface Node { label: String }
        type User implements Node { id: ID! label: String }
        type Query { value(input: Filter): User }
        extend type Query { search: Search }
        union Search = User
        type Other { value: Int }
        extend union Search = Other
        enum Status { ACTIVE }
        extend enum Status { LEGACY }
        input Filter { status: Status }
        extend input Filter { date: Date }
    "#;
    Schema::parse_and_validate(valid, "schema.graphql").unwrap();
    let invalid = [
        ("3.2 unique types", "type Query { value: Int } scalar Query"),
        ("3.2 reserved names", "type Query { __value: Int }"),
        ("3.3 query root required", "type Other { value: Int }"),
        ("3.3 object roots", "schema { query: Root } scalar Root"),
        (
            "3.3 distinct roots",
            "schema { query: Query mutation: Query } type Query { value: Int }",
        ),
        (
            "3.3 unique schema",
            "schema { query: Query } schema { query: Query } type Query { value: Int }",
        ),
        (
            "3.3.2 duplicate root extension",
            "type Query { value: Int } schema { query: Query } extend schema { query: Query }",
        ),
        ("3.4 input types", "type Query { value(input: Query): Int }"),
        (
            "3.4 output types",
            "input Input { value: Int } type Query { value: Input }",
        ),
        (
            "3.5 scalar extension target",
            "type Query { value: Int } extend scalar Missing @specifiedBy(url: \"x\")",
        ),
        (
            "3.6 field uniqueness",
            "type Query { value: Int value: Int }",
        ),
        ("3.6 known field types", "type Query { value: Missing }"),
        ("3.6 nonempty object", "type Query"),
        (
            "3.6.1 argument uniqueness",
            "type Query { value(x: Int, x: Int): Int }",
        ),
        (
            "3.6.3 extension kind",
            "type Query { value: Int } extend interface Query { other: Int }",
        ),
        (
            "3.6.3 duplicate extension field",
            "type Query { value: Int } extend type Query { value: Int }",
        ),
        (
            "3.7 required interface fields",
            "interface Node { id: ID! } type Query implements Node { value: Int }",
        ),
        (
            "3.7 covariant field type",
            "interface Node { id: ID! } type Query implements Node { id: String! }",
        ),
        (
            "3.7 invariant argument type",
            "interface Node { value(x: Int): Int } type Query implements Node { value(x: String): Int }",
        ),
        (
            "3.7 no additional required argument",
            "interface Node { value: Int } type Query implements Node { value(x: Int!): Int }",
        ),
        (
            "3.7 transitive interfaces",
            "interface Node { id: ID! } interface Named implements Node { id: ID! } type Query implements Named { id: ID! }",
        ),
        (
            "3.7 no self implementation",
            "interface Node implements Node { id: ID! } type Query { value: Int }",
        ),
        (
            "3.7.1 duplicate interface field extension",
            "interface Node { id: ID! } extend interface Node { id: ID! } type Query { value: Int }",
        ),
        (
            "3.8 object union members",
            "union Result = Int type Query { value: Result }",
        ),
        (
            "3.8 unique union members",
            "union Result = Query | Query type Query { value: Int }",
        ),
        (
            "3.8 nonempty union",
            "union Result type Query { value: Int }",
        ),
        (
            "3.8.1 duplicate union extension",
            "union Result = Query extend union Result = Query type Query { value: Int }",
        ),
        (
            "3.9 unique enum values",
            "enum Status { ACTIVE ACTIVE } type Query { value: Status }",
        ),
        (
            "3.9 reserved enum values",
            "enum Status { true } type Query { value: Status }",
        ),
        ("3.9 nonempty enum", "enum Status type Query { value: Int }"),
        (
            "3.9.1 duplicate enum extension",
            "enum Status { ACTIVE } extend enum Status { ACTIVE } type Query { value: Int }",
        ),
        (
            "3.10 unique input fields",
            "input Input { x: Int x: Int } type Query { value: Int }",
        ),
        (
            "3.10 nonempty input",
            "input Input type Query { value: Int }",
        ),
        (
            "3.10.2 duplicate input extension",
            "input Input { x: Int } extend input Input { x: Int } type Query { value: Int }",
        ),
        (
            "3.13 known directives",
            "type Query @missing { value: Int }",
        ),
        (
            "3.13 directive locations",
            "type Query @skip(if: true) { value: Int }",
        ),
        (
            "3.13 unique directives",
            "directive @d on OBJECT type Query @d @d { value: Int }",
        ),
        (
            "3.13 unique definitions",
            "directive @d on FIELD directive @d on FIELD type Query { value: Int }",
        ),
        (
            "3.13 directive argument types",
            "directive @d(x: Query) on FIELD type Query { value: Int }",
        ),
        (
            "3.13.6 existing directive",
            "extend directive @missing @deprecated type Query { value: Int }",
        ),
        (
            "3.13.6 duplicate annotation",
            "directive @d @deprecated on FIELD extend directive @d @deprecated type Query { value: Int }",
        ),
        (
            "3.13.6 direct cycle",
            "directive @d on DIRECTIVE_DEFINITION extend directive @d @d type Query { value: Int }",
        ),
        (
            "3.13.6 indirect cycle",
            "directive @a on DIRECTIVE_DEFINITION directive @b @a on DIRECTIVE_DEFINITION extend directive @a @b type Query { value: Int }",
        ),
        (
            "3.13.6 cycle through input",
            "directive @d(x: Input) on INPUT_FIELD_DEFINITION input Input { x: Int @d } type Query { value: Int }",
        ),
    ];
    for (clause, source) in invalid {
        let errors = Schema::parse_and_validate(source, "schema.graphql").expect_err(clause);
        assert!(!errors.errors.is_empty(), "{clause}");
    }
}

const SCHEMA: &str = r#"
    directive @tag(x: Int!) repeatable on FIELD
    enum Status { ACTIVE }
    input Filter { required: Int! optional: Int }
    interface Node { id: ID! }
    type User implements Node { id: ID! }
    type Other { value: Int }
    union Search = User | Other
    type Query {
        value(x: Int, required: Int! = 1, status: Status, filter: Filter): Int
        needs(x: Int!): Int
        user: User
        node: Node
        search: Search
    }
"#;

struct NoDispatch;
impl Dispatcher<()> for NoDispatch {
    async fn resolve<'a>(
        &'a self,
        _: &'a (),
        _: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Result<ResolvedValue<()>, ResolverError> {
        panic!("invalid requests must fail before application dispatch");
    }
}

#[tokio::test]
async fn executable_validation_clauses_precede_dispatch() {
    let schema = Schema::parse_and_validate(SCHEMA, "schema.graphql").unwrap();
    for query in [
        "{ value value }",
        "{ a: value(x: 1) b: value(x: 2) }",
        "query Q($x: Int = 1) { needs(x: $x) }",
        "query Q($x: Int) { value(required: $x) }",
        "{ value @tag(x: 1) @tag(x: 2) }",
        "{ node { ... on User { id } } search { ... on User { id } ... on Other { value } } }",
        "query Q($x: Int) { ...F } fragment F on Query { value(x: $x) }",
    ] {
        ExecutableDocument::parse_and_validate(&schema, query, "request.graphql").unwrap();
    }
    let invalid = [
        (
            "5.1.1 executable definitions",
            "type Extra { value: Int } { value }",
        ),
        (
            "5.2.1.1 unique operation names",
            "query Q { value } query Q { value }",
        ),
        (
            "5.2.2.1 lone anonymous operation",
            "{ value } query Q { value }",
        ),
        ("5.3.1 defined fields", "{ Value }"),
        ("5.3.1 union field selection", "{ search { id } }"),
        ("5.3.2 field merging", "{ a: value a: needs(x: 1) }"),
        ("5.3.2 argument merging", "{ value(x: 1) value(x: 2) }"),
        ("5.3.3 leaf selections", "{ value {} }"),
        ("5.3.3 composite selections", "{ user }"),
        ("5.4.1 known arguments", "{ value(unknown: 1) }"),
        ("5.4.2 unique arguments", "{ value(x: 1, x: 2) }"),
        ("5.4.3 required arguments", "{ needs }"),
        ("5.4.3 non-null arguments", "{ needs(x: null) }"),
        (
            "5.5.1.1 unique fragments",
            "{ ...F } fragment F on Query { value } fragment F on Query { value }",
        ),
        (
            "5.5.1.2 known fragment type",
            "{ ...F } fragment F on Missing { value }",
        ),
        (
            "5.5.1.3 composite fragment type",
            "{ ...F } fragment F on Int { __typename }",
        ),
        (
            "5.5.1.4 used fragments",
            "{ value } fragment F on Query { value }",
        ),
        ("5.5.2.1 defined spreads", "{ ...Missing }"),
        (
            "5.5.2.2 no fragment cycles",
            "{ ...F } fragment F on Query { ...G } fragment G on Query { ...F }",
        ),
        (
            "5.5.2.3 possible spreads",
            "{ user { ... on Other { value } } }",
        ),
        ("5.6.1 literal type", "{ value(x: \"1\") }"),
        ("5.6.1 enum literal kind", "{ value(status: \"ACTIVE\") }"),
        ("5.6.1 enum membership", "{ value(status: MISSING) }"),
        (
            "5.6.2 known input fields",
            "{ value(filter: {required: 1, extra: 1}) }",
        ),
        (
            "5.6.3 unique input fields",
            "{ value(filter: {required: 1, required: 2}) }",
        ),
        ("5.6.4 required input fields", "{ value(filter: {}) }"),
        (
            "5.6.4 non-null input fields",
            "{ value(filter: {required: null}) }",
        ),
        ("5.7.1 defined directives", "{ value @missing }"),
        (
            "5.7.2 directive location",
            "query Q @skip(if: true) { value }",
        ),
        (
            "5.7.3 unique directives",
            "{ value @skip(if: true) @skip(if: false) }",
        ),
        ("5.4 directive argument name", "{ value @tag(other: 1) }"),
        ("5.4 directive required argument", "{ value @tag }"),
        (
            "5.8.1 unique variables",
            "query Q($x: Int, $x: Int) { value(x: $x) }",
        ),
        (
            "5.8.2 input variables",
            "query Q($x: Query) { value(x: $x) }",
        ),
        (
            "5.8.3 defined variables in fragments",
            "{ ...F } fragment F on Query { value(x: $x) }",
        ),
        ("5.8.4 used variables", "query Q($x: Int) { value }"),
        (
            "5.8.5 compatible variables",
            "query Q($x: String) { value(x: $x) }",
        ),
        (
            "5.8.5 nullable required variable",
            "query Q($x: Int) { needs(x: $x) }",
        ),
        (
            "5.8.5 list compatibility",
            "query Q($x: [Int]) { value(x: $x) }",
        ),
        (
            "5.6.1 variable default type",
            "query Q($x: Int = \"bad\") { value(x: $x) }",
        ),
    ];
    for (clause, query) in invalid {
        assert!(
            ExecutableDocument::parse_and_validate(&schema, query, "request.graphql").is_err(),
            "{clause}: {query}"
        );
        let response = serde_json::to_value(
            necrassrs::execute(&schema, &Request::new(query), &NoDispatch, &()).await,
        )
        .unwrap();
        assert!(response.get("data").is_none(), "{clause}: {response}");
        let errors = response["errors"].as_array().expect(clause);
        assert!(!errors.is_empty(), "{clause}");
        assert!(
            errors.iter().any(|error| error["locations"]
                .as_array()
                .is_some_and(|locations| !locations.is_empty())),
            "{clause}: {response}"
        );
    }
}
