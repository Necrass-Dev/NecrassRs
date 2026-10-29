use generated::{fields, types};
use necrassrs::{Field, Resolver, ResolverError};
use serde_json::{Value, json};

mod resolvers {
    use necrassrs::{JsonValue, Scalar, ScalarError, ScalarLiteral};

    pub struct DateTime;

    impl Scalar for DateTime {
        fn parse_literal(_: &ScalarLiteral) -> Result<Self, ScalarError> {
            Err(ScalarError::new(
                "introspection must not coerce custom scalars",
            ))
        }

        fn parse_value(_: &JsonValue) -> Result<Self, ScalarError> {
            Err(ScalarError::new(
                "introspection must not coerce custom scalars",
            ))
        }

        fn serialize(&self) -> Result<JsonValue, ScalarError> {
            Err(ScalarError::new(
                "introspection must not serialize custom scalars",
            ))
        }
    }
}

struct Query;
struct User;

impl generated::resolvers::QueryResolver<()> for Query {}

impl Resolver<fields::Query::lookup, ()> for Query {
    type Output = Vec<types::Search<User>>;

    async fn resolve(
        &self,
        _: &(),
        _: <fields::Query::lookup as Field>::Args,
    ) -> Result<Self::Output, ResolverError> {
        panic!("introspection must not call the query resolver");
    }
}

impl Resolver<fields::User::id, ()> for User {
    type Output = necrassrs::Id;

    async fn resolve(
        &self,
        _: &(),
        _: <fields::User::id as Field>::Args,
    ) -> Result<Self::Output, ResolverError> {
        panic!("introspection must not call an object resolver");
    }
}

fn named<'a>(values: &'a Value, name: &str) -> &'a Value {
    values
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["name"] == name)
        .unwrap()
}

