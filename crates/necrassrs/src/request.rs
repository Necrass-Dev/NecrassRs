use apollo_compiler::{ExecutableDocument, response::GraphQLError, validation::Valid};

pub struct Request {
    query: String,
}

impl Request {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
        }
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the execution entry point once it is added"
    )
)]
pub(crate) fn prepare_request(
    schema: &Valid<apollo_compiler::Schema>,
    request: &Request,
) -> Result<Valid<ExecutableDocument>, Vec<GraphQLError>> {
    ExecutableDocument::parse_and_validate(schema, request.query.as_str(), "request.graphql")
        .map_err(|error| {
            error
                .errors
                .iter()
                .map(|diagnostic| diagnostic.to_json())
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apollo_compiler::Schema;

    #[test]
    fn syntax_error_is_rejected_during_request_preparation() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new("query {");

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert!(!errors.is_empty());
        assert!(!errors[0].locations.is_empty());
    }
}
