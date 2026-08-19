//! GitHub Projects v2 GraphQL client (read-only).
//!
//! Phase 2 of the roadmap pipeline (spec §3.7). Provides
//! `list_project_items()` and `get_project_item()` for the sync
//! engine (Phase 3) to consume. The HTTP path is exercised end-to-end
//! by Phase 9 real-API smoke tests; here we test the parser, the JWT
//! shape, the token cache, and the backoff policy directly.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use thiserror::Error;
use tokio::sync::Mutex;

// ---------- credentials -----------------------------------------------------

/// GitHub App credentials. Private key is PEM-encoded (PKCS#1 or PKCS#8).
#[derive(Clone)]
pub struct GitHubAppCreds {
    pub app_id: String,
    pub installation_id: String,
    pub private_key: String,
}

impl std::fmt::Debug for GitHubAppCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAppCreds")
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

// ---------- error taxonomy --------------------------------------------------

#[derive(Debug, Error)]
pub enum GitHubError {
    /// Authentication failed — bad App credentials, bad installation,
    /// expired JWT, etc. Not retryable.
    #[error("auth error: {0}")]
    Auth(String),
    /// GitHub returned 429 or a `RATE_LIMITED` GraphQL error.
    /// Retryable.
    #[error("rate limited (retry_after_seconds={retry_after_seconds:?})")]
    RateLimited { retry_after_seconds: Option<u32> },
    /// Network error, 5xx, or other intermittent failure. Retryable.
    #[error("transient error: {0}")]
    Transient(String),
    /// GraphQL response didn't match our types, missing `data`, or
    /// envelope parse failed. Not retryable — the schema needs an
    /// update first.
    #[error("schema mismatch: {0}")]
    Schema(String),
    /// Anything else (4xx that isn't auth, etc.). Not retryable.
    #[error("other: {0}")]
    Other(String),
}

// ---------- JWT minting -----------------------------------------------------

#[derive(Serialize)]
struct AppJwtClaims<'a> {
    iat: i64,
    exp: i64,
    iss: &'a str,
}

/// Mint a GitHub-App-flavoured JWT.
///
/// GitHub requires `iat <= now`, `exp <= now + 10 minutes`. We pin
/// `iat = now - 60s` (clock-skew tolerance) and `exp = now + 9 minutes`.
pub fn mint_app_jwt(creds: &GitHubAppCreds, now: DateTime<Utc>) -> Result<String, GitHubError> {
    let claims = AppJwtClaims {
        iat: now.timestamp() - 60,
        exp: now.timestamp() + 9 * 60,
        iss: &creds.app_id,
    };
    let key = EncodingKey::from_rsa_pem(creds.private_key.as_bytes())
        .map_err(|e| GitHubError::Auth(format!("PEM parse: {e}")))?;
    encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|e| GitHubError::Auth(format!("JWT encode: {e}")))
}

// ---------- installation-token cache ---------------------------------------

#[derive(Clone, Debug)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Caches a single installation token. Returns the cached value while
/// it still has at least 5 minutes of life left; otherwise calls the
/// provided fetcher.
pub struct TokenCache {
    cached: Mutex<Option<InstallationToken>>,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCache {
    pub fn new() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }

    pub async fn get_or_fetch<F, Fut>(
        &self,
        now: DateTime<Utc>,
        fetcher: F,
    ) -> Result<String, GitHubError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<InstallationToken, GitHubError>>,
    {
        {
            let cache = self.cached.lock().await;
            if let Some(tok) = &*cache {
                if tok.expires_at > now + Duration::minutes(5) {
                    return Ok(tok.token.clone());
                }
            }
        }
        let fresh = fetcher().await?;
        let token = fresh.token.clone();
        let mut cache = self.cached.lock().await;
        *cache = Some(fresh);
        Ok(token)
    }
}

// ---------- backoff policy --------------------------------------------------

pub const DEFAULT_BACKOFF_MS: &[u64] = &[0, 250, 1000];

/// Run `op` repeatedly per the supplied per-attempt delays. Retries on
/// `RateLimited` + `Transient`; surfaces `Auth`, `Schema`, `Other`
/// immediately.
pub async fn with_backoff<F, Fut, T>(delays_ms: &[u64], mut op: F) -> Result<T, GitHubError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, GitHubError>>,
{
    let mut last_err: Option<GitHubError> = None;
    for &delay_ms in delays_ms {
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if matches!(
                    e,
                    GitHubError::Auth(_) | GitHubError::Schema(_) | GitHubError::Other(_)
                ) {
                    return Err(e);
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| GitHubError::Other("with_backoff: empty delays".into())))
}

// ---------- response types --------------------------------------------------

