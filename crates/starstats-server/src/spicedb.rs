//! SpiceDB authorization client foundation.
//!
//! StarStats uses SpiceDB (Zanzibar-style ReBAC) for cross-user
//! permission decisions. The schema lives at `infra/spicedb/schema.zed`
//! and is applied separately via the `zed` CLI; this module is a
//! thin client wrapper that the server uses at request time.
//!
//! ## Posture
//! - [`SpicedbClient::connect`] is async and may fail if the sidecar
//!   is unreachable. Callers should treat that as **degraded mode** —
//!   log a warning and continue without SpiceDB rather than fail boot.
//! - [`SpicedbClient::ping`] performs a real round-trip (read schema)
//!   so `/readyz` can flag a misconfigured deployment.
//! - [`SpicedbClient::check_permission`] wraps `CheckPermission` and
//!   returns a `bool`. Conditional / caveated permissions are reported
//!   as deny — the StarStats schema does not currently use caveats.
//!
//! ## Why this crate
//! `spicedb-client` (v0.1) wraps the auto-generated `spicedb-grpc`
//! types behind an ergonomic `SpicedbClient`. It pins `tonic 0.12`,
//! which already lives in our dep tree via `opentelemetry-otlp`. The
//! alternative — vendoring `tonic-build` + the `authzed/api` protos —
//! is significantly more wiring for the same surface area we need.

use crate::config::SpicedbConfig;
use anyhow::{Context, Result};
use async_trait::async_trait;
use spicedb_client::builder::{
    ReadRelationshipsRequestBuilder, RelationshipFilterBuilder, SubjectFilterBuilder,
    WriteRelationshipsRequestBuilder,
};
use spicedb_grpc::authzed::api::v1::{
    check_permission_response::Permissionship, consistency::Requirement as ConsistencyRequirement,
    CheckPermissionRequest, Consistency, DeleteRelationshipsRequest, ObjectReference,
    ReadRelationshipsRequest, RelationshipFilter, SubjectFilter, SubjectReference,
    WriteRelationshipsRequest,
};

/// Reference to a SpiceDB object (resource or subject).
///
/// `object_type` mirrors a `definition` name from the .zed schema
/// (e.g. `"user"`, `"stats_record"`); `object_id` is the application
/// identifier (e.g. a UUID, or in our case the `preferred_username`).
#[derive(Debug, Clone)]
pub struct ObjectRef {
    pub object_type: String,
    pub object_id: String,
}

impl ObjectRef {
    pub fn new(object_type: impl Into<String>, object_id: impl Into<String>) -> Self {
        Self {
            object_type: object_type.into(),
            object_id: object_id.into(),
        }
    }
}

impl From<ObjectRef> for ObjectReference {
    fn from(r: ObjectRef) -> Self {
        ObjectReference {
            object_type: r.object_type,
            object_id: r.object_id,
        }
    }
}

/// Thin wrapper over `spicedb_client::SpicedbClient`.
///
/// `Clone` is cheap — the inner type wraps an `Arc<Channel>`-style
/// tonic channel, so cloning shares the underlying connection.
#[derive(Clone)]
pub struct SpicedbClient {
    inner: spicedb_client::SpicedbClient,
}

impl SpicedbClient {
    /// Connect to the SpiceDB sidecar and authenticate with the
    /// preshared key.
    ///
    /// This actually opens the gRPC channel — failures here mean the
    /// sidecar is unreachable or the URL is malformed. Callers should
    /// log + degrade rather than panic.
    pub async fn connect(cfg: SpicedbConfig) -> Result<Self> {
        let inner = spicedb_client::SpicedbClient::from_url_and_preshared_key(
            cfg.endpoint.clone(),
            cfg.preshared_key,
        )
        .await
        .with_context(|| format!("connect to SpiceDB at {}", cfg.endpoint))?;

        Ok(Self { inner })
    }

