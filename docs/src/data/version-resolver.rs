impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::version, C> for Query {
    type Output = String;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::version::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        ::core::unimplemented!()
    }
}
