mod generated {
    include!(concat!(env!("OUT_DIR"), "/necrassrs.rs"));
}

struct Query;

impl generated::resolvers::QueryResolver<()> for Query {
    async fn hello<'a>(
        &'a self,
        _: &'a (),
        args: generated::types::Query::hello::Args,
    ) -> Result<String, necrassrs::ResolverError> {
        Ok(format!("Hello, {}", args.name))
    }
}

fn main() {
    let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);

    futures::executor::block_on(async {
        let first = necrassrs::Request::new(r#"{ hello(name: "Sheri") }"#);
        let second = necrassrs::Request::new(r#"{ hello(name: "Tachibana Sheri") }"#);
        let responses = [
            necrassrs::execute(&schema, &first, &dispatcher, &()).await,
            necrassrs::execute(&schema, &second, &dispatcher, &()).await,
        ];
        println!("{}", serde_json::to_string(&responses).unwrap());
    });
}
