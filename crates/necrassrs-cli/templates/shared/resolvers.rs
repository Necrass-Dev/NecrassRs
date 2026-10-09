pub struct Query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {}

impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::hello, C> for Query {
    type Output = String;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::Query::hello::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        Ok(format!("Hello, {}", args.name))
    }
}
