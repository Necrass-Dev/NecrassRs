pub struct Token(pub String);

impl necrassrs::Scalar for Token {
    fn parse_literal(value: &necrassrs::ScalarLiteral) -> Result<Self, necrassrs::ScalarError> {
        match value {
            necrassrs::ScalarLiteral::String(value) if value == "allowed" => {
                Ok(Self(value.clone()))
            }
            _ => Err(necrassrs::ScalarError::new(
                "Expected the string literal allowed",
            )),
        }
    }

    fn parse_value(value: &necrassrs::JsonValue) -> Result<Self, necrassrs::ScalarError> {
        match value.as_str() {
            Some("allowed") => Ok(Self("allowed".into())),
            _ => Err(necrassrs::ScalarError::new(
                "Expected the variable value allowed",
            )),
        }
    }

    fn serialize(&self) -> Result<necrassrs::JsonValue, necrassrs::ScalarError> {
        Ok(self.0.clone().into())
    }
}
