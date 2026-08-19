//! Shared error envelope for all API routes.
//!
//! Every handler that needs a JSON error body should use `ApiErrorBody`
//! as the `ToSchema` type in `#[utoipa::path(...)]` response declarations.
//! Individual modules keep their own private `err()` helper for
//! constructing responses — the `&'static str` variant avoids an
//! allocation for codes known at compile time.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The single error envelope shape emitted by every API endpoint.
///
/// `detail` is omitted from the serialised output when `None` so
/// clients that only care about `error` don't need to handle a null
/// field.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorBody {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
