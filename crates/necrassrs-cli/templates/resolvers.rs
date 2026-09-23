pub struct Query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {
    async fn hello(
        &self,
        _context: &C,
        args: crate::generated::types::Query::hello::Args,
    ) -> Result<String, necrassrs::ResolverError> {
        Ok(format!("Hello, {}", args.name))
    }
}