/// A project item, GraphQL-shaped (not yet mapped to `RoadmapItem`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectItem {
    pub id: String,
    pub content: ProjectItemContent,
    pub custom_fields: HashMap<String, ProjectFieldValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectItemContent {
    Issue {
        title: String,
        body: String,
        url: String,
        /// Labels on the linked Issue — used by Phase 3 to extract
        /// channel/* and surface/* tags (see spec §3.3).
        labels: Vec<String>,
    },
    PullRequest {
        title: String,
        body: String,
        url: String,
        merged_at: Option<DateTime<Utc>>,
        labels: Vec<String>,
    },
    /// Draft issues have no labels (Projects v2 doesn't attach labels
    /// to drafts). Channels/surfaces are always empty.
    DraftIssue { title: String, body: String },
    /// Unknown content kind. Future-proofing: the GraphQL schema may
    /// add new content types and we don't want to hard-fail on
    /// unrecognised typenames.
    Other { typename: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectFieldValue {
    Text(String),
    Number(f64),
    SingleSelect {
        option_name: String,
        option_id: String,
    },
    Date(DateTime<Utc>),
    Iteration {
        title: String,
    },
}

// ---------- parsing ---------------------------------------------------------

#[derive(Deserialize)]
struct GqlEnvelope<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize, Debug)]
struct GqlError {
    message: String,
    #[serde(rename = "type")]
    ty: Option<String>,
}

pub fn parse_graphql_response<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, GitHubError> {
    let env: GqlEnvelope<T> = serde_json::from_slice(bytes)
        .map_err(|e| GitHubError::Schema(format!("envelope parse: {e}")))?;
    if let Some(errs) = env.errors {
        if !errs.is_empty() {
            let first = &errs[0];
            let ty = first.ty.as_deref().unwrap_or("");
            return Err(match ty {
                "RATE_LIMITED" => GitHubError::RateLimited {
                    retry_after_seconds: None,
                },
                "UNAUTHENTICATED" | "FORBIDDEN" => GitHubError::Auth(first.message.clone()),
                _ => GitHubError::Schema(format!("graphql error [{}]: {}", ty, first.message)),
            });
        }
    }
    env.data
        .ok_or_else(|| GitHubError::Schema("missing data field".to_string()))
}

#[derive(Deserialize, Debug)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize, Debug)]
struct RawItemsConnection {
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
    nodes: Vec<RawProjectItem>,
}

#[derive(Deserialize, Debug)]
struct RawListProjectItemsData {
    node: Option<RawNode>,
}

#[derive(Deserialize, Debug)]
struct RawNode {
    items: RawItemsConnection,
}

#[derive(Deserialize, Debug)]
struct RawGetProjectItemData {
    node: Option<RawProjectItem>,
}

// Slim raw payload for the `projectItems` connection on an Issue. We
// only need item ids + their parent Project ids — full ProjectItem
// fields come back via the existing `get_project_item` round-trip.
#[derive(Deserialize, Debug)]
struct RawProjectItemsForIssueData {
    node: Option<RawIssueProjectItems>,
}

#[derive(Deserialize, Debug)]
struct RawIssueProjectItems {
    #[serde(rename = "projectItems")]
    project_items: RawIssueProjectItemsConn,
}

#[derive(Deserialize, Debug)]
struct RawIssueProjectItemsConn {
    nodes: Vec<RawIssueProjectItemRef>,
}

#[derive(Deserialize, Debug)]
struct RawIssueProjectItemRef {
    id: String,
    project: RawProjectIdOnly,
}

#[derive(Deserialize, Debug)]
struct RawProjectIdOnly {
    id: String,
}

#[derive(Deserialize, Debug)]
struct RawProjectItem {
    id: String,
    content: Option<RawContent>,
    #[serde(rename = "fieldValues")]
    field_values: RawFieldValuesConnection,
}

#[derive(Deserialize, Debug)]
struct RawContent {
    #[serde(rename = "__typename")]
    typename: String,
    title: Option<String>,
    body: Option<String>,
    url: Option<String>,
    #[serde(rename = "mergedAt")]
    merged_at: Option<DateTime<Utc>>,
    /// Labels on the linked Issue / PR. Absent for DraftIssue.
    labels: Option<RawLabelsConnection>,
}

#[derive(Deserialize, Debug)]
struct RawLabelsConnection {
    nodes: Vec<RawLabel>,
}

#[derive(Deserialize, Debug)]
struct RawLabel {
    name: String,
}

fn collect_labels(raw: &RawContent) -> Vec<String> {
    raw.labels
        .as_ref()
        .map(|conn| conn.nodes.iter().map(|l| l.name.clone()).collect())
        .unwrap_or_default()
}

