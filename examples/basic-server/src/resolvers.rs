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
    use crate::generated::{resolvers::QueryResolver, types::Query::hello::Args};

    #[tokio::test]
    async fn hello_matches_names_and_reports_unknown_users() {
        let name = |name: &str| Args { name: name.into() };
        assert_eq!(
            Query.hello(&(), name("Sheri")).await.ok().as_deref(),
            Some("Hello, Sheri"),
        );

        let error = Query.hello(&(), name("Unknown")).await.err().unwrap();
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
