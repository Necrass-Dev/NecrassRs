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
        let witches = ["Sheri", "Margot"];
        if witches.contains(&args.name.as_str()) {
            Ok(format!("Hello, {}", args.name))
        } else {
            Err(
                ResolverError::new(format!("User \"{}\" was not found.", args.name))
                    .with_extension("code", "USER_NOT_FOUND"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::{fields, types::Query::hello::Args};
    use necrassrs::Resolver;

    #[tokio::test]
    async fn hello_matches_names_and_reports_unknown_users() {
        let name = |name: &str| Args { name: name.into() };
        assert_eq!(
            <Query as Resolver<fields::Query::hello, ()>>::resolve(&Query, &(), name("Sheri"),)
                .await
                .ok()
                .as_deref(),
            Some("Hello, Sheri"),
        );

        let error =
            <Query as Resolver<fields::Query::hello, ()>>::resolve(&Query, &(), name("Unknown"))
                .await
                .err()
                .unwrap();
        assert_eq!(error.message(), "User \"Unknown\" was not found.");
        assert_eq!(
            error
                .extensions()
                .and_then(|extensions| extensions.get("code"))
                .and_then(necrassrs::JsonValue::as_str),
            Some("USER_NOT_FOUND"),
        );
    }
}
