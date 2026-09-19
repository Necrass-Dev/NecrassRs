use apollo_compiler::response::{JsonMap, JsonValue};

pub struct ResolverError {
    message: String,
    extensions: Option<JsonMap>,
}

impl ResolverError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            extensions: None,
        }
    }

    pub fn with_extension(mut self, key: &str, value: impl Into<JsonValue>) -> Self {
        self.extensions
            .get_or_insert_default()
            .insert(key, value.into());

        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn extensions(&self) -> Option<&JsonMap> {
        self.extensions.as_ref()
    }
}