#[derive(Deserialize, Debug)]
struct RawFieldValuesConnection {
    nodes: Vec<RawFieldValue>,
}

#[derive(Deserialize, Debug)]
struct RawFieldValue {
    #[serde(rename = "__typename")]
    typename: String,
    // Per-variant fields. All Optional so a single struct covers every
    // GraphQL union arm.
    text: Option<String>,
    number: Option<f64>,
    /// For SingleSelect, GitHub returns `name` (option label) +
    /// `optionId`.
    name: Option<String>,
    #[serde(rename = "optionId")]
    option_id: Option<String>,
    date: Option<DateTime<Utc>>,
    title: Option<String>,
    /// The owning field — used as the key in our `HashMap`.
    field: Option<RawFieldRef>,
}

#[derive(Deserialize, Debug)]
struct RawFieldRef {
    name: Option<String>,
}

fn parse_field_value(raw: &RawFieldValue) -> Option<(String, ProjectFieldValue)> {
    let field_name = raw.field.as_ref().and_then(|f| f.name.clone())?;
    let value = match raw.typename.as_str() {
        "ProjectV2ItemFieldTextValue" => ProjectFieldValue::Text(raw.text.clone()?),
        "ProjectV2ItemFieldNumberValue" => ProjectFieldValue::Number(raw.number?),
        "ProjectV2ItemFieldSingleSelectValue" => ProjectFieldValue::SingleSelect {
            option_name: raw.name.clone()?,
            option_id: raw.option_id.clone()?,
        },
        "ProjectV2ItemFieldDateValue" => ProjectFieldValue::Date(raw.date?),
        "ProjectV2ItemFieldIterationValue" => ProjectFieldValue::Iteration {
            title: raw.title.clone()?,
        },
        _ => return None, // unknown field type — ignore rather than fail
    };
    Some((field_name, value))
}

fn parse_project_item(raw: RawProjectItem) -> ProjectItem {
    let content = match raw.content {
        None => ProjectItemContent::Other {
            typename: "<missing content>".to_string(),
        },
        Some(c) => {
            let labels = collect_labels(&c);
            match c.typename.as_str() {
                "Issue" => ProjectItemContent::Issue {
                    title: c.title.unwrap_or_default(),
                    body: c.body.unwrap_or_default(),
                    url: c.url.unwrap_or_default(),
                    labels,
                },
                "PullRequest" => ProjectItemContent::PullRequest {
                    title: c.title.unwrap_or_default(),
                    body: c.body.unwrap_or_default(),
                    url: c.url.unwrap_or_default(),
                    merged_at: c.merged_at,
                    labels,
                },
                "DraftIssue" => ProjectItemContent::DraftIssue {
                    title: c.title.unwrap_or_default(),
                    body: c.body.unwrap_or_default(),
                },
                other => ProjectItemContent::Other {
                    typename: other.to_string(),
                },
            }
        }
    };
    let mut custom_fields = HashMap::new();
    for raw_fv in &raw.field_values.nodes {
        if let Some((k, v)) = parse_field_value(raw_fv) {
            custom_fields.insert(k, v);
        }
    }
    ProjectItem {
        id: raw.id,
        content,
        custom_fields,
    }
}

/// Coalesce a vec of pages into a flat `Vec<ProjectItem>`.
fn coalesce_pages(pages: Vec<RawItemsConnection>) -> Vec<ProjectItem> {
    pages
        .into_iter()
        .flat_map(|p| p.nodes.into_iter().map(parse_project_item))
        .collect()
}

// ---------- queries ---------------------------------------------------------

const LIST_PROJECT_ITEMS_QUERY: &str = r#"
query($projectId: ID!, $cursor: String) {
  node(id: $projectId) {
    ... on ProjectV2 {
      items(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          content {
            __typename
            ... on Issue { title body url labels(first: 20) { nodes { name } } }
            ... on PullRequest { title body url mergedAt labels(first: 20) { nodes { name } } }
            ... on DraftIssue { title body }
          }
          fieldValues(first: 50) {
            nodes {
              __typename
              ... on ProjectV2ItemFieldTextValue {
                text
                field { ... on ProjectV2Field { name } }
              }
              ... on ProjectV2ItemFieldNumberValue {
                number
                field { ... on ProjectV2Field { name } }
              }
              ... on ProjectV2ItemFieldSingleSelectValue {
                name
                optionId
                field { ... on ProjectV2SingleSelectField { name } }
              }
              ... on ProjectV2ItemFieldDateValue {
                date
                field { ... on ProjectV2Field { name } }
              }
              ... on ProjectV2ItemFieldIterationValue {
                title
                field { ... on ProjectV2IterationField { name } }
              }
            }
          }
        }
      }
    }
  }
}
"#;

