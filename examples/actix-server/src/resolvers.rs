use necrassrs::ResolverError;

#[allow(non_camel_case_types)]
pub struct Query;

#[allow(non_snake_case)]
impl<C: ::core::marker::Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {
    async fn r#hello(
        &self,
        _context: &C,
        args: crate::generated::types::Query::r#hello::Args,
    ) -> ::core::result::Result<::std::string::String, ::necrassrs::ResolverError> {
        if ["Sheri", "Margot"].contains(&args.name.as_str()) {
            Ok(format!("Hello, {}", args.name))
        } else {
            Err(
                ResolverError::new(format!("User \"{}\" was not found.", args.name))
                    .with_extension("code", "USER_NOT_FOUND"),
            )
        }
    }
}
