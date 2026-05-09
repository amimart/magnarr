use async_graphql::{Context, EmptySubscription, Error, Object, Schema};

use crate::app::error::AppError;
use crate::app::App;
use crate::graphql::scalars::MagnetUri;
use crate::graphql::types::Download;

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
    /// Submits a new download: validates the magnet, persists it, and hands it
    /// off to the torrent client. Returns the created download record.
    async fn download(
        &self,
        ctx: &Context<'_>,
        magnet: MagnetUri,
        target_dir: String,
    ) -> Result<Download, Error> {
        let app = ctx.data::<App>()?;
        app.download(magnet.0, target_dir)
            .await
            .map(Download::from)
            .map_err(|e| match e {
                AppError::AlreadyExists => Error::new("download already exists"),
                AppError::TorrentClient(e) => {
                    Error::new(format!("torrent client error: {e}"))
                }
                AppError::Repository(e) => Error::new(format!("repository error: {e}")),
            })
    }
}

pub fn build_schema(app: App) -> AppSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(app)
        .finish()
}

/// Builds the schema without runtime data, used for SDL export.
pub fn build_schema_sdl() -> String {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .finish()
        .sdl()
}

