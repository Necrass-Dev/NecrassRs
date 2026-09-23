#[allow(non_camel_case_types)]
pub struct Query;

#[allow(non_snake_case)]
impl<C: ::core::marker::Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {
    async fn r#hello(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::r#hello::Args,
    ) -> ::core::result::Result<::std::string::String, ::necrassrs::ResolverError> {
        ::core::unimplemented!()
    }
}
