use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use reqwest::{Client, Error, Response, StatusCode};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum AuthenticatedClientError {
    #[error("authentication failed: {0}")]
    AuthFailed(StatusCode),
    #[error(transparent)]
    ClientError(#[from] Error),
}

pub struct AuthenticatedClient {
    login_fn: Arc<LoginFn>,
    base_client: Client,
    session: RwLock<Session>,
}

pub type LoginFn = dyn Fn(Client) -> LoginFuture + Send + Sync;
pub type LoginFuture = Pin<Box<dyn Future<Output = Result<Client, AuthenticatedClientError>> + Send>>;

#[derive(Clone)]
struct Session {
    id: u64,
    client: Option<Client>,
}

impl AuthenticatedClient {
    pub fn new(base_client: Client, login_fn: Arc<LoginFn>) -> Self {
        Self {
            login_fn,
            base_client,
            session: RwLock::new(Session { id: 0, client: None }),
        }
    }

    async fn session(&self) -> Result<(Client, u64), AuthenticatedClientError> {
        {
            let session = self.session.read().await;
            if let Some(client) = &session.client {
                return Ok((client.clone(), session.id));
            }
        }

        let mut session = self.session.write().await;

        let auth_client = self.login_fn.clone()(self.base_client.clone()).await?;
        session.id += 1;
        session.client = Some(auth_client.clone());

        Ok((auth_client, session.id))
    }

    async fn renew_session(&self, old_id: u64) -> Result<(Client, u64), AuthenticatedClientError> {
        let mut session = self.session.write().await;
        if session.id != old_id {
            if let Some(client) = &session.client {
                return Ok((client.clone(), session.id));
            }
        }

        let auth_client = self.login_fn.clone()(self.base_client.clone()).await?;
        session.id += 1;
        session.client = Some(auth_client.clone());

        Ok((auth_client, session.id))
    }

    pub async fn with_auth<F, Fut>(&self, f: F) -> Result<Response, AuthenticatedClientError>
    where
        F: Fn(Client) -> Fut,
        Fut: Future<Output = Result<Response, Error>>,
    {
        let (http, id) = self.session().await?;
        let resp = f(http).await?;
        if !Self::is_auth_error(&resp) {
            return Ok(resp);
        }

        let (http, _) = self.renew_session(id).await?;
        Self::ensure_authenticated(f(http).await?)
    }

    pub fn is_auth_error(resp: &Response) -> bool {
        matches!(resp.status(), StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED)
    }

    pub fn ensure_authenticated(resp: Response) -> Result<Response, AuthenticatedClientError> {
        if Self::is_auth_error(&resp) {
            Err(AuthenticatedClientError::AuthFailed(resp.status()))
        }else {
            Ok(resp)
        }
    }
}