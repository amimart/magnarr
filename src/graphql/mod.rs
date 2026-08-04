mod scalars;
pub mod schema;
mod types;

use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

use crate::app::service::DownloadService;
use crate::graphql::schema::{build_schema, AppSchema};

pub struct GraphqlServer<S>
where
    S: DownloadService + 'static,
{
    schema: AppSchema<S>,
}

pub struct GraphqlContext<S> {
    pub app: Arc<S>,
    pub max_page_size: usize,
}

impl<S> GraphqlServer<S>
where
    S: DownloadService + 'static,
{
    pub fn new(app: Arc<S>, max_page_size: usize) -> Self {
        Self {
            schema: build_schema(GraphqlContext { app, max_page_size }),
        }
    }

    pub fn axum_router(&self) -> Router {
        Router::new()
            .route("/graphql", post(graphql_handler))
            .route("/graphql", get(graphql_playground))
            .with_state(self.schema.clone())
    }
}

async fn graphql_handler<S>(
    State(schema): State<AppSchema<S>>,
    req: GraphQLRequest,
) -> GraphQLResponse
where
    S: DownloadService + 'static,
{
    schema.execute(req.into_inner()).await.into()
}

async fn graphql_playground() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}