    /// Confirm the channel is live by issuing a read-only RPC.
    ///
    /// Uses `ReadSchema`, which is the cheapest call that exercises
    /// authn (preshared key) and the gRPC plumbing. A `NotFound` from
    /// SpiceDB (no schema written yet) is still a successful ping —
    /// the server is reachable, the schema just hasn't been applied.
    pub async fn ping(&self) -> Result<()> {
        // `read_schema` takes `&mut self` on the inner client, but the
        // inner client is internally `Clone` over a shared channel —
        // so a per-call clone is the standard pattern.
        let mut inner = self.inner.clone();
        match inner.read_schema().await {
            Ok(_) => Ok(()),
            Err(e) => {
                // Treat NotFound (schema absent) as a successful ping
                // — the server is reachable, the schema just hasn't
                // been applied yet. We match on the gRPC status code
                // string rather than pulling tonic into our direct
                // deps; the error variant is stable.
                if let spicedb_client::result::Error::TonicStatus(status) = &e {
                    // `code()` returns a `tonic::Code` whose Display
                    // for NotFound is the literal "NotFound".
                    let code_str = format!("{:?}", status.code());
                    if code_str == "NotFound" {
                        tracing::debug!(
                            "SpiceDB ping: schema not yet written (NotFound), \
                             treating as reachable"
                        );
                        return Ok(());
                    }
                }
                Err(anyhow::anyhow!("SpiceDB ping failed: {e}"))
            }
        }
    }

    /// Check whether `subject` has `permission` on `resource`.
    ///
    /// Returns `Ok(true)` only when SpiceDB reports
    /// `PERMISSIONSHIP_HAS_PERMISSION`. Conditional permissions
    /// (caveated) and explicit denies both yield `Ok(false)`. RPC
    /// errors propagate as `Err` so callers can decide whether to
    /// fail-open (advisory) or fail-closed (enforced).
    pub async fn check_permission(
        &self,
        resource: ObjectRef,
        permission: &str,
        subject: ObjectRef,
    ) -> Result<bool> {
        self.check_permission_with_consistency(resource, permission, subject, false)
            .await
    }

    async fn check_permission_with_consistency(
        &self,
        resource: ObjectRef,
        permission: &str,
        subject: ObjectRef,
        fully_consistent: bool,
    ) -> Result<bool> {
        let request = CheckPermissionRequest {
            consistency: consistency_requirement(fully_consistent),
            resource: Some(resource.into()),
            permission: permission.to_string(),
            subject: Some(SubjectReference {
                object: Some(subject.into()),
                optional_relation: String::new(),
            }),
            context: None,
            with_tracing: false,
        };

        let mut inner = self.inner.clone();
        let response = inner
            .check_permission(request)
            .await
            .context("SpiceDB CheckPermission RPC failed")?;

        Ok(response.permissionship() == Permissionship::HasPermission)
    }

    // -- Relationship writes / reads ---------------------------------
    //
    // The wave-2 sharing endpoints mutate `stats_record:#share_with_*`
    // relationships directly. We use the `spicedb-client` builder
    // helpers (`create_relationship` / `delete_relationship`) so the
    // call sites stay readable; the inner type still wraps a single
    // `WriteRelationships` RPC per call. None of these methods batch
    // — share grants/revokes are user-initiated and rare.

    /// Write the owner relationship for a stats_record, encoding
    /// "this handle owns its own stats record":
    /// `stats_record:<handle>#owner@user:<handle>`.
    ///
    /// Called once per signup. Idempotent (TOUCH semantics) — also
    /// safe to invoke from any path that wants to ensure the
    /// relation exists (e.g. a future backfill for accounts that
    /// pre-date this wiring).
    ///
    /// Without this relation, the `stats_record.view` permission
    /// (which sums `owner + share_with_user + share_with_org->view +
    /// share_with_org->view_all_member_stats + public_view`) is empty
    /// for a freshly-created account — so any reinstated SpiceDB
    /// self-view gate would 403 the owner of the data. See the
    /// `query::summary` comment block (which skips SpiceDB on
    /// self-view today) for the historical context.
    pub async fn write_owner(&self, handle: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.update_relationship("stats_record", handle, "owner", "user", handle);
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (owner) failed")?;
        Ok(())
    }

