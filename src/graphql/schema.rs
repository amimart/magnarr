use crate::app::download::DownloadCursor;
use crate::graphql::scalars::MagnetUri;
use crate::graphql::types::{Download, DownloadStatus, SortOrder};
use crate::graphql::GraphqlContext;
use async_graphql::connection::{Connection, Edge, EmptyFields};
use async_graphql::{Context, EmptySubscription, Error, Object, Schema};
use base64::Engine;
use chrono::{DateTime, Utc};

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
        status: Option<DownloadStatus>,
        from: Option<DateTime<Utc>>,
        after: Option<String>,
        #[graphql(default = 50)] first: i32,
        order: Option<SortOrder>,
    ) -> async_graphql::Result<Connection<String, Download, EmptyFields, EmptyFields>> {
        let ctx = ctx.data::<GraphqlContext>()?;
        let after = after.as_deref().map(decode_downloads_cursor).transpose()?;
        let limit =
            usize::try_from(first).map_err(|_| Error::new("`first` must be non-negative"))?;
        if limit > ctx.max_page_size {
            return Err(Error::new(format!(
                "`first` cannot be greater than {}",
                ctx.max_page_size
            )));
        }
        let has_previous_page = after.is_some();

        let iter = ctx.app.downloads(
            status.map(Into::into),
            from,
            after,
            order.unwrap_or(SortOrder::Desc).into(),
        )?;

        let mut edges = iter
            .take(limit + 1)
            .map(|r| r.map(|e| Edge::new(encode_downloads_cursor(e.key.into()), e.record.into())))
            .collect::<Result<Vec<_>, _>>()?;

        let has_next_page = edges.len() > limit;
        edges.truncate(limit);

        let mut connection = Connection::new(has_previous_page, has_next_page);
        connection.edges.extend(edges);
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
        let ctx = ctx.data::<GraphqlContext>()?;
        let dl = ctx.app.download(magnet.0, target_dir).await?;
        Ok(dl.into())
    }
}

pub fn build_schema(ctx: GraphqlContext) -> AppSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(ctx)
        .finish()
}

/// Builds the schema without runtime data, used for SDL export.
pub fn build_schema_sdl() -> String {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .finish()
        .sdl()
}

fn encode_downloads_cursor(cursor: DownloadCursor) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cursor)
}

