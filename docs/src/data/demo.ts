import schema from "../../../crates/necrassrs-cli/templates/shared/schema.graphql?raw";
import resolver from "../../../crates/necrassrs-cli/templates/shared/resolvers.rs?raw";

export const initialSchema = schema.trim();
export const initialResolver = resolver.trim();
export const evolvedSchema = initialSchema.replace(/\n}$/, "\n    version: String!\n}");
export const evolvedResolver = `${initialResolver}

impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Query::r#version, C> for Query {
    type Output = ::std::string::String;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Query::r#version::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        ::core::unimplemented!()
    }
}`;