    /// Mark a stats_record as publicly viewable by writing the
    /// wildcard relationship `stats_record:<handle>#public_view@user:*`.
    ///
    /// Idempotent — uses TOUCH semantics so re-issuing the same write
    /// is a no-op.
    pub async fn write_public_view(&self, handle: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        // TOUCH = create-or-update; safe to call repeatedly.
        req.update_relationship("stats_record", handle, "public_view", "user", "*");
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (public_view) failed")?;
        Ok(())
    }

    /// Remove the public-view wildcard for `handle`.
    pub async fn delete_public_view(&self, handle: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.delete_relationship("stats_record", handle, "public_view", "user", "*");
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (delete public_view) failed")?;
        Ok(())
    }

    /// Probe whether `stats_record:<handle>#public_view@user:*` is
    /// currently written — backs the user-facing "is my profile
    /// public?" toggle on `/sharing`.
    ///
    /// We do NOT use `CheckPermission` here: SpiceDB rejects checks
    /// against a wildcard subject with `InvalidArgument: cannot
    /// perform check on wildcard subject`. The toggle's truth is
    /// "does this exact tuple exist?" not "would `view` resolve for
    /// somebody" — so `ReadRelationships` with `optional_limit = 1`
    /// is both legal and semantically tighter. Pinned to
    /// `fully_consistent` because callers (`get_visibility`) read
    /// immediately after their own writes via the redirect-then-render
    /// flow on the Next.js page, and a follower-replica stale snapshot
    /// would replay the "enable then disable broken" UX.
    pub async fn has_public_view(&self, handle: &str) -> Result<bool> {
        let filter = RelationshipFilter {
            resource_type: "stats_record".to_string(),
            optional_resource_id: handle.to_string(),
            optional_resource_id_prefix: String::new(),
            optional_relation: "public_view".to_string(),
            optional_subject_filter: Some(SubjectFilter {
                subject_type: "user".to_string(),
                optional_subject_id: "*".to_string(),
                optional_relation: None,
            }),
        };
        let req = ReadRelationshipsRequest {
            consistency: consistency_requirement(true),
            relationship_filter: Some(filter),
            optional_limit: 1,
            optional_cursor: None,
        };

        let mut inner = self.inner.clone();
        let mut stream = inner
            .read_relationships(req)
            .await
            .context("SpiceDB ReadRelationships (public_view) failed")?;

        match stream.message().await {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(anyhow::anyhow!(
                "SpiceDB ReadRelationships stream error: {e}"
            )),
        }
    }

