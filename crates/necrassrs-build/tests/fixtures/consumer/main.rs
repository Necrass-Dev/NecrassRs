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
    let _schema =
        necrassrs::Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let _dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
}
