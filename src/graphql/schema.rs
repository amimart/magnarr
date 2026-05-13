use async_graphql::connection::{Connection, Edge, EmptyFields};
use async_graphql::{Context, EmptySubscription, Error, Object, Schema};
use base64::Engine;

use crate::app::download::{DownloadListOrder, DownloadListQuery, MAX_DOWNLOADS_PAGE_SIZE};
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

    async fn downloads(
        &self,
        ctx: &Context<'_>,
        after: Option<String>,
        first: Option<i32>,
    ) -> async_graphql::Result<Connection<String, Download, EmptyFields, EmptyFields>> {
        let app = ctx.data::<App>()?;
        let after = after.as_deref().map(decode_downloads_cursor).transpose()?;
        let first = first
            .map(|limit| {
                usize::try_from(limit).map_err(|_| Error::new("`first` must be non-negative"))
            })
            .transpose()?;
        let limit = first
            .unwrap_or(app.downloads_page_size())
            .clamp(1, MAX_DOWNLOADS_PAGE_SIZE);
        let has_previous_page = after.is_some();
        let mut iter = app.downloads(DownloadListQuery {
            order: Some(DownloadListOrder::CreatedAtDesc),
            from_created_at: after.as_ref().map(|cursor| cursor.created_at),
            after_info_hash: after.as_ref().map(|cursor| cursor.info_hash.clone()),
            ..Default::default()
        })?;
        let downloads = iter.by_ref().take(limit + 1).collect::<Vec<_>>();
        let has_next_page = downloads.len() > limit;

        let mut connection = Connection::new(has_previous_page, has_next_page);
        connection
            .edges
            .extend(downloads.into_iter().take(limit).map(|download| {
                let cursor = encode_downloads_cursor(download.created_at, &download.info_hash);
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
        let app = ctx.data::<App>()?;
        let dl = app.download(magnet.0, target_dir).await?;
        Ok(dl.into())
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

fn encode_downloads_cursor(created_at: chrono::DateTime<chrono::Utc>, info_hash: &str) -> String {
    let raw = format!("{}:{info_hash}", created_at.timestamp_micros());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn decode_downloads_cursor(cursor: &str) -> async_graphql::Result<DecodedDownloadsCursor> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| Error::new("invalid downloads cursor"))?;
    let raw = String::from_utf8(raw).map_err(|_| Error::new("invalid downloads cursor"))?;
    let (created_at, info_hash) = raw
        .split_once(':')
        .ok_or_else(|| Error::new("invalid downloads cursor"))?;
    let created_at = created_at
        .parse::<i64>()
        .map_err(|_| Error::new("invalid downloads cursor"))?;
    let created_at = chrono::DateTime::from_timestamp_micros(created_at)
        .ok_or_else(|| Error::new("invalid downloads cursor"))?;

    Ok(DecodedDownloadsCursor {
        created_at,
        info_hash: info_hash.to_owned(),
    })
}

struct DecodedDownloadsCursor {
    created_at: chrono::DateTime<chrono::Utc>,
    info_hash: String,
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

    fn build_test_schema(default_page_size: usize) -> (AppSchema, tempfile::TempDir) {
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
            default_page_size,
        );

        (build_schema(app), dir)
    }

    #[tokio::test]
    async fn downloads_query_uses_default_page_size() {
        let (schema, _dir) = build_test_schema(2);

        let response = schema
            .execute(Request::new(
                "{ downloads { edges { node { infoHash } } pageInfo { hasNextPage endCursor } } }",
            ))
            .await;

        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let data = response.data.into_json().unwrap();
        let downloads = &data["downloads"];
        assert_eq!(downloads["edges"].as_array().unwrap().len(), 2);
        assert_eq!(
            downloads["edges"][0]["node"]["infoHash"].as_str().unwrap(),
            "1111111111111111111111111111111111111111"
        );
        assert!(downloads["pageInfo"]["hasNextPage"].as_bool().unwrap());
        assert!(downloads["pageInfo"]["endCursor"].as_str().is_some());
    }

    #[tokio::test]
    async fn downloads_query_uses_cursor_for_next_page() {
        let (schema, _dir) = build_test_schema(2);

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
