#[cfg(test)]
mod tests {
    use crate::{Request, request::prepare_request};
    use apollo_compiler::Schema;

    #[test]
    fn direct_root_field_is_collected_by_response_key() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new(
            r#"
                query {
                    greeting: hello(name: "Sheri")
                }
            "#,
        );

        let prepared = prepare_request(&schema, &request).unwrap();

        let fields = super::collect_fields(&prepared);
        let collected = fields.get("greeting").unwrap();

        assert_eq!(fields.len(), 1);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].name.as_str(), "hello");
    }
}