const GET_PROJECT_ITEM_QUERY: &str = r#"
query($itemId: ID!) {
  node(id: $itemId) {
    ... on ProjectV2Item {
      id
      content {
        __typename
        ... on Issue { title body url }
        ... on PullRequest { title body url mergedAt }
        ... on DraftIssue { title body }
      }
      fieldValues(first: 50) {
        nodes {
          __typename
          ... on ProjectV2ItemFieldTextValue { text field { ... on ProjectV2Field { name } } }
          ... on ProjectV2ItemFieldNumberValue { number field { ... on ProjectV2Field { name } } }
          ... on ProjectV2ItemFieldSingleSelectValue { name optionId field { ... on ProjectV2SingleSelectField { name } } }
          ... on ProjectV2ItemFieldDateValue { date field { ... on ProjectV2Field { name } } }
          ... on ProjectV2ItemFieldIterationValue { title field { ... on ProjectV2IterationField { name } } }
        }
      }
    }
  }
}
"#;

// Issue → linked Project items (filtered to one project in code, not
// GraphQL — projectItems doesn't accept a filter argument). 20 should
// cover the realistic case (one Issue on a handful of Projects); we
// don't paginate this connection.
const PROJECT_ITEMS_FOR_ISSUE_QUERY: &str = r#"
query($issueId: ID!) {
  node(id: $issueId) {
    ... on Issue {
      projectItems(first: 20) {
        nodes {
          id
          project { id }
        }
      }
    }
  }
}
"#;

// ---------- client ----------------------------------------------------------

pub struct GitHubGraphQLClient {
    http: reqwest::Client,
    creds: GitHubAppCreds,
    api_base: String,
    graphql_url: String,
    tokens: TokenCache,
}

impl GitHubGraphQLClient {
    pub fn new(creds: GitHubAppCreds) -> Self {
        Self::with_api_base(creds, "https://api.github.com")
    }

    pub fn with_api_base(creds: GitHubAppCreds, api_base: impl Into<String>) -> Self {
        let api_base = api_base.into().trim_end_matches('/').to_string();
        let graphql_url = format!("{api_base}/graphql");
        let http = reqwest::Client::builder()
            .user_agent("StarStats-Roadmap/1.0")
            .build()
            .expect("reqwest Client build");
        Self {
            http,
            creds,
            api_base,
            graphql_url,
            tokens: TokenCache::new(),
        }
    }

