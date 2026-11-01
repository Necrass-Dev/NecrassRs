use lead_compiler::Schema;
use necrassrs::{Dispatcher, FieldCoordinate, JsonMap, Request, ResolvedValue, ResolverError};

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
async fn executable_validation_errors_precede_dispatch() {
    let schema = Schema::parse_and_validate(
        "type Query { value(required: Int!): Int }",
        "schema.graphql",
    )
    .unwrap();
    let response = serde_json::to_value(
        necrassrs::execute(&schema, &Request::new("{ value }"), &NoDispatch, &()).await,
    )
    .unwrap();

    assert!(response.get("data").is_none(), "{response}");
    let errors = response["errors"].as_array().unwrap();
    assert!(!errors.is_empty(), "{response}");
    assert!(
        errors.iter().any(|error| error["locations"]
            .as_array()
            .is_some_and(|locations| !locations.is_empty())),
        "{response}"
    );
}
