use necrassrs::ResolverError;

#[allow(non_camel_case_types)]
pub struct Query;

#[allow(non_snake_case)]
impl<C: ::core::marker::Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}

impl<C: ::core::marker::Sync> ::necrassrs::Resolver<crate::generated::fields::Query::r#hello, C>
    for self::Query
{
    type Output = ::std::string::String;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::Query::r#hello::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
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
