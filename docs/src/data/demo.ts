import schema from "../../../crates/necrassrs-cli/templates/shared/schema.graphql?raw";
import resolver from "../../../crates/necrassrs-cli/templates/shared/resolvers.rs?raw";
import versionResolver from "./version-resolver.rs?raw";

export const initialSchema = schema.trim();
export const initialResolver = resolver.trim();
export const evolvedSchema = initialSchema.replace(/\n}$/, "\n    version: String!\n}");
export const evolvedResolver = `${initialResolver}\n\n${versionResolver.trim()}`;