    /// Enumerate handles whose `public_view@user:*` tuple exists —
    /// i.e. every owner who has flipped the public-view toggle.
    /// Backs Piece 3's `/v1/discover/profiles` endpoint.
    ///
    /// We do NOT use `LookupResources` here: SpiceDB rejects a
    /// wildcard subject with `InvalidArgument: cannot perform lookup
    /// resources on wildcard`, mirroring the constraint on
    /// `CheckPermission` (see [`Self::has_public_view`]). Since the
    /// public-view toggle is mediated by exactly one relation, a
    /// `ReadRelationships` scan over `stats_record:*#public_view@user:*`
    /// is both legal and a tighter answer to the same question.
    ///
    /// `optional_limit` is forwarded to SpiceDB so a runaway listing
    /// can't open-loop the gRPC stream. Callers pass a sane cap; the
    /// discover route passes 1000 today (see `MAX_PUBLIC_PROFILES_LOOKUP`
    /// in `discover_routes.rs`). Zero asks SpiceDB for everything.
    ///
    /// Stream consumed eagerly into a Vec — the discover handler needs
    /// the full set up front to intersect against the SQL `users` row
    /// before applying its own pagination cursor.
    pub async fn list_public_profile_handles(&self, optional_limit: u32) -> Result<Vec<String>> {
        let filter = RelationshipFilter {
            resource_type: "stats_record".to_string(),
            optional_resource_id: String::new(),
            optional_resource_id_prefix: String::new(),
            optional_relation: "public_view".to_string(),
            optional_subject_filter: Some(SubjectFilter {
                subject_type: "user".to_string(),
                optional_subject_id: "*".to_string(),
                optional_relation: None,
            }),
        };
        let req = ReadRelationshipsRequest {
            consistency: None,
            relationship_filter: Some(filter),
            optional_limit,
            optional_cursor: None,
        };

        let mut inner = self.inner.clone();
        let mut stream = inner
            .read_relationships(req)
            .await
            .context("SpiceDB ReadRelationships (public_view scan) failed")?;

        let mut out: Vec<String> = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(msg)) => {
                    if let Some(rel) = msg.relationship {
                        if let Some(resource) = rel.resource {
                            if !resource.object_id.is_empty() {
                                out.push(resource.object_id);
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "SpiceDB ReadRelationships stream error: {e}"
                    ));
                }
            }
        }
        Ok(out)
    }

    /// Grant `recipient_handle` read access to `owner_handle`'s
    /// stats_record. Writes
    /// `stats_record:<owner>#share_with_user@user:<recipient>`.
    pub async fn write_share_with_user(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
    ) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.update_relationship(
            "stats_record",
            owner_handle,
            "share_with_user",
            "user",
            recipient_handle,
        );
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (share_with_user) failed")?;
        Ok(())
    }

    /// Revoke a previously-granted user share. Idempotent — DELETE on
    /// a non-existent relationship is not an error in SpiceDB.
    pub async fn delete_share_with_user(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
    ) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.delete_relationship(
            "stats_record",
            owner_handle,
            "share_with_user",
            "user",
            recipient_handle,
        );
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (delete share_with_user) failed")?;
        Ok(())
    }

    /// List the recipient handles that `owner_handle` has shared their
    /// stats_record with. Streams `ReadRelationships` and collects the
    /// `subject.object.object_id` of every match.
    pub async fn list_share_with_user(&self, owner_handle: &str) -> Result<Vec<String>> {
        // Disambiguate `new` between the four builder traits implemented
        // for `ReadRelationshipsRequest`.
        let mut req: ReadRelationshipsRequest =
            <ReadRelationshipsRequest as ReadRelationshipsRequestBuilder>::new();
        // Filter: stats_record:<owner>#share_with_user@user:*
        <ReadRelationshipsRequest as RelationshipFilterBuilder>::resource_type(
            &mut req,
            "stats_record",
        );
        <ReadRelationshipsRequest as RelationshipFilterBuilder>::resource_id(
            &mut req,
            owner_handle,
        );
        <ReadRelationshipsRequest as RelationshipFilterBuilder>::relation(
            &mut req,
            "share_with_user",
        );
        // Restrict to user subjects (the schema only allows user here,
        // but being explicit lets us drop wildcard rows if they ever
        // appear by accident).
        let subj = SubjectFilter::new("user");
        req.relationship_filter
            .get_or_insert_with(Default::default)
            .optional_subject_filter = Some(subj);

        let mut inner = self.inner.clone();
        let mut stream = inner
            .read_relationships(req)
            .await
            .context("SpiceDB ReadRelationships (share_with_user) failed")?;

        let mut out: Vec<String> = Vec::new();
        // tonic's Streaming exposes `message()` which returns
        // `Result<Option<T>, Status>` — read until the server signals
        // end-of-stream (None).
        loop {
            match stream.message().await {
                Ok(Some(msg)) => {
                    if let Some(rel) = msg.relationship {
                        if let Some(subj) = rel.subject {
                            if let Some(obj) = subj.object {
                                // Skip the wildcard sentinel if any
                                // crept in — we only return concrete
                                // recipients.
                                if obj.object_id != "*" {
                                    out.push(obj.object_id);
                                }
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "SpiceDB ReadRelationships stream error: {e}"
                    ));
                }
            }
        }
        Ok(out)
    }

    /// Inbound shares: list the owner-handles whose `stats_record`
    /// has a `share_with_user@user:<recipient_handle>` row. This is
    /// the mirror of `list_share_with_user` — there we pin the
    /// resource and walk subjects; here we pin the subject and walk
    /// resources, so SpiceDB returns the owners who have granted the
    /// caller read access.
    ///
    /// The builder traits used for outbound queries only set resource
    /// fields, so we build the `RelationshipFilter` by hand here to
    /// populate the subject-id slot (matches the pattern in
    /// `delete_org_member_all_roles`).
    pub async fn list_shared_with_me(&self, recipient_handle: &str) -> Result<Vec<String>> {
        let filter = RelationshipFilter {
            resource_type: "stats_record".to_string(),
            // Empty = no resource_id filter; we want every owner who
            // has shared with this user.
            optional_resource_id: String::new(),
            optional_resource_id_prefix: String::new(),
            optional_relation: "share_with_user".to_string(),
            optional_subject_filter: Some(SubjectFilter {
                subject_type: "user".to_string(),
                optional_subject_id: recipient_handle.to_string(),
                optional_relation: None,
            }),
        };
        let req = ReadRelationshipsRequest {
            consistency: None,
            relationship_filter: Some(filter),
            optional_limit: 0,
            optional_cursor: None,
        };

        let mut inner = self.inner.clone();
        let mut stream = inner
            .read_relationships(req)
            .await
            .context("SpiceDB ReadRelationships (shared_with_me) failed")?;

        let mut out: Vec<String> = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(msg)) => {
                    if let Some(rel) = msg.relationship {
                        if let Some(resource) = rel.resource {
                            // resource.object_id is the OWNER's handle
                            // — the person sharing with us.
                            if !resource.object_id.is_empty() {
                                out.push(resource.object_id);
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "SpiceDB ReadRelationships stream error: {e}"
                    ));
                }
            }
        }
        Ok(out)
    }

    // -- Org membership / org-share helpers --------------------------
    //
    // Wave-2B layers `organization` + `share_with_org` on top of the
    // sharing surface. The methods below match the existing pattern
    // (single WriteRelationships per call, idempotent semantics) and
    // expose just enough shape for `org_routes` and the extended
    // `sharing_routes::*_org` handlers. Bulk cleanup-on-delete uses
    // SpiceDB's `DeleteRelationships` so we can wipe every member
    // row in a single round trip.

    /// Write `organization:<slug>#<role>@user:<handle>`. `role` must
    /// be one of `owner`, `admin`, `member`; the route layer
    /// validates that before calling. Idempotent (TOUCH semantics).
    pub async fn write_org_role(&self, slug: &str, handle: &str, role: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.update_relationship("organization", slug, role, "user", handle);
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (write_org_role) failed")?;
        Ok(())
    }

    /// Delete the relationship for a single role. Idempotent — DELETE
    /// on a non-existent relationship is not an error in SpiceDB.
    /// Kept available for a future "demote member" endpoint that
    /// targets one role at a time; the current `remove_member`
    /// handler uses [`Self::delete_org_member_all_roles`].
    #[allow(dead_code)]
    pub async fn delete_org_role(&self, slug: &str, handle: &str, role: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.delete_relationship("organization", slug, role, "user", handle);
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (delete_org_role) failed")?;
        Ok(())
    }

    /// Delete every org-membership relationship for `handle` across
    /// all three roles in a single bulk-delete RPC.
    ///
    /// We don't have a "user across all relations" filter that
    /// matches SpiceDB's `RelationshipFilter` shape (each call pins a
    /// single relation), so this fans out into three sequential
    /// `DeleteRelationships` calls — one per role. SpiceDB treats
    /// "no rows matched" as success, so we don't need to know which
    /// role(s) the user actually held.
    pub async fn delete_org_member_all_roles(&self, slug: &str, handle: &str) -> Result<()> {
        for role in ["owner", "admin", "member"] {
            let subject_filter = SubjectFilter {
                subject_type: "user".to_string(),
                optional_subject_id: handle.to_string(),
                optional_relation: None,
            };
            let filter = RelationshipFilter {
                resource_type: "organization".to_string(),
                optional_resource_id: slug.to_string(),
                optional_resource_id_prefix: String::new(),
                optional_relation: role.to_string(),
                optional_subject_filter: Some(subject_filter),
            };
            let req = DeleteRelationshipsRequest {
                relationship_filter: Some(filter),
                optional_preconditions: Vec::new(),
                optional_limit: 0,
                optional_allow_partial_deletions: false,
            };
            let mut inner = self.inner.clone();
            inner.delete_relationships(req).await.with_context(|| {
                format!("SpiceDB DeleteRelationships (org member, role={role}) failed")
            })?;
        }
        Ok(())
    }

    /// Bulk-delete every org-membership row + every share-with-org
    /// pointer aimed at this slug. Best-effort: if any of the calls
    /// fails the caller still proceeds with the Postgres delete.
    /// Logs (callers do) but does not propagate the underlying error
    /// shape; returns Ok if every RPC came back clean.
    pub async fn delete_all_org_relationships(&self, slug: &str) -> Result<()> {
        // 1. Wipe every membership row regardless of role.
        for role in ["owner", "admin", "member"] {
            let filter = RelationshipFilter {
                resource_type: "organization".to_string(),
                optional_resource_id: slug.to_string(),
                optional_resource_id_prefix: String::new(),
                optional_relation: role.to_string(),
                optional_subject_filter: None,
            };
            let req = DeleteRelationshipsRequest {
                relationship_filter: Some(filter),
                optional_preconditions: Vec::new(),
                optional_limit: 0,
                optional_allow_partial_deletions: false,
            };
            let mut inner = self.inner.clone();
            inner.delete_relationships(req).await.with_context(|| {
                format!("SpiceDB DeleteRelationships (cleanup org role={role}) failed")
            })?;
        }
        // 2. Wipe any `stats_record:*#share_with_org@organization:<slug>`
        //    pointer so we don't leave dangling subject references.
        let subject_filter = SubjectFilter {
            subject_type: "organization".to_string(),
            optional_subject_id: slug.to_string(),
            optional_relation: None,
        };
        let filter = RelationshipFilter {
            resource_type: "stats_record".to_string(),
            optional_resource_id: String::new(),
            optional_resource_id_prefix: String::new(),
            optional_relation: "share_with_org".to_string(),
            optional_subject_filter: Some(subject_filter),
        };
        let req = DeleteRelationshipsRequest {
            relationship_filter: Some(filter),
            optional_preconditions: Vec::new(),
            optional_limit: 0,
            optional_allow_partial_deletions: false,
        };
        let mut inner = self.inner.clone();
        inner
            .delete_relationships(req)
            .await
            .context("SpiceDB DeleteRelationships (cleanup share_with_org) failed")?;
        Ok(())
    }

    /// List every member of `organization:<slug>` across all three
    /// roles. Returns `(handle, role)` tuples. Streams
    /// `ReadRelationships` once per role and accumulates the results;
    /// SpiceDB doesn't have a "match any of relations" filter so this
    /// is the cheapest correct shape.
    pub async fn list_org_members(&self, slug: &str) -> Result<Vec<(String, String)>> {
        let mut out: Vec<(String, String)> = Vec::new();
        for role in ["owner", "admin", "member"] {
            let mut req: ReadRelationshipsRequest =
                <ReadRelationshipsRequest as ReadRelationshipsRequestBuilder>::new();
            <ReadRelationshipsRequest as RelationshipFilterBuilder>::resource_type(
                &mut req,
                "organization",
            );
            <ReadRelationshipsRequest as RelationshipFilterBuilder>::resource_id(&mut req, slug);
            <ReadRelationshipsRequest as RelationshipFilterBuilder>::relation(&mut req, role);
            let subj = SubjectFilter::new("user");
            req.relationship_filter
                .get_or_insert_with(Default::default)
                .optional_subject_filter = Some(subj);

            let mut inner = self.inner.clone();
            let mut stream = inner.read_relationships(req).await.with_context(|| {
                format!("SpiceDB ReadRelationships (org members, role={role}) failed")
            })?;
            loop {
                match stream.message().await {
                    Ok(Some(msg)) => {
                        if let Some(rel) = msg.relationship {
                            if let Some(subj) = rel.subject {
                                if let Some(obj) = subj.object {
                                    if obj.object_id != "*" {
                                        out.push((obj.object_id, role.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "SpiceDB ReadRelationships stream error (role={role}): {e}"
                        ));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Write `stats_record:<owner>#share_with_org@organization:<slug>`.
    pub async fn write_share_with_org(&self, owner_handle: &str, org_slug: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.update_relationship(
            "stats_record",
            owner_handle,
            "share_with_org",
            "organization",
            org_slug,
        );
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (share_with_org) failed")?;
        Ok(())
    }

    /// Inverse of [`Self::write_share_with_org`]. Idempotent.
    pub async fn delete_share_with_org(&self, owner_handle: &str, org_slug: &str) -> Result<()> {
        let mut req = WriteRelationshipsRequest::default();
        req.delete_relationship(
            "stats_record",
            owner_handle,
            "share_with_org",
            "organization",
            org_slug,
        );
        let mut inner = self.inner.clone();
        inner
            .write_relationships(req)
            .await
            .context("SpiceDB WriteRelationships (delete share_with_org) failed")?;
        Ok(())
    }

    /// List the org slugs `owner_handle` has share-with-org rows for.
    pub async fn list_share_with_org(&self, owner_handle: &str) -> Result<Vec<String>> {
        let mut req: ReadRelationshipsRequest =
            <ReadRelationshipsRequest as ReadRelationshipsRequestBuilder>::new();
        <ReadRelationshipsRequest as RelationshipFilterBuilder>::resource_type(
            &mut req,
            "stats_record",
        );
        <ReadRelationshipsRequest as RelationshipFilterBuilder>::resource_id(
            &mut req,
            owner_handle,
        );
        <ReadRelationshipsRequest as RelationshipFilterBuilder>::relation(
            &mut req,
            "share_with_org",
        );
        let subj = SubjectFilter::new("organization");
        req.relationship_filter
            .get_or_insert_with(Default::default)
            .optional_subject_filter = Some(subj);

        let mut inner = self.inner.clone();
        let mut stream = inner
            .read_relationships(req)
            .await
            .context("SpiceDB ReadRelationships (share_with_org) failed")?;
        let mut out: Vec<String> = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(msg)) => {
                    if let Some(rel) = msg.relationship {
                        if let Some(subj) = rel.subject {
                            if let Some(obj) = subj.object {
                                if obj.object_id != "*" {
                                    out.push(obj.object_id);
                                }
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "SpiceDB ReadRelationships stream error (share_with_org): {e}"
                    ));
                }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// PublicAccessChecker trait
// ---------------------------------------------------------------------------

/// Abstraction over the SpiceDB public-visibility check used by the
/// `/v1/public/:handle/*` handlers.
///
/// Models only the single call site that matters: `check_permission` with
/// resource `stats_record:<handle>`, permission `view`, subject `user:*`.
/// Write paths, pings, and org-member queries stay on [`SpicedbClient`]
/// directly and are intentionally outside this trait's surface.
///
/// Production: implemented by [`SpicedbClient`] below.
/// Tests: implemented by [`test_support::StubAccessChecker`].
#[async_trait]
pub trait PublicAccessChecker: Send + Sync + 'static {
    /// Returns `Ok(true)` if the profile is publicly viewable,
    /// `Ok(false)` if the permission is denied, or `Err(_)` if SpiceDB
    /// is unreachable (callers map this to 503).
    async fn check_public_access(&self, handle: &str) -> anyhow::Result<bool>;
}

#[async_trait]
impl PublicAccessChecker for SpicedbClient {
    async fn check_public_access(&self, handle: &str) -> anyhow::Result<bool> {
        // SpiceDB rejects `CheckPermission` with a wildcard subject
        // (`InvalidArgument: cannot perform check on wildcard subject`),
        // which is what `user:*` looked up via CheckPermission ends up
        // doing — the production manifestation is a `CheckPermission
        // RPC failed` warn line on every public-profile read. Mirror
        // the same fix the visibility + discover routes already use:
        // probe the `public_view@user:*` tuple directly with
        // `ReadRelationships` (which permits wildcard subjects). The
        // result is identical to the legacy CheckPermission semantics
        // — the tuple's existence IS the truth about whether the
        // profile is public.
        //
        // No `.inspect_err` here — callers log the failure with their own
        // handle/request context. Logging in both places produced double
        // warnings on every SpiceDB outage (caught by P4 of the
        // PR #59-followup review team).
        self.has_public_view(handle).await
    }
}

/// Test doubles for [`PublicAccessChecker`].
///
/// Gated behind `#[cfg(test)]` so no mock code ships to production.
/// Route-layer unit tests import [`test_support::StubAccessChecker`]
/// directly to exercise the SpiceDB-denied (404) and SpiceDB-allowed (200)
/// paths without a live sidecar.
#[cfg(test)]
pub mod test_support {
    use super::*;

    /// Mock that always returns a fixed `Ok(bool)` or a simulated error.
    ///
    /// Construct via [`StubAccessChecker::allowed`],
    /// [`StubAccessChecker::denied`], or [`StubAccessChecker::failing`].
    pub struct StubAccessChecker {
        allowed: Option<bool>, // None => simulate an RPC error
    }

    impl StubAccessChecker {
        /// Always grants public access.
        pub fn allowed() -> Self {
            Self {
                allowed: Some(true),
            }
        }
        /// Always denies public access (profile not public → 404).
        pub fn denied() -> Self {
            Self {
                allowed: Some(false),
            }
        }
        /// Simulates a SpiceDB RPC failure (→ 503).
        pub fn failing() -> Self {
            Self { allowed: None }
        }
    }

    #[async_trait]
    impl PublicAccessChecker for StubAccessChecker {
        async fn check_public_access(&self, _handle: &str) -> anyhow::Result<bool> {
            match self.allowed {
                Some(v) => Ok(v),
                None => Err(anyhow::anyhow!("simulated spicedb outage")),
            }
        }
    }
}

impl std::fmt::Debug for SpicedbClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpicedbClient").finish_non_exhaustive()
    }
}

/// Pure helper: pick the [`Consistency`] requirement for a permission
/// check or relationship read. `None` is encoded as the absent oneof,
/// which the SpiceDB server interprets as `minimize_latency` (the
/// default). `true` requests `fully_consistent`, used on
/// read-back-after-write paths (e.g. [`SpicedbClient::has_public_view`])
/// where a stale follower replica would otherwise misreport the
/// caller's own most-recent mutation.
///
/// Extracted as a free function so the consistency choice is
/// directly testable without a live SpiceDB sidecar.
fn consistency_requirement(fully_consistent: bool) -> Option<Consistency> {
    if fully_consistent {
        Some(Consistency {
            requirement: Some(ConsistencyRequirement::FullyConsistent(true)),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ref_converts_to_grpc_type() {
        let r = ObjectRef::new("stats_record", "alice");
        let g: ObjectReference = r.into();
        assert_eq!(g.object_type, "stats_record");
        assert_eq!(g.object_id, "alice");
    }

    /// Regression: the default check-permission path stays on
    /// `minimize_latency` (encoded as the absent oneof) so high-volume
    /// reads aren't penalised by the strong-consistency dispatch.
    #[test]
    fn default_consistency_is_minimize_latency_via_absent_oneof() {
        assert!(consistency_requirement(false).is_none());
    }

    /// Regression for the "enable public profile then disable is
    /// broken" report. The toggle on /sharing rendered stale because
    /// `GET /v1/me/visibility` was issued against `minimize_latency`,
    /// which can serve a pre-write snapshot from a follower replica
    /// for some bounded window after a SpiceDB write. The handler now
    /// asks for `fully_consistent`; this test asserts the underlying
    /// `Consistency` shape carried on the RPC.
    #[test]
    fn strong_consistency_requests_fully_consistent() {
        let c = consistency_requirement(true).expect("strong path must set consistency");
        match c.requirement {
            Some(ConsistencyRequirement::FullyConsistent(v)) => assert!(v),
            other => panic!("expected FullyConsistent(true), got {other:?}"),
        }
    }
}
