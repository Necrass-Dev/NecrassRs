use apollo_compiler::{
    Node,
    ast::Value,
    parser::SourceSpan,
    response::{JsonMap, JsonValue},
};

pub(crate) struct InputCoercionError {
    pub(crate) message: String,
    pub(crate) location: Option<SourceSpan>,
}

impl InputCoercionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: None,
        }
    }

    fn at(message: impl Into<String>, location: Option<SourceSpan>) -> Self {
        Self {
            message: message.into(),
            location,
        }
    }
}

pub(crate) fn literal_to_json(value: &Node<Value>) -> Result<JsonValue, InputCoercionError> {
    match value.as_ref() {
        Value::Null => Ok(JsonValue::Null),
        Value::Enum(value) => Ok(value.as_str().into()),
        Value::String(value) => Ok(value.as_str().into()),
        Value::Boolean(value) => Ok((*value).into()),
        Value::Int(number) => number.as_str().parse().map(JsonValue::Number).map_err(|_| {
            InputCoercionError::at(
                "integer argument is outside the supported JSON range",
                value.location(),
            )
        }),
        Value::Float(number) => number.as_str().parse().map(JsonValue::Number).map_err(|_| {
            InputCoercionError::at(
                "float argument is outside the supported JSON range",
                value.location(),
            )
        }),
        Value::List(values) => values
            .iter()
            .map(literal_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Into::into),
        Value::Object(values) => values
            .iter()
            .map(|(name, value)| Ok((name.as_str().into(), literal_to_json(value)?)))
            .collect::<Result<JsonMap, _>>()
            .map(Into::into),
        Value::Variable(name) => Err(InputCoercionError::at(
            format!("unresolved variable '${name}'"),
            value.location(),
        )),
    }
}