    /// Fetch a fresh installation token from the GitHub API. Used by
    /// the cache when no valid token is available.
    async fn fetch_installation_token_now(
        &self,
        now: DateTime<Utc>,
    ) -> Result<InstallationToken, GitHubError> {
        let jwt = mint_app_jwt(&self.creds, now)?;
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.api_base, self.creds.installation_id
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GitHubError::Transient(format!("install token POST: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(GitHubError::Auth(format!("install token: {status}")));
        }
        if status.is_server_error() {
            return Err(GitHubError::Transient(format!(
                "install token 5xx: {status}"
            )));
        }
        if !status.is_success() {
            return Err(GitHubError::Other(format!("install token: {status}")));
        }
        #[derive(Deserialize)]
        struct Resp {
            token: String,
            expires_at: DateTime<Utc>,
        }
        let body: Resp = resp
            .json()
            .await
            .map_err(|e| GitHubError::Schema(format!("install token parse: {e}")))?;
        Ok(InstallationToken {
            token: body.token,
            expires_at: body.expires_at,
        })
    }

    pub async fn ensure_installation_token(&self) -> Result<String, GitHubError> {
        let now = Utc::now();
        self.tokens
            .get_or_fetch(now, || self.fetch_installation_token_now(now))
            .await
    }

    /// One-shot GraphQL POST. No retry. Used internally by `graphql`.
    async fn graphql_once<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<T, GitHubError> {
        let token = self.ensure_installation_token().await?;
        let body = serde_json::json!({ "query": query, "variables": variables });
        let resp = self
            .http
            .post(&self.graphql_url)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await
            .map_err(|e| GitHubError::Transient(format!("graphql POST: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(GitHubError::Auth(format!("graphql HTTP {status}")));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok());
            return Err(GitHubError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status.is_server_error() {
            return Err(GitHubError::Transient(format!("graphql 5xx: {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| GitHubError::Transient(format!("graphql read body: {e}")))?;
        parse_graphql_response::<T>(&bytes)
    }

    /// GraphQL query with default backoff retry policy.
    pub async fn graphql<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<T, GitHubError> {
        with_backoff(DEFAULT_BACKOFF_MS, || {
            self.graphql_once::<T>(query, variables.clone())
        })
        .await
    }

    /// List all items on the given Project (v2), paginating through.
    pub async fn list_project_items(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProjectItem>, GitHubError> {
        let mut pages: Vec<RawItemsConnection> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let vars = serde_json::json!({
                "projectId": project_id,
                "cursor": cursor,
            });
            let data: RawListProjectItemsData =
                self.graphql(LIST_PROJECT_ITEMS_QUERY, vars).await?;
            let node = data
                .node
                .ok_or_else(|| GitHubError::Schema(format!("project not found: {project_id}")))?;
            let has_next = node.items.page_info.has_next_page;
            let end_cursor = node.items.page_info.end_cursor.clone();
            pages.push(node.items);
            if !has_next || end_cursor.is_none() {
                break;
            }
            cursor = end_cursor;
        }
        Ok(coalesce_pages(pages))
    }

    /// Hot-read a single project item by ID.
    pub async fn get_project_item(&self, item_id: &str) -> Result<ProjectItem, GitHubError> {
        let vars = serde_json::json!({ "itemId": item_id });
        let data: RawGetProjectItemData = self.graphql(GET_PROJECT_ITEM_QUERY, vars).await?;
        let raw = data
            .node
            .ok_or_else(|| GitHubError::Schema(format!("item not found: {item_id}")))?;
        Ok(parse_project_item(raw))
    }

    /// Return Project item IDs for every Project item that has the
    /// given Issue as its content, filtered to those belonging to
    /// `project_id`. Used by the webhook receiver to find which local
    /// rows to re-sync when an Issue's labels change.
    ///
    /// An Issue can be on multiple Projects (across the org); the
    /// filter limits the result to our configured Project so we don't
    /// thrash on changes that don't affect us.
    pub async fn list_project_item_ids_for_issue(
        &self,
        issue_id: &str,
        project_id: &str,
    ) -> Result<Vec<String>, GitHubError> {
        let vars = serde_json::json!({ "issueId": issue_id });
        let data: RawProjectItemsForIssueData =
            self.graphql(PROJECT_ITEMS_FOR_ISSUE_QUERY, vars).await?;
        let Some(node) = data.node else {
            return Ok(Vec::new());
        };
        Ok(node
            .project_items
            .nodes
            .into_iter()
            .filter(|n| n.project.id == project_id)
            .map(|n| n.id)
            .collect())
    }
}

// ---------- GitHubReader trait (test seam) ---------------------------------

/// Read-only seam over the GitHub Projects v2 API. Lets sync code
/// depend on a behaviour rather than the concrete reqwest-backed
/// client, so tests can substitute fakes.
#[async_trait]
pub trait GitHubReader: Send + Sync {
    async fn list_project_items(&self, project_id: &str) -> Result<Vec<ProjectItem>, GitHubError>;

    async fn get_project_item(&self, item_id: &str) -> Result<ProjectItem, GitHubError>;

    /// Project item IDs for every Project item containing the given
    /// Issue, scoped to `project_id`. See
    /// [`GitHubGraphQLClient::list_project_item_ids_for_issue`].
    async fn list_project_item_ids_for_issue(
        &self,
        issue_id: &str,
        project_id: &str,
    ) -> Result<Vec<String>, GitHubError>;
}

#[async_trait]
impl GitHubReader for GitHubGraphQLClient {
    async fn list_project_items(&self, project_id: &str) -> Result<Vec<ProjectItem>, GitHubError> {
        GitHubGraphQLClient::list_project_items(self, project_id).await
    }

    async fn get_project_item(&self, item_id: &str) -> Result<ProjectItem, GitHubError> {
        GitHubGraphQLClient::get_project_item(self, item_id).await
    }

    async fn list_project_item_ids_for_issue(
        &self,
        issue_id: &str,
        project_id: &str,
    ) -> Result<Vec<String>, GitHubError> {
        GitHubGraphQLClient::list_project_item_ids_for_issue(self, issue_id, project_id).await
    }
}

// ---------- tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;

    // Reuse a single 2048-bit RSA key across tests so we only pay the
    // key-generation cost once per test process.
    fn test_rsa_pem() -> String {
        static KEY: OnceLock<String> = OnceLock::new();
        KEY.get_or_init(|| {
            use rsa::pkcs8::EncodePrivateKey;
            let priv_key =
                rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("rsa generate");
            priv_key
                .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
                .expect("pkcs8 pem")
                .to_string()
        })
        .clone()
    }

    fn test_creds() -> GitHubAppCreds {
        GitHubAppCreds {
            app_id: "1234".to_string(),
            installation_id: "5678".to_string(),
            private_key: test_rsa_pem(),
        }
    }

    fn decode_jwt_claims(token: &str) -> serde_json::Value {
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have 3 segments");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .expect("base64 decode");
        serde_json::from_slice(&payload).expect("claims json")
    }

    #[test]
    fn mint_app_jwt_has_correct_claims() {
        let creds = test_creds();
        let now = Utc::now();
        let token = mint_app_jwt(&creds, now).expect("mint jwt");
        let claims = decode_jwt_claims(&token);
        assert_eq!(claims["iss"].as_str().unwrap(), "1234");
        let iat = claims["iat"].as_i64().unwrap();
        let exp = claims["exp"].as_i64().unwrap();
        let now_ts = now.timestamp();
        // iat is exactly 60s in the past.
        assert_eq!(iat, now_ts - 60, "iat should be exactly now - 60s");
        // exp is exactly 9 minutes ahead.
        assert_eq!(exp, now_ts + 9 * 60, "exp should be exactly now + 9 min");
        // Sanity: exp - iat is well under GitHub's 10-minute ceiling.
        assert!(exp - iat <= 10 * 60);
    }

    #[test]
    fn mint_app_jwt_rejects_garbage_pem() {
        let creds = GitHubAppCreds {
            app_id: "1".into(),
            installation_id: "2".into(),
            private_key: "not a pem".into(),
        };
        let err = mint_app_jwt(&creds, Utc::now()).expect_err("garbage PEM");
        assert!(matches!(err, GitHubError::Auth(_)));
    }

    #[tokio::test]
    async fn installation_token_caches_until_5_min_before_expiry() {
        let cache = TokenCache::new();
        let call_count = AtomicU32::new(0);
        let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();

        // Initial fetch.
        let t1 = cache
            .get_or_fetch(now, || async {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, GitHubError>(InstallationToken {
                    token: "tok".to_string(),
                    expires_at: now + Duration::minutes(60),
                })
            })
            .await
            .unwrap();
        assert_eq!(t1, "tok");

        // 30 min later — still cached.
        let t2 = cache
            .get_or_fetch(now + Duration::minutes(30), || async {
                panic!("should not refetch within validity window");
            })
            .await
            .unwrap();
        assert_eq!(t2, "tok");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // 54 min later — within 6 min of expiry but still more than 5 — still cached.
        let _ = cache
            .get_or_fetch(now + Duration::minutes(54), || async {
                panic!("should not refetch yet");
            })
            .await
            .unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // 56 min later — within 5 min safety window — refetches.
        let t4 = cache
            .get_or_fetch(now + Duration::minutes(56), || async {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok(InstallationToken {
                    token: "tok2".to_string(),
                    expires_at: now + Duration::minutes(120),
                })
            })
            .await
            .unwrap();
        assert_eq!(t4, "tok2");
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rate_limit_retries_with_backoff_to_success() {
        let counter = AtomicU32::new(0);
        let result = with_backoff(&[0, 0, 0], || async {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(GitHubError::RateLimited {
                    retry_after_seconds: None,
                })
            } else {
                Ok::<u32, GitHubError>(n)
            }
        })
        .await;
        assert!(matches!(result, Ok(2)));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn transient_retries_then_drains_to_last_err() {
        let counter = AtomicU32::new(0);
        let result: Result<(), GitHubError> = with_backoff(&[0, 0, 0], || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(GitHubError::Transient("net".into()))
        })
        .await;
        assert!(matches!(result, Err(GitHubError::Transient(_))));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn auth_error_does_not_retry() {
        let counter = AtomicU32::new(0);
        let result: Result<(), GitHubError> = with_backoff(&[0, 0, 0], || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(GitHubError::Auth("bad token".into()))
        })
        .await;
        assert!(matches!(result, Err(GitHubError::Auth(_))));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn schema_error_does_not_retry() {
        let counter = AtomicU32::new(0);
        let result: Result<(), GitHubError> = with_backoff(&[0, 0, 0], || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(GitHubError::Schema("missing field".into()))
        })
        .await;
        assert!(matches!(result, Err(GitHubError::Schema(_))));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parse_graphql_response_unwraps_data() {
        #[derive(Deserialize)]
        struct Body {
            name: String,
        }
        let raw = br#"{"data": {"name": "hello"}}"#;
        let parsed: Body = parse_graphql_response(raw).unwrap();
        assert_eq!(parsed.name, "hello");
    }

    #[test]
    fn parse_graphql_response_maps_rate_limited_error_type() {
        let raw = br#"{"errors": [{"message": "rate limit", "type": "RATE_LIMITED"}]}"#;
        let err: Result<serde_json::Value, _> = parse_graphql_response(raw);
        assert!(matches!(
            err,
            Err(GitHubError::RateLimited {
                retry_after_seconds: None
            })
        ));
    }

    #[test]
    fn parse_graphql_response_maps_unauthenticated_to_auth() {
        let raw = br#"{"errors": [{"message": "bad", "type": "UNAUTHENTICATED"}]}"#;
        let err: Result<serde_json::Value, _> = parse_graphql_response(raw);
        assert!(matches!(err, Err(GitHubError::Auth(_))));
    }

    #[test]
    fn parse_graphql_response_carries_unknown_error_as_schema() {
        let raw = br#"{"errors": [{"message": "weird thing", "type": "WEIRD"}]}"#;
        let err: Result<serde_json::Value, _> = parse_graphql_response(raw);
        match err {
            Err(GitHubError::Schema(msg)) => {
                assert!(
                    msg.contains("WEIRD"),
                    "expected payload to carry type tag: {msg}"
                );
                assert!(msg.contains("weird thing"));
            }
            other => panic!("expected Schema, got {other:?}"),
        }
    }

    #[test]
    fn parse_graphql_response_rejects_malformed_envelope() {
        let raw = br#"not json"#;
        let err: Result<serde_json::Value, _> = parse_graphql_response(raw);
        assert!(matches!(err, Err(GitHubError::Schema(_))));
    }

    // ---------- field-value parser ----------

    fn raw_text(field: &str, text: &str) -> RawFieldValue {
        RawFieldValue {
            typename: "ProjectV2ItemFieldTextValue".into(),
            text: Some(text.into()),
            number: None,
            name: None,
            option_id: None,
            date: None,
            title: None,
            field: Some(RawFieldRef {
                name: Some(field.into()),
            }),
        }
    }

    fn raw_number(field: &str, n: f64) -> RawFieldValue {
        RawFieldValue {
            typename: "ProjectV2ItemFieldNumberValue".into(),
            text: None,
            number: Some(n),
            name: None,
            option_id: None,
            date: None,
            title: None,
            field: Some(RawFieldRef {
                name: Some(field.into()),
            }),
        }
    }

    fn raw_select(field: &str, name: &str, id: &str) -> RawFieldValue {
        RawFieldValue {
            typename: "ProjectV2ItemFieldSingleSelectValue".into(),
            text: None,
            number: None,
            name: Some(name.into()),
            option_id: Some(id.into()),
            date: None,
            title: None,
            field: Some(RawFieldRef {
                name: Some(field.into()),
            }),
        }
    }

    fn raw_date(field: &str, d: DateTime<Utc>) -> RawFieldValue {
        RawFieldValue {
            typename: "ProjectV2ItemFieldDateValue".into(),
            text: None,
            number: None,
            name: None,
            option_id: None,
            date: Some(d),
            title: None,
            field: Some(RawFieldRef {
                name: Some(field.into()),
            }),
        }
    }

    fn raw_iteration(field: &str, title: &str) -> RawFieldValue {
        RawFieldValue {
            typename: "ProjectV2ItemFieldIterationValue".into(),
            text: None,
            number: None,
            name: None,
            option_id: None,
            date: None,
            title: Some(title.into()),
            field: Some(RawFieldRef {
                name: Some(field.into()),
            }),
        }
    }

    #[test]
    fn field_value_parser_handles_all_five_variants() {
        let now = Utc::now();
        let text = parse_field_value(&raw_text("Status", "in flight")).unwrap();
        assert_eq!(text.0, "Status");
        assert_eq!(text.1, ProjectFieldValue::Text("in flight".into()));

        let num = parse_field_value(&raw_number("Votes", 42.0)).unwrap();
        assert_eq!(num.1, ProjectFieldValue::Number(42.0));

        let sel = parse_field_value(&raw_select("Category", "UI", "OPT_1")).unwrap();
        assert_eq!(
            sel.1,
            ProjectFieldValue::SingleSelect {
                option_name: "UI".into(),
                option_id: "OPT_1".into()
            }
        );

        let date = parse_field_value(&raw_date("Target", now)).unwrap();
        assert_eq!(date.1, ProjectFieldValue::Date(now));

        let iter = parse_field_value(&raw_iteration("Cycle", "Sprint 12")).unwrap();
        assert_eq!(
            iter.1,
            ProjectFieldValue::Iteration {
                title: "Sprint 12".into()
            }
        );
    }

    #[test]
    fn field_value_parser_ignores_unknown_typename() {
        let raw = RawFieldValue {
            typename: "ProjectV2ItemFieldFutureFancyValue".into(),
            text: None,
            number: None,
            name: None,
            option_id: None,
            date: None,
            title: None,
            field: Some(RawFieldRef {
                name: Some("X".into()),
            }),
        };
        assert!(parse_field_value(&raw).is_none());
    }

    #[test]
    fn issue_content_carries_labels_for_channel_extraction() {
        let raw = RawProjectItem {
            id: "PVTI_z".into(),
            content: Some(RawContent {
                typename: "Issue".into(),
                title: Some("Voting UI".into()),
                body: Some("...".into()),
                url: Some("https://x/z".into()),
                merged_at: None,
                labels: Some(RawLabelsConnection {
                    nodes: vec![
                        RawLabel {
                            name: "channel/live".into(),
                        },
                        RawLabel {
                            name: "channel/beta".into(),
                        },
                        RawLabel {
                            name: "area:roadmap".into(),
                        },
                    ],
                }),
            }),
            field_values: RawFieldValuesConnection { nodes: vec![] },
        };
        let item = parse_project_item(raw);
        match item.content {
            ProjectItemContent::Issue { labels, .. } => {
                assert_eq!(labels.len(), 3);
                let channels: Vec<&str> = labels
                    .iter()
                    .filter_map(|l| l.strip_prefix("channel/"))
                    .collect();
                assert_eq!(channels, vec!["live", "beta"]);
            }
            other => panic!("expected Issue content, got {other:?}"),
        }
    }

    #[test]
    fn draft_issue_has_no_labels_field() {
        let raw = RawProjectItem {
            id: "PVTI_d".into(),
            content: Some(RawContent {
                typename: "DraftIssue".into(),
                title: Some("Draft".into()),
                body: Some("not yet promoted".into()),
                url: None,
                merged_at: None,
                labels: None,
            }),
            field_values: RawFieldValuesConnection { nodes: vec![] },
        };
        let item = parse_project_item(raw);
        assert!(matches!(
            item.content,
            ProjectItemContent::DraftIssue { .. }
        ));
    }

    #[test]
    fn list_project_items_pages_coalesce() {
        // Two pages, two items each, with distinct field values.
        let p1 = RawItemsConnection {
            page_info: PageInfo {
                has_next_page: true,
                end_cursor: Some("c1".into()),
            },
            nodes: vec![
                RawProjectItem {
                    id: "PVTI_a".into(),
                    content: Some(RawContent {
                        typename: "Issue".into(),
                        title: Some("Item A".into()),
                        body: Some("body A".into()),
                        url: Some("https://x/a".into()),
                        merged_at: None,
                        labels: Some(RawLabelsConnection {
                            nodes: vec![
                                RawLabel {
                                    name: "channel/live".into(),
                                },
                                RawLabel {
                                    name: "channel/beta".into(),
                                },
                            ],
                        }),
                    }),
                    field_values: RawFieldValuesConnection {
                        nodes: vec![raw_text("Status", "shipped")],
                    },
                },
                RawProjectItem {
                    id: "PVTI_b".into(),
                    content: Some(RawContent {
                        typename: "DraftIssue".into(),
                        title: Some("Item B".into()),
                        body: Some("body B".into()),
                        url: None,
                        merged_at: None,
                        labels: None,
                    }),
                    field_values: RawFieldValuesConnection { nodes: vec![] },
                },
            ],
        };
        let p2 = RawItemsConnection {
            page_info: PageInfo {
                has_next_page: false,
                end_cursor: None,
            },
            nodes: vec![RawProjectItem {
                id: "PVTI_c".into(),
                content: Some(RawContent {
                    typename: "PullRequest".into(),
                    title: Some("Item C".into()),
                    body: Some("body C".into()),
                    url: Some("https://x/c".into()),
                    merged_at: Some(Utc::now()),
                    labels: Some(RawLabelsConnection { nodes: vec![] }),
                }),
                field_values: RawFieldValuesConnection {
                    nodes: vec![raw_number("Votes", 7.0)],
                },
            }],
        };
        let coalesced = coalesce_pages(vec![p1, p2]);
        assert_eq!(coalesced.len(), 3);
        assert_eq!(coalesced[0].id, "PVTI_a");
        assert_eq!(
            coalesced[0].custom_fields.get("Status"),
            Some(&ProjectFieldValue::Text("shipped".into()))
        );
        assert!(matches!(
            &coalesced[1].content,
            ProjectItemContent::DraftIssue { .. }
        ));
        assert_eq!(
            coalesced[2].custom_fields.get("Votes"),
            Some(&ProjectFieldValue::Number(7.0))
        );
    }

    #[test]
    fn project_item_unknown_content_typename_does_not_panic() {
        let raw = RawProjectItem {
            id: "PVTI_x".into(),
            content: Some(RawContent {
                typename: "SomeFutureType".into(),
                title: None,
                body: None,
                url: None,
                merged_at: None,
                labels: None,
            }),
            field_values: RawFieldValuesConnection { nodes: vec![] },
        };
        let item = parse_project_item(raw);
        assert!(matches!(
            item.content,
            ProjectItemContent::Other { typename } if typename == "SomeFutureType"
        ));
    }
}