fn decode_downloads_cursor(cursor: &str) -> async_graphql::Result<DownloadCursor> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| Error::new("invalid downloads cursor"))?;
    Ok(DownloadCursor::new(raw))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;
    use crate::app::download::SortOrder as AppSortOrder;
    use crate::app::error::AppError;
    use crate::app::service::{DownloadIter, DownloadService};
    use crate::types::DownloadStatus as DomainDownloadStatus;
    use crate::types::{Download as DomainDownload, Magnet};
    use async_graphql::Request;
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use collette::iter::Entry;
    use collette::Cursor;

    const MAGNETS: [&str; 3] = [
        "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=first",
        "magnet:?xt=urn:btih:FEDCBA0987654321FEDCBA0987654321FEDCBA09&dn=second",
        "magnet:?xt=urn:btih:1111111111111111111111111111111111111111&dn=third",
    ];

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DownloadsCall {
        status: Option<crate::types::DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: AppSortOrder,
    }

    struct MockDownloadService {
        downloads: Vec<DomainDownload>,
        last_downloads_call: Mutex<Option<DownloadsCall>>,
    }

    #[async_trait]
    impl DownloadService for MockDownloadService {
        async fn download(
            &self,
            _magnet: Magnet,
            _target_dir: String,
        ) -> Result<DomainDownload, AppError> {
            unimplemented!()
        }

        fn downloads(
            &self,
            status: Option<crate::types::DownloadStatus>,
            from: Option<chrono::DateTime<chrono::Utc>>,
            after: Option<DownloadCursor>,
            order: AppSortOrder,
        ) -> Result<DownloadIter<'_>, AppError> {
            *self.last_downloads_call.lock().unwrap() = Some(DownloadsCall {
                status,
                from,
                after: after.clone(),
                order,
            });

            let mut downloads = self.downloads.clone();
            downloads.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.info_hash.cmp(&right.info_hash))
            });
            if matches!(order, AppSortOrder::Desc) {
                downloads.reverse();
            }

            let mut entries = downloads
                .into_iter()
                .filter(move |download| {
                    if status.is_some_and(|status| download.status != status) {
                        return false;
                    }

                    if let Some(from) = from {
                        let key = (download.created_at, download.info_hash.as_str());
                        let from_key = (from, "");
                        return match order {
                            AppSortOrder::Asc => key >= from_key,
                            AppSortOrder::Desc => key <= from_key,
                        };
                    }

                    true
                })
                .map(|record| {
                    let key = Cursor::from_key((
                        record.created_at.timestamp_micros(),
                        record.info_hash.as_str(),
                    ));
                    Entry { record, key }
                })
                .collect::<Vec<_>>();

            if let Some(after) = after {
                let cursor = Cursor::from(after);
                if let Some(position) = entries.iter().position(|entry| entry.key == cursor) {
                    entries.drain(..=position);
                }
            }

            Ok(Box::new(entries.into_iter().map(Ok)))
        }
    }

    fn new_mock_service() -> Arc<MockDownloadService> {
        let mut downloads = Vec::new();
        for (index, magnet) in MAGNETS.iter().enumerate() {
            let magnet: Magnet = magnet.parse().unwrap();
            let mut download = DomainDownload::new(magnet, "/downloads".to_owned());
            let ts = Utc.timestamp_opt((index as i64 + 1) * 10, 0).unwrap();
            download.created_at = ts;
            download.updated_at = ts;
            download.status = match index {
                0 => DomainDownloadStatus::Queued,
                1 => DomainDownloadStatus::Submitted,
                _ => DomainDownloadStatus::Submitted,
            };
            downloads.push(download);
        }

        Arc::new(MockDownloadService {
            downloads,
            last_downloads_call: Mutex::new(None),
        })
    }

    fn build_test_schema(service: Arc<dyn DownloadService>, max_page_size: usize) -> AppSchema {
        build_schema(GraphqlContext {
            app: service,
            max_page_size,
        })
    }

    #[tokio::test]
    async fn downloads_query_uses_cursor_for_next_page() {
        let service = new_mock_service();
        let schema = build_test_schema(service.clone(), 100);

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
        assert_eq!(
            *service.last_downloads_call.lock().unwrap(),
            Some(DownloadsCall {
                status: None,
                from: None,
                after: Some(DownloadCursor::from(Cursor::from_key((
                    Utc.timestamp_opt(20, 0).unwrap().timestamp_micros(),
                    "FEDCBA0987654321FEDCBA0987654321FEDCBA09",
                )))),
                order: AppSortOrder::Desc,
            })
        );
    }

    #[tokio::test]
    async fn downloads_query_forwards_filters_and_sorting() {
        let service = new_mock_service();
        let schema = build_test_schema(service.clone(), 100);

        let response = schema
            .execute(Request::new(
                r#"{
                    downloads(
                        status: SUBMITTED
                        from: "1970-01-01T00:00:20Z"
                        order: ASC
                        first: 5
                    ) {
                        edges { node { infoHash status } }
                        pageInfo { hasNextPage }
                    }
                }"#,
            ))
            .await;

        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let data = response.data.into_json().unwrap();
        let edges = data["downloads"]["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2);
        assert_eq!(
            edges[0]["node"]["infoHash"].as_str().unwrap(),
            "FEDCBA0987654321FEDCBA0987654321FEDCBA09"
        );
        assert_eq!(edges[0]["node"]["status"].as_str().unwrap(), "SUBMITTED");
        assert_eq!(
            edges[1]["node"]["infoHash"].as_str().unwrap(),
            "1111111111111111111111111111111111111111"
        );
        assert_eq!(edges[1]["node"]["status"].as_str().unwrap(), "SUBMITTED");
        assert!(!data["downloads"]["pageInfo"]["hasNextPage"]
            .as_bool()
            .unwrap());
        assert_eq!(
            *service.last_downloads_call.lock().unwrap(),
            Some(DownloadsCall {
                status: Some(DomainDownloadStatus::Submitted),
                from: Some(Utc.timestamp_opt(20, 0).unwrap()),
                after: None,
                order: AppSortOrder::Asc,
            })
        );
    }

    #[tokio::test]
    async fn downloads_query_rejects_page_size_over_maximum() {
        let schema = build_test_schema(new_mock_service(), 100);

        let response = schema
            .execute(Request::new(
                "{ downloads(first: 101) { edges { node { infoHash } } } }",
            ))
            .await;

        assert_eq!(response.errors.len(), 1);
        assert_eq!(
            response.errors[0].message,
            "`first` cannot be greater than 100"
        );
    }
}
