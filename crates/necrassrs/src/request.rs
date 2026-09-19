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
