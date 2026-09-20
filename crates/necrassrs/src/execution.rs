use apollo_compiler::{
    Name,
    collections::IndexMap,
    executable::{Field, Selection},
};

use crate::request::PreparedRequest;

fn collect_fields(prepared: &PreparedRequest) -> IndexMap<Name, Vec<&Field>> {
    prepared
        .operation
        .selection_set
        .selections
        .iter()
        .filter_map(|selection| match selection {
            Selection::Field(field) => Some(field.as_ref()),
            _ => None,
        })
        .fold(IndexMap::default(), |mut fields, field| {
            fields
                .entry(field.response_key().clone())
                .or_default()
                .push(field);
            fields
        })
}

#[cfg(test)]
mod tests {
    use crate::{Request, request::prepare_request};
    use apollo_compiler::{Schema, response::serde_json_bytes::json};

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

    #[test]
    fn literal_argument_is_coerced_for_resolver_dispatch() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new(
            r#"
                query {
                    hello(name: "Sheri")
                }
            "#,
        );

        let prepared = prepare_request(&schema, &request).unwrap();
        let fields = super::collect_fields(&prepared);
        let field = fields.get("hello").unwrap()[0];

        let arguments = super::coerce_argument_values(&prepared, field).unwrap();

        assert_eq!(arguments.get("name"), Some(&json!("Sheri")));
    }
}