fn main() {
    let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "schema.graphql").unwrap();
    let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
    let execute = |query: &str, introspection| {
        let request = necrassrs::Request::new(query);
        let response = futures::executor::block_on(necrassrs::execute_with_options(
            &schema,
            &request,
            &dispatcher,
            &(),
            necrassrs::ExecutionOptions { introspection },
        ));
        let serialized = serde_json::to_string(&response).unwrap();
        serde_json::from_str::<Value>(&serialized).unwrap()
    };
    let query = r#"
        {
            __typename
            missing: __type(name: "Missing") { name }
            __schema {
                queryType { name }
                mutationType { name }
                subscriptionType { name }
                directives(includeDeprecated: true) {
                    name locations isRepeatable isDeprecated deprecationReason
                    args(includeDeprecated: true) { ...Input }
                }
                types {
                    kind name isOneOf specifiedByURL
                    interfaces { name }
                    possibleTypes { name }
                    fields(includeDeprecated: true) {
                        name isDeprecated deprecationReason type { ...TypeRef }
                        args(includeDeprecated: true) { ...Input }
                    }
                    inputFields(includeDeprecated: true) { ...Input }
                    enumValues(includeDeprecated: true) { name isDeprecated deprecationReason }
                }
            }
        }
        fragment Input on __InputValue {
            name defaultValue isDeprecated deprecationReason type { ...TypeRef }
        }
        fragment TypeRef on __Type {
            kind name ofType { kind name ofType { kind name ofType { kind name } } }
        }
    "#;
    let response = execute(query, true);
    assert!(response.get("errors").is_none(), "{response}");
    assert_eq!(response["data"]["__typename"], "Query");
    assert_eq!(response["data"]["missing"], Value::Null);
    let schema = &response["data"]["__schema"];
    assert_eq!(schema["queryType"], json!({"name": "Query"}));
    assert_eq!(schema["mutationType"], Value::Null);
    assert_eq!(schema["subscriptionType"], Value::Null);
    let types = &schema["types"];
    for (name, kind) in [
        ("Query", "OBJECT"),
        ("User", "OBJECT"),
        ("Node", "INTERFACE"),
        ("Search", "UNION"),
        ("Status", "ENUM"),
        ("Filter", "INPUT_OBJECT"),
        ("Choice", "INPUT_OBJECT"),
        ("DateTime", "SCALAR"),
        ("Int", "SCALAR"),
        ("Float", "SCALAR"),
        ("Boolean", "SCALAR"),
        ("String", "SCALAR"),
        ("ID", "SCALAR"),
    ] {
        assert_eq!(named(types, name)["kind"], kind, "{name}");
    }
    assert_eq!(named(types, "Choice")["isOneOf"], true);
    assert_eq!(named(types, "Filter")["isOneOf"], false);
    assert_eq!(named(types, "DateTime")["isOneOf"], Value::Null);
    assert_eq!(
        named(types, "DateTime")["specifiedByURL"],
        "https://example.com/date-time"
    );
    assert_eq!(
        named(types, "User")["interfaces"],
        json!([{"name": "Node"}])
    );
    for name in ["Node", "Search"] {
        assert_eq!(
            named(types, name)["possibleTypes"],
            json!([{"name": "User"}])
        );
    }
    let filter = &named(types, "Filter")["inputFields"];
    for (name, default) in [
        ("limit", "10"),
        ("status", "ACTIVE"),
        ("enabled", "true"),
        ("ratio", "1.5"),
        ("label", "\"visible\""),
    ] {
        assert_eq!(named(filter, name)["defaultValue"], default, "{name}");
    }
    assert_eq!(named(filter, "at")["defaultValue"], Value::Null);
    assert_eq!(named(filter, "old")["isDeprecated"], true);
    assert_eq!(named(filter, "old")["deprecationReason"], "Use limit");
    let legacy = named(&named(types, "Status")["enumValues"], "LEGACY");
    assert_eq!(legacy["isDeprecated"], true);
    assert_eq!(legacy["deprecationReason"], "Use ACTIVE");
    let lookup = named(&named(types, "Query")["fields"], "lookup");
    assert_eq!(lookup["isDeprecated"], true);
    assert_eq!(lookup["deprecationReason"], "Use search");
    assert_eq!(named(&lookup["args"], "limit")["defaultValue"], "3");
    assert_eq!(
        named(&lookup["args"], "old")["deprecationReason"],
        "Use limit"
    );
    assert_eq!(
        lookup["type"],
        json!({
            "kind": "NON_NULL", "name": null, "ofType": {
                "kind": "LIST", "name": null, "ofType": {
                    "kind": "NON_NULL", "name": null, "ofType": {"kind": "UNION", "name": "Search"}
                }
            }
        })
    );
    let tag = named(&schema["directives"], "tag");
    assert_eq!(tag["isRepeatable"], true);
    assert_eq!(tag["locations"], json!(["FIELD_DEFINITION"]));
    assert_eq!(named(&tag["args"], "label")["defaultValue"], "\"public\"");
    let old = named(&schema["directives"], "old");
    assert_eq!(old["isDeprecated"], true);
    assert_eq!(old["deprecationReason"], "Use tag");

    let filtered = execute(
        r#"{
        status: __type(name: "Status") { enumValues { name } }
        input: __type(name: "Filter") { inputFields { name } }
        query: __type(name: "Query") { fields { name } }
        arguments: __type(name: "Query") { fields(includeDeprecated: true) { args { name } } }
        __schema { directives { name } }
    }"#,
        true,
    );
    assert!(filtered.get("errors").is_none(), "{filtered}");
    assert_eq!(
        filtered["data"]["status"]["enumValues"],
        json!([{"name": "ACTIVE"}])
    );
    assert_eq!(filtered["data"]["query"]["fields"], json!([]));
    for values in [
        &filtered["data"]["input"]["inputFields"],
        &filtered["data"]["arguments"]["fields"][0]["args"],
        &filtered["data"]["__schema"]["directives"],
    ] {
        assert!(
            values
                .as_array()
                .unwrap()
                .iter()
                .all(|value| value["name"] != "old")
        );
    }
    let disabled = execute(query, false);
    assert!(disabled.get("data").is_none(), "{disabled}");
    assert!(!disabled["errors"].as_array().unwrap().is_empty());
    assert_eq!(
        execute("{ __typename }", false),
        json!({"data": {"__typename": "Query"}})
    );
}
