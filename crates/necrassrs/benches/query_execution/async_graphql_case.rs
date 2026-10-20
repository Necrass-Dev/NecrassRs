use std::time::Duration;

use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema};

use crate::workload::DELAY_MILLISECONDS;

type BenchmarkSchema = Schema<Query, EmptyMutation, EmptySubscription>;

pub struct Fixture {
    schema: BenchmarkSchema,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            schema: Schema::new(Query, EmptyMutation, EmptySubscription),
        }
    }

    pub async fn execute(&self, query: &str) -> async_graphql::Response {
        self.schema.execute(query).await
    }
}

struct Query;

#[Object]
impl Query {
    async fn immediate(&self, value: i32) -> i32 {
        value
    }

    async fn delayed(&self, value: i32) -> i32 {
        delay().await;
        value
    }

    async fn nested(&self) -> Nested {
        Nested
    }

    async fn items(&self, count: i32) -> Vec<Item> {
        (0..count).map(Item).collect()
    }

    async fn chain(&self) -> Chain {
        Chain
    }
}

struct Nested;

#[Object]
impl Nested {
    async fn delayed(&self, value: i32) -> i32 {
        delay().await;
        value
    }
}

struct Item(i32);

#[Object]
impl Item {
    async fn value(&self) -> i32 {
        delay().await;
        self.0
    }
}

struct Chain;

#[Object]
impl Chain {
    async fn next(&self) -> Chain {
        delay().await;
        Chain
    }

    async fn value(&self) -> i32 {
        delay().await;
        1
    }
}

async fn delay() {
    tokio::time::sleep(Duration::from_millis(DELAY_MILLISECONDS)).await;
}
