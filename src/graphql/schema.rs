use async_graphql::{EmptySubscription, Object, Schema};

use crate::app::App;

pub type AppSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// Health check — returns "ok" when the server is running.
    async fn health(&self) -> &str {
        "ok"
    }
}

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    /// Placeholder — real mutations will be added as features land.
    async fn _placeholder(&self) -> bool {
        false
    }
}

pub fn build_schema(app: App) -> AppSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(app)
        .finish()
}
