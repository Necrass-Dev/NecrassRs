#[cfg(test)]
mod test {
    use apollo_compiler::Schema;

    #[test]
    fn generates_required_string_argument_struct() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");

        let generated = super::generate(&schema).expect("generation must succeed");
        let normalized: String = generated.split_whitespace().collect();

        assert!(
            normalized.contains("pubmodtypes{pubstructQueryHelloArgs{pubname:String,"),
            "expected a public argument struct in the types module, got:\n{generated}"
        );
    }
}
