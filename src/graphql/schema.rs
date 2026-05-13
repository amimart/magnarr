use std::sync::Arc;
use async_graphql::connection::{Connection, Edge, EmptyFields};
use async_graphql::{Context, EmptySubscription, Error, Object, Schema};
use base64::Engine;

use crate::app::download::{DownloadCursor, DownloadListOrder, MAX_DOWNLOADS_PAGE_SIZE};
use crate::app::App;
use crate::app::service::DownloadService;
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

    async fn downloads(
        &self,
        ctx: &Context<'_>,
        after: Option<String>,
        #[graphql(default = 50)] first: i32,
    ) -> async_graphql::Result<Connection<String, Download, EmptyFields, EmptyFields>> {
        let app = ctx.data::<Arc<dyn DownloadService>>()?;
        let after = after.as_deref().map(decode_downloads_cursor).transpose()?;
        let limit = usize::try_from(first).map_err(|_| Error::new("`first` must be non-negative"))?;
        if limit > MAX_DOWNLOADS_PAGE_SIZE {
            return Err(Error::new(format!(
                "`first` cannot be greater than {MAX_DOWNLOADS_PAGE_SIZE}"
            )));
        }
        let has_previous_page = after.is_some();
        let mut iter =
            app.downloads(None, None, after.clone(), DownloadListOrder::CreatedAtDesc)?;
        let downloads = iter
            .by_ref()
            .take(limit + 1)
            .collect::<Result<Vec<_>, _>>()?;
        let has_next_page = downloads.len() > limit;

        let mut connection = Connection::new(has_previous_page, has_next_page);
        connection
            .edges
            .extend(downloads.into_iter().take(limit).map(|download| {
                let cursor = encode_downloads_cursor(&DownloadCursor::from_download(&download));
                Edge::new(cursor, download.into())
            }));
        Ok(connection)
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
    ) -> async_graphql::Result<Download> {
        let app = ctx.data::<Arc<dyn DownloadService>>()?;
        let dl = app.download(magnet.0, target_dir).await?;
        Ok(dl.into())
    }
}

pub fn build_schema(app: Arc<dyn DownloadService>) -> AppSchema {
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

fn encode_downloads_cursor(cursor: &DownloadCursor) -> String {
    let raw = serde_json::to_vec(cursor).expect("download cursor should always serialize");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn decode_downloads_cursor(cursor: &str) -> async_graphql::Result<DownloadCursor> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| Error::new("invalid downloads cursor"))?;
    serde_json::from_slice(&raw).map_err(|_| Error::new("invalid downloads cursor"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_graphql::Request;
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::app::download::DownloadRepository;
    use crate::app::torrent::{TorrentClient, TorrentClientError};
    use crate::store::redb::RedbStore;
    use crate::types::{Download as DomainDownload, Magnet, TorrentStatus};

    const MAGNETS: [&str; 3] = [
        "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=first",
        "magnet:?xt=urn:btih:FEDCBA0987654321FEDCBA0987654321FEDCBA09&dn=second",
        "magnet:?xt=urn:btih:1111111111111111111111111111111111111111&dn=third",
    ];

    struct NoopTorrentClient;

    #[async_trait]
    impl TorrentClient for NoopTorrentClient {
        async fn download(&self, _magnet: &Magnet) -> Result<(), TorrentClientError> {
            Ok(())
        }

        async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
            Err(TorrentClientError::NotFound(info_hash.to_owned()))
        }
    }

    fn build_test_schema() -> (AppSchema, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = RedbStore::new(dir.path().join("test.redb").to_str().unwrap()).unwrap();

        for (index, magnet) in MAGNETS.iter().enumerate() {
            let magnet: Magnet = magnet.parse().unwrap();
            let mut download = DomainDownload::new(magnet, "/downloads".to_owned());
            let ts = Utc.timestamp_opt((index as i64 + 1) * 10, 0).unwrap();
            download.created_at = ts;
            download.updated_at = ts;
            store.create_download(&download).unwrap();
        }

        let app = App::new(
            Arc::new(store),
            Arc::new(NoopTorrentClient),
            Duration::from_secs(60),
            dir.path().join("downloads"),
        );

        (build_schema(Arc::new(app)), dir)
    }

    #[tokio::test]
    async fn downloads_query_uses_cursor_for_next_page() {
        let (schema, _dir) = build_test_schema();

        let first_response = schema
            .execute(Request::new(
                "{ downloads(first: 2) { edges { node { infoHash } } pageInfo { endCursor hasNextPage } } }",
            ))
            .await;
        assert!(
            first_response.errors.is_empty(),
            "{:?}",
            first_response.errors
        );
        let first_data = first_response.data.into_json().unwrap();
        let end_cursor = first_data["downloads"]["pageInfo"]["endCursor"]
            .as_str()
            .unwrap()
            .to_owned();

        let second_response = schema
            .execute(Request::new(format!(
                "{{ downloads(first: 2, after: \"{end_cursor}\") {{ edges {{ node {{ infoHash }} }} pageInfo {{ hasNextPage }} }} }}"
            )))
            .await;

        assert!(
            second_response.errors.is_empty(),
            "{:?}",
            second_response.errors
        );
        let second_data = second_response.data.into_json().unwrap();
        let edges = second_data["downloads"]["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges[0]["node"]["infoHash"].as_str().unwrap(),
            "ABCDEF1234567890ABCDEF1234567890ABCDEF12"
        );
        assert!(!second_data["downloads"]["pageInfo"]["hasNextPage"]
            .as_bool()
            .unwrap());
    }
}
