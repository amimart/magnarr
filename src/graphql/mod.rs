mod scalars;
pub mod schema;
mod types;

use std::sync::Arc;
use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;

use crate::app::service::DownloadService;
use crate::graphql::schema::{build_schema, AppSchema};

pub struct GraphqlServer {
    schema: AppSchema,
}

impl GraphqlServer {
    pub fn new(app: Arc<dyn DownloadService>) -> Self {
        Self {
            schema: build_schema(app),
        }
    }

    pub fn axum_router(&self) -> Router {
        Router::new()
            .route("/graphql", post(graphql_handler))
            .route("/graphql", get(graphql_playground))
            .with_state(self.schema.clone())
    }
}

async fn graphql_handler(State(schema): State<AppSchema>, req: GraphQLRequest) -> GraphQLResponse {
    schema.execute(req.into_inner()).await.into()
}

async fn graphql_playground() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}
