import schema from "../../../crates/necrassrs-cli/templates/shared/schema.graphql?raw";
import resolver from "../../../crates/necrassrs-cli/templates/shared/resolvers.rs?raw";

export const initialSchema = schema.trim();
export const initialResolver = resolver.trim();
export const evolvedSchema = initialSchema.replace(/\n}$/, "\n    version: String!\n}");
export const evolvedResolver = initialResolver.replace(
  /\n}$/,
  `

    async fn version(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::version::Args,
    ) -> Result<String, necrassrs::ResolverError> {
        unimplemented!()
    }
}`,
);
