use std::{future::Future, pin::Pin, time::Duration};

use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, JsonValue, Request, ResolvedObject, ResolvedValue,
    ResolverError, Schema, Valid,
};

use crate::workload::{DELAY_MILLISECONDS, SDL};

pub struct Fixture {
    schema: Valid<Schema>,
    dispatcher: BenchmarkDispatcher,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            schema: Schema::parse_and_validate(SDL, "benchmark.graphql").unwrap(),
            dispatcher: BenchmarkDispatcher,
        }
    }

    pub async fn execute(&self, request: &Request) -> necrassrs::Response {
        necrassrs::execute(&self.schema, request, &self.dispatcher, &()).await
    }
}

struct BenchmarkDispatcher;

enum BenchmarkObject {
    Nested,
    Item(i32),
    Chain,
}

impl Dispatcher<()> for BenchmarkDispatcher {
    async fn resolve<'a>(
        &'a self,
        _context: &'a (),
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
    ) -> Result<ResolvedValue, ResolverError> {
        match coordinate.field {
            "immediate" => Ok(ResolvedValue::Json(argument(arguments, "value")?.into())),
            "delayed" => {
                delay().await;
                Ok(ResolvedValue::Json(argument(arguments, "value")?.into()))
            }
            "nested" => Ok(ResolvedValue::Object(Box::new(BenchmarkObject::Nested))),
            "items" => Ok(ResolvedValue::List(
                (0..argument(arguments, "count")?)
                    .map(|index| ResolvedValue::Object(Box::new(BenchmarkObject::Item(index))))
                    .collect(),
            )),
            "chain" => Ok(ResolvedValue::Object(Box::new(BenchmarkObject::Chain))),
            field => Err(ResolverError::new(format!("Unknown Query field '{field}'"))),
        }
    }
}

impl ResolvedObject<()> for BenchmarkObject {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Nested => "Nested",
            Self::Item(_) => "Item",
            Self::Chain => "Chain",
        }
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a (),
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedValue, ResolverError>> + Send + 'a>> {
        Box::pin(async move {
            match (self, coordinate.field) {
                (Self::Nested, "delayed") => {
                    delay().await;
                    Ok(ResolvedValue::Json(argument(arguments, "value")?.into()))
                }
                (Self::Item(index), "value") => {
                    delay().await;
                    Ok(ResolvedValue::Json(JsonValue::from(*index)))
                }
                (Self::Chain, "next") => {
                    delay().await;
                    Ok(ResolvedValue::Object(Box::new(Self::Chain)))
                }
                (Self::Chain, "value") => {
                    delay().await;
                    Ok(ResolvedValue::Json(JsonValue::from(1)))
                }
                _ => Err(ResolverError::new(format!(
                    "Unknown {} field '{}'",
                    self.type_name(),
                    coordinate.field
                ))),
            }
        })
    }
}

fn argument(arguments: &JsonMap, name: &str) -> Result<i32, ResolverError> {
    arguments
        .get(name)
        .and_then(JsonValue::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| ResolverError::new(format!("Invalid '{name}' benchmark argument")))
}

async fn delay() {
    tokio::time::sleep(Duration::from_millis(DELAY_MILLISECONDS)).await;
}
