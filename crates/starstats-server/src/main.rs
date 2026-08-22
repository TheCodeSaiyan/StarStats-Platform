//! StarStats API server bootstrap.
//!
//! Wires `/healthz`, `/readyz`, `/metrics` for ops, `/v1/ingest` for
//! the desktop client, `/v1/me/*` for read queries. First-party JWT
//! auth — the server loads or generates an RSA keypair at startup and
//! mints + verifies its own tokens.

use crate::appearance::PostgresAppearanceStore;
use crate::audit::{AuditLog, PostgresAuditLog};
use crate::audit_mirror::MinioMirror;
use crate::auth::{AuthVerifier, ServerKey, TokenIssuer};
use crate::config::Config;
use crate::devices::PostgresDeviceStore;
use crate::discover_routes::PostgresDiscoverStore;
use crate::entity_rollup::PostgresEntityRollupStore;
use crate::hangar_store::PostgresHangarStore;
use crate::kek::Kek;
use crate::magic_link::PostgresMagicLinkStore;
use crate::mail::Mailer;
use crate::orgs::PostgresOrgStore;
use crate::preferences_store::PostgresPreferencesStore;
use crate::profile_store::PostgresProfileStore;
use crate::profile_view_stats::{PostgresProfileViewStatsStore, ProfileViewStatsStore};
use crate::recovery_codes::PostgresRecoveryCodeStore;
use crate::reference_data::{ReferenceCategory, ReferenceClient, ReferenceFetchOutcomeCategory};
use crate::reference_store::ReferenceStore;
use crate::repo::PostgresStore;
use crate::rsi_org_store::PostgresRsiOrgStore;
use crate::share_metadata::PostgresShareMetadataStore;
use crate::share_reports::PostgresShareReportStore;
use crate::spicedb::{PublicAccessChecker, SpicedbClient};
use crate::staff_roles::{PostgresStaffRoleStore, StaffRoleStore};
use crate::telemetry::{init_telemetry, TelemetryHandles};
use crate::users::PostgresUserStore;
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Extension, Router,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

mod account_restrictions;
mod admin_inference_rules;
mod admin_org_routes;
mod admin_parser_health;
mod admin_parser_rules;
mod admin_parser_submissions;
mod admin_reference_routes;
mod admin_restriction_routes;
mod admin_routes;
mod admin_sharing_routes;
mod admin_submission_routes;
mod admin_user_insights;
mod admin_user_routes;
mod api_error;
mod appearance;
mod appearance_routes;
mod audit;
mod audit_mirror;
mod auth;
mod auth_routes;
mod config;
mod contract_entities;
mod contract_routes;
mod contracts;
mod db_metrics;
mod device_routes;
mod devices;
mod discover_routes;
mod enrichment;
mod entity_rollup;
mod event_timeline;
mod facts;
mod facts_routes;
mod facts_store;
mod hangar_routes;
mod hangar_store;
mod health;
mod inference_rules;
mod ingest;
mod kek;
mod location_catalog_cache;
mod location_enrichment;
mod locations;
mod magic_link;
mod magic_link_routes;
mod mail;
mod openapi;
mod orders;
mod org_routes;
mod orgs;
mod parser_def_routes;
pub mod parser_health;
pub mod parser_health_job;
pub mod parser_health_store;
mod parser_rules;
mod parser_submissions;
mod preferences_routes;
mod preferences_store;
pub mod profile_layout;
mod profile_layout_routes;
mod profile_store;
mod profile_view_stats;
mod query;
mod recovery_codes;
mod reference_data;
mod reference_media;
mod reference_resolve;
mod reference_routes;
mod reference_stats;
mod reference_store;
mod reference_vectors;
mod repo;
mod restriction_guard;
mod retention;
mod retention_routes;
mod revolut;
mod revolut_routes;
mod roadmap;
mod rsi_org_routes;
mod rsi_org_store;
mod rsi_profile_routes;
mod rsi_verify;
mod rsi_verify_routes;
mod share_metadata;
mod share_reports;
pub mod share_scopes;
mod share_scopes_routes;
mod sharing_routes;
mod ship_matrix_admin_routes;
mod ship_matrix_config_store;
mod ship_matrix_enrichment;
mod ship_matrix_media_routes;
mod smtp_admin_routes;
mod smtp_config_store;
mod spicedb;
mod staff_roles;
mod stat_reconcile;
mod submission_routes;
mod submissions;
mod supporter_routes;
mod supporters;
mod telemetry;
mod totp;
mod totp_routes;
mod unknown_tag_routes;
pub mod unknown_tags;
mod update_routes;
mod users;
mod validation;
mod waitlist;
mod waitlist_routes;
mod well_known;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let TelemetryHandles {
        prometheus,
        otel_guard,
    } = init_telemetry()?;

    let cfg = Config::from_env()?;

    // Session-level guards applied to every pooled connection. statement_timeout
    // is deliberately generous (60s) so it bounds a runaway query without killing
    // the legitimate background reconcile/retention work that shares this pool;
    // lock_timeout + idle_in_transaction_session_timeout cap contention and stuck
    // transactions (the audit advisory-lock path — see POOL-1 in
    // docs/audit/postgres-performance-review-2026-07-22.md). Values are milliseconds.
    let connect_opts = cfg.database_url.parse::<PgConnectOptions>()?.options([
        ("statement_timeout", "60000"),
        ("lock_timeout", "5000"),
        ("idle_in_transaction_session_timeout", "120000"),
    ]);
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .connect_with(connect_opts)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    // Publish connection-pool gauges to /metrics (pool-exhaustion visibility).
    db_metrics::spawn(pool.clone());

    let store = Arc::new(PostgresStore::new(pool.clone()));
    let users = Arc::new(PostgresUserStore::new(pool.clone()));
    let reference_store = Arc::new(reference_store::PostgresReferenceStore::new(pool.clone()));
    // In-memory caches of the slim `/v1/reference/{category}` listings,
    // peer-group stats, and compare/cohort vectors. Without them, every
    // request re-reads every row's full metadata to rebuild the
    // projection (multi-MB Postgres scan for item + vehicle).
    //
    // TTL must OUTLAST the reconcile cadence (`REFRESH_OK`, 24h below):
    // reference data only changes on the daily reconcile, which re-primes
    // all three caches on success. A TTL shorter than the reconcile
    // interval expires the cache between re-primes, so on a low-traffic
    // deploy nearly every navigation pays a full cold rebuild on the
    // request path (the original 5-min TTL did exactly this — seconds of
    // latency per KB click). 26h keeps the cache warm between daily
    // re-primes; if reconciles fail past the window, a request-path
    // rebuild is the acceptable rare fallback.
    let reference_cache_ttl = Duration::from_secs(26 * 3600);
    let reference_list_cache = Arc::new(reference_routes::ReferenceListCache::new(
        reference_cache_ttl,
    ));
    let reference_stats_cache = Arc::new(reference_stats::ReferenceStatsCache::new(
        reference_cache_ttl,
    ));
    let reference_vectors_cache = Arc::new(reference_vectors::ReferenceVectorsCache::new(
        reference_cache_ttl,
    ));
    let reference_client = Arc::new(reference_data::WikiReferenceClient::new()?);
    // Secondary location-taxonomy enrichment source. Pulls richer
    // tier/subtype/placement metadata from starcitizen.tools and
    // joins onto existing reference_registry rows by slug. Does NOT
    // insert. See `docs/PLAN-LOCATION-TAXONOMY-V2.md`.
    let location_enrichment_client =
        Arc::new(location_enrichment::ToolsWikiEnrichmentClient::new()?);
    // Catalogue snapshot consumed by the ingest classifier. Starts
    // empty; the cron tasks below populate it on first refresh.
    // Failure to load on startup is non-fatal — the classifier
    // degrades cleanly to synthetic+heuristic+fallback paths.
    let location_catalog_cache =
        match location_catalog_cache::LocationCatalogCache::load_from_store(
            reference_store.as_ref(),
        )
        .await
        {
            Ok(c) => {
                tracing::info!("location catalog loaded at startup");
                c
            }
            Err(e) => {
                tracing::warn!(error = %e, "location catalog load failed; using empty snapshot");
                location_catalog_cache::LocationCatalogCache::empty()
            }
        };
    let profiles: Arc<PostgresProfileStore> = Arc::new(PostgresProfileStore::new(pool.clone()));
    // Public-profile view counters. Shared between the public-profile
    // tracking path and the `/v1/me/profile-views` read endpoint via a
    // dyn-cast so a future replacement (e.g. Redis-backed counter) is
    // a single-call swap.
    let profile_view_stats: Arc<dyn ProfileViewStatsStore> =
        Arc::new(PostgresProfileViewStatsStore::new(pool.clone()));
    let orgs: Arc<PostgresOrgStore> = Arc::new(PostgresOrgStore::new(pool.clone()));
    let rsi_orgs: Arc<PostgresRsiOrgStore> = Arc::new(PostgresRsiOrgStore::new(pool.clone()));
    let share_metadata: Arc<PostgresShareMetadataStore> =
        Arc::new(PostgresShareMetadataStore::new(pool.clone()));
    let share_reports: Arc<PostgresShareReportStore> =
        Arc::new(PostgresShareReportStore::new(pool.clone()));
    let health_pool = pool.clone();
    let devices: Arc<PostgresDeviceStore> = Arc::new(PostgresDeviceStore::new(pool.clone()));
    // Piece 3 — public-profile discovery. Store carries the SQL-side
    // filter; the SpiceDB lookup runs in the route handler so the
    // store stays a pure data dependency (no outbound RPC).
    let discover_store: Arc<PostgresDiscoverStore> =
        Arc::new(PostgresDiscoverStore::new(pool.clone()));
    // Cross-session entity rollup — store carries the GROUP BY +
    // per-entity event projection. Same auth posture as
    // `event_timeline` (share_event_timeline grant gate).
    let entity_rollup_store: Arc<PostgresEntityRollupStore> =
        Arc::new(PostgresEntityRollupStore::new(pool.clone()));
    let hangars: Arc<PostgresHangarStore> = Arc::new(PostgresHangarStore::new(pool.clone()));
    let preferences: Arc<PostgresPreferencesStore> =
        Arc::new(PostgresPreferencesStore::new(pool.clone()));
    // Owner-side profile widget layout. NULL column → web falls back to
    // DEFAULT_LAYOUT; non-NULL → stored arrangement returned by
    // GET /v1/users/me/profile-layout.
    let profile_layout_store: Arc<dyn profile_layout::ProfileLayoutStore> = Arc::new(
        profile_layout::PostgresProfileLayoutStore::new(pool.clone()),
    );
    // Per-widget sharing toggles (Plan 3b Option A). Stored in
    // `users.share_scopes` JSONB under the "widgets" sub-key.
    // All fields default to false (private) until the owner opts in.
    let share_scopes_store: Arc<dyn share_scopes::ShareScopesStore> =
        Arc::new(share_scopes::PostgresShareScopesStore::new(pool.clone()));
    // The auth extractor consults this dyn handle on every device-token
    // request to enforce revocation.
    let device_store_dyn: Arc<dyn devices::DeviceStore> = devices.clone();

    // Connect to SpiceDB if configured. Same posture as the OTel
    // exporter: a missing or unreachable sidecar logs a warning and
    // boots in degraded mode rather than failing.
    let spicedb: Arc<Option<SpicedbClient>> = match cfg.spicedb.clone() {
        Some(sc) => match SpicedbClient::connect(sc).await {
            Ok(c) => {
                tracing::info!("SpiceDB client connected");
                Arc::new(Some(c))
            }
            Err(e) => {
                tracing::warn!(error = %e, "SpiceDB connect failed; continuing without authz client");
                Arc::new(None)
            }
        },
        None => {
            tracing::info!("SpiceDB not configured (no preshared key); skipping");
            Arc::new(None)
        }
    };

    // Trait-typed view of the SpiceDB client for routes that only need
    // the public-visibility check. Lets route-layer tests inject a stub
    // implementation (see spicedb::test_support::StubAccessChecker)
    // without standing up a real SpiceDB sidecar.
    //
    // Why a parallel extension rather than retyping the existing
    // `Arc<Option<SpicedbClient>>` to the trait: other handlers
    // (sharing_routes, rsi_profile_routes, discover_routes, hangar
    // writes, etc.) bind the SpicedbClient extension by concrete type
    // and need the full client surface — they can't take a narrower
    // PublicAccessChecker. The two extensions share the same underlying
    // gRPC channel (SpicedbClient clones cheaply); the duplication is
    // cheap and lets the public-read trait stay focused.
    //
    // Outer Arc required: axum Extension<T> requires T: Clone + Send +
    // Sync + 'static, which Arc<Option<...>> satisfies. None means "not
    // configured" — routes map that to 503, matching the existing
    // Arc<Option<SpicedbClient>> semantics.
    let public_access_checker: Arc<Option<Arc<dyn PublicAccessChecker>>> =
        Arc::new(spicedb.as_ref().as_ref().map(|c| {
            let arc: Arc<dyn PublicAccessChecker> = Arc::new(c.clone());
            arc
        }));

    // Connect to MinIO if configured. Same posture as SpiceDB: missing
    // credentials -> skipped; unreachable bucket -> warn-and-degrade.
    // The mirror is plumbed through the audit log only — it is NOT a
    // separate Extension layer because no handler reads it directly.
    let minio_mirror: Arc<Option<MinioMirror>> = match cfg.minio.clone() {
        Some(mc) => match MinioMirror::connect(mc).await {
            Ok(m) => {
                // `connect` only wires the SDK client; surface a clean
                // PutObject path early by pinging on boot. A failing
                // ping doesn't take down the server — `/readyz` will
                // continue to report `minio: fail` until it recovers.
                if let Err(e) = m.ping().await {
                    tracing::warn!(
                        error = %e,
                        "MinIO ping failed at boot; mirror enabled but reporting unhealthy"
                    );
                } else {
                    tracing::info!("MinIO audit mirror connected");
                }
                Arc::new(Some(m))
            }
            Err(e) => {
                tracing::warn!(error = %e, "MinIO connect failed; continuing without audit mirror");
                Arc::new(None)
            }
        },
        None => {
            tracing::info!("MinIO not configured (no access key); skipping audit mirror");
            Arc::new(None)
        }
    };

    // Magic-link + recovery-code stores. Both are thin Postgres
    // wrappers; no external deps to fail at boot. Construct before
    // the audit log because `PostgresAuditLog::new` consumes `pool`.
    let magic_link_store = Arc::new(PostgresMagicLinkStore::new(pool.clone()));
    let recovery_store = Arc::new(PostgresRecoveryCodeStore::new(pool.clone()));
    let submissions_store = Arc::new(submissions::PostgresSubmissionStore::new(pool.clone()));
    // Shared `dyn SubmissionStore` handle so the admin parser-submissions
    // publish handler can promote a shape into the community queue. The
    // community submission routes take a concrete `State<Arc<S>>`, so this
    // Extension is the only place the trait object is layered.
    let submissions_store_dyn: Arc<dyn submissions::SubmissionStore> = submissions_store.clone();
    // Rule-author moderation surface for tray-promoted parser
    // submissions (W6). Stored as a dyn handle so the per-route
    // handlers can pull it off an Extension layer without dragging
    // a State generic through the admin router.
    let admin_parser_submissions_store: Arc<
        dyn admin_parser_submissions::AdminParserSubmissionsStore,
    > = Arc::new(admin_parser_submissions::PostgresAdminParserSubmissionsStore::new(pool.clone()));
    // Source of the DB-backed parser-definition manifest served at
    // GET /v1/parser-definitions (migration 0048).
    let parser_rules_store: Arc<dyn parser_rules::ParserRulesStore> =
        Arc::new(parser_rules::PostgresParserRulesStore::new(pool.clone()));
    // Parser-health findings + run heartbeats (migration 0064). Backs the
    // background detector loop and the /v1/admin/parser-health endpoints.
    // Player Facts (#368): session projection feeding the pure fact engine.
    let facts_store: Arc<dyn facts_store::FactsStore> =
        Arc::new(facts_store::PostgresFactsStore::new(pool.clone()));

    let parser_health_store: Arc<dyn parser_health_store::ParserHealthStore> = Arc::new(
        parser_health_store::PostgresParserHealthStore::new(pool.clone()),
    );

    // Opt-in unknown shell-tag sightings (migration 0065). Feeds the
    // candidate-cause correlation on /admin/parser-health. Metadata only —
    // engine symbol names, never log line bodies.
    let unknown_tag_store: Arc<dyn unknown_tags::UnknownTagStore> =
        Arc::new(unknown_tags::PostgresUnknownTagStore::new(pool.clone()));
    // Source of the DB-backed inference-rule portion of the same
    // manifest (migration 0050), plus the admin publish/list endpoints
    // merged into `admin_router` below.
    let inference_rules_store: Arc<dyn inference_rules::InferenceRulesStore> = Arc::new(
        inference_rules::PostgresInferenceRulesStore::new(pool.clone()),
    );
    let supporter_store = Arc::new(supporters::PostgresSupporterStore::new(pool.clone()));
    let orders_store = Arc::new(orders::PostgresOrderStore::new(pool.clone()));
    // Tier-based data retention. The policy store is read on every
    // sweep tick (and on every admin "list policies" request); the
    // sweep itself runs in a tokio task spawned at the bottom of this
    // function. Tier is derived from supporter_status at sweep time --
    // no parallel `users.tier` column to keep in sync.
    let retention_policy_store: Arc<dyn retention::RetentionPolicyStore> =
        Arc::new(retention::PostgresRetentionPolicyStore::new(pool.clone()));
    // Site-wide staff role store. Read by `get_me` (to surface roles
    // in MeResponse) and by the admin extractors that gate /v1/admin/*.
    // Constructed BEFORE the audit log because the audit constructor
    // moves `pool`.
    let staff_roles_store: Arc<PostgresStaffRoleStore> =
        Arc::new(PostgresStaffRoleStore::new(pool.clone()));

    // Build the audit log with the optional mirror (`None` = no mirror;
    // `Some(...)` wires best-effort PUTs) and spawn its background writer.
    // The same `PostgresAuditLog` instance is shared as both the
    // writer (Arc<dyn AuditLog>) and the reader (Arc<dyn AuditQuery>)
    // — there's only ever one DB pool behind it, so cloning the Arc
    // is cheap and keeps the two trait views in sync without
    // duplicating connection management.
    let mirror_for_audit: Option<Arc<MinioMirror>> = minio_mirror.as_ref().clone().map(Arc::new);
    // `audit_writer` is the drain handle — see the `shutdown_and_drain`
    // call after `axum::serve` returns. Dropping it early would NOT stop
    // the writer, but it would forfeit the flush.
    let (pg_audit, audit_writer) = PostgresAuditLog::new(pool.clone(), mirror_for_audit);
    let pg_audit = Arc::new(pg_audit);
    let audit: Arc<dyn AuditLog> = pg_audit.clone();
    let audit_query: Arc<dyn crate::audit::AuditQuery> = pg_audit;

    // Type-erased handle for the StaffRoleStore extension. The admin
    // extractors look up `Arc<dyn StaffRoleStore>` from request
    // extensions; `get_me` does the same. The concrete `staff_roles_store`
    // (constructed earlier) stays alongside for the bootstrap call below
    // because the function takes `&S: StaffRoleStore` directly.
    let staff_roles_dyn: Arc<dyn StaffRoleStore> = staff_roles_store.clone();

    // Idempotently grant `admin` to every handle in
    // STARSTATS_BOOTSTRAP_ADMIN_HANDLES (comma-separated). Failures
    // inside the bootstrap (handle not found, audit-log write fail)
    // are logged and DO NOT abort startup -- a typo in the env var
    // shouldn't keep the server down.
    if let Err(e) = staff_roles::bootstrap_admins_from_env(
        users.as_ref(),
        staff_roles_store.as_ref(),
        audit.as_ref(),
        "STARSTATS_BOOTSTRAP_ADMIN_HANDLES",
    )
    .await
    {
        tracing::error!(error = ?e, "staff_roles bootstrap returned an error");
    }

    // KEK for envelope encryption at rest (TOTP secrets and SMTP
    // password). Loaded from disk; a missing file fails the boot
    // unless STARSTATS_KEK_AUTOGEN=true (silent re-key would lock out
    // every 2FA user). Moved ahead of the mailer init so the DB SMTP
    // config — which decrypts the password under this key — can feed
    // the initial mailer build.
    let kek = Arc::new(Kek::load_or_generate(
        &cfg.kek.path,
        cfg.kek.autogen_allowed,
    )?);
    tracing::info!(path = %cfg.kek.path.display(), "KEK loaded");

    // Trait import needed for the `.get()` / `.put()` calls on
    // `Arc<PostgresSmtpConfigStore>` immediately below.
    use crate::smtp_config_store::SmtpConfigStore as _;

    // DB-backed SMTP config store. The singleton row is seeded by
    // migration 0020 so `get()` always returns; the `enabled` flag
    // decides whether we honour it.
    let smtp_config_store = Arc::new(smtp_config_store::PostgresSmtpConfigStore::new(
        pool.clone(),
    ));

    // Mail transport precedence:
    //   1. DB row when `enabled = true` — admin-managed via /v1/admin/smtp.
    //   2. Env-based SmtpConfig (existing posture) when DB is disabled
    //      or unreadable.
    //   3. NoopMailer fallback (built into `mail::build_mailer`).
    //
    // The chosen transport is wrapped in `SwappableMailer` so the
    // admin save flow can hot-reload it without restarting the server.
    let initial_mailer: Arc<dyn Mailer> = match smtp_config_store.get(&kek).await {
        Ok(rec) if rec.enabled => {
            tracing::info!(
                host = %rec.host,
                "SMTP: using DB-managed config (admin set enabled = true)"
            );
            mail::build_mailer_from_record(&rec)
        }
        Ok(_) => {
            tracing::info!("SMTP: DB config disabled; falling back to env-based config");
            mail::build_mailer(cfg.smtp.as_ref())
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "SMTP: DB config read failed; falling back to env-based config"
            );
            mail::build_mailer(cfg.smtp.as_ref())
        }
    };
    let mailer_swap = Arc::new(mail::SwappableMailer::new(initial_mailer));
    let mailer: Arc<dyn Mailer> = mailer_swap.clone();

    // HTTP client for the RSI bio scrape. A reqwest build failure
    // here is fatal: there is no degraded mode for "we couldn't
    // configure TLS" — the verify endpoint would return 503 forever.
    let rsi_client: Arc<dyn rsi_verify::RsiClient> = Arc::new(rsi_verify::HttpRsiClient::new()?);

    // Tauri auto-updater config. The struct itself is always
    // constructed (default path); the file at that path may be
    // absent — the handler treats absence as "no update yet" and
    // returns 204 without erroring.
    let updater_cfg = Arc::new(cfg.updater.clone());
    tracing::info!(
        manifest_path = %cfg.updater.manifest_path.display(),
        "updater manifest path configured"
    );

    let server_key = ServerKey::load_or_generate(&cfg.jwt.key_path, cfg.jwt.autogen_allowed)?;
    tracing::info!(kid = %server_key.kid, "server JWT key loaded");

    let key_pem = std::fs::read_to_string(&cfg.jwt.key_path)?;
    let jwks_doc = Arc::new(well_known::JwksDocument::from_server_key(
        &server_key,
        &key_pem,
    )?);
    let discovery_cfg = Arc::new(well_known::DiscoveryConfig {
        issuer: cfg.jwt.issuer.clone(),
    });

    // The issuer mints tokens for /v1/auth/*; the verifier checks
    // incoming bearer tokens on every protected route. Both share
    // the same key.
    let issuer = Arc::new(TokenIssuer::new(
        server_key.clone(),
        cfg.jwt.issuer.clone(),
        cfg.jwt.audience.clone(),
    ));
    let verifier = Arc::new(AuthVerifier::new(
        server_key,
        cfg.jwt.issuer.clone(),
        cfg.jwt.audience.clone(),
    ));

    // Per-feature route builders live alongside their handlers
    // (`{feature}::routes()`); main just composes them. Each builder
    // wires whatever `State<_>` shape its handlers need; the shared
    // `Extension<_>`s (verifier, issuer, audit, …) are layered onto
    // the merged outer router below.
    let auth_router = auth_routes::routes(users.clone());
    // Public-beta signup gate. Reads waitlist_config (migration 0050),
    // whose gate_enabled defaults FALSE — so this is inert until an admin
    // turns it on from /admin/waitlist, and turning it back off is the
    // rollback. No redeploy either way.
    let waitlist_store: Arc<dyn waitlist::WaitlistStore> =
        Arc::new(waitlist::PostgresWaitlistStore::new(pool.clone()));
    let waitlist_router = waitlist_routes::routes();
    // Sitewide appearance defaults (migration 0052). Public GET so the
    // signed-out shell can stamp the default wave speed before any
    // auth exists; admin GET/PUT gated by RequireModerator.
    let appearance_store: Arc<dyn appearance::AppearanceStore> =
        Arc::new(PostgresAppearanceStore::new(pool.clone()));
    // Reference-sync trigger. Capacity 1 with `try_send` at the call
    // site: a sync already queued makes a second request redundant, so
    // an overlapping trigger is reported as "already running" rather
    // than queueing a duplicate four-category re-fetch.
    let (reference_sync_tx, mut reference_sync_rx) = tokio::sync::mpsc::channel::<()>(1);

    let appearance_router = appearance_routes::routes();
    let devices_for_prefs = devices.clone();
    let device_router = device_routes::routes(devices, users.clone(), audit.clone());
    let sharing_router = sharing_routes::routes(users.clone(), orgs.clone(), store.clone());
    let share_metadata_dyn: Arc<dyn crate::share_metadata::ShareMetadataStore> =
        share_metadata.clone();
    let share_reports_dyn: Arc<dyn crate::share_reports::ShareReportStore> = share_reports.clone();
    // Threaded through the request layer so sharing/discover handlers
    // can look up supporter chip info by handle. Same dyn-cast pattern
    // as share_metadata_dyn etc.
    let supporter_store_dyn: Arc<dyn crate::supporters::SupporterStore> = supporter_store.clone();

    // Account restrictions. Read on every restriction-gated request via
    // `RequireUnrestricted`. The guard denies with 503 when this
    // extension is missing rather than letting requests through, so a
    // wiring mistake takes the gated routes down loudly instead of
    // quietly disabling enforcement.
    let account_restrictions_dyn: Arc<dyn crate::account_restrictions::AccountRestrictionStore> =
        Arc::new(crate::account_restrictions::PostgresAccountRestrictionStore::new(pool.clone()));

    // Admin-console per-user insight (device sync state, entry counts,
    // retention context). Read-only; batched by page so the users list
    // doesn't fan out per row.
    let admin_user_insights_dyn: Arc<dyn crate::admin_user_insights::AdminUserInsightsStore> =
        Arc::new(crate::admin_user_insights::PostgresAdminUserInsights::new(
            pool.clone(),
        ));

    // Roadmap pipeline (Phases 1-8). The store is always constructed
    // so the public read API works even without GitHub credentials —
    // the reader + reconciler + writeback only spin up when
    // `cfg.roadmap` is Some.
    let roadmap_store: Arc<roadmap::store::PostgresRoadmapStore> =
        Arc::new(roadmap::store::PostgresRoadmapStore::new(pool.clone()));
    let roadmap_store_dyn: Arc<dyn roadmap::store::RoadmapStore> = roadmap_store.clone();
    let roadmap_reader_dyn: Option<Arc<dyn roadmap::github_graphql::GitHubReader>> =
        cfg.roadmap.as_ref().map(|rc| {
            let creds = roadmap::github_graphql::GitHubAppCreds {
                app_id: rc.gh_app_id.clone(),
                installation_id: rc.gh_app_installation_id.clone(),
                private_key: rc.gh_app_private_key.clone(),
            };
            let client: Arc<roadmap::github_graphql::GitHubGraphQLClient> =
                Arc::new(roadmap::github_graphql::GitHubGraphQLClient::new(creds));
            let r: Arc<dyn roadmap::github_graphql::GitHubReader> = client;
            r
        });

    // Audit v2.1 §C — abuse-signal auto-pause needs a dyn-cast UserStore
    // so the report handler (which is monomorphic over the trait via an
    // Extension, not a State generic like add_share) can stamp
    // shares_paused_until on the owner.
    let users_dyn: Arc<dyn crate::users::UserStore> = users.clone();
    let rsi_router = rsi_verify_routes::routes(users.clone());
    let profile_router =
        rsi_profile_routes::routes(users.clone(), profiles.clone(), profile_view_stats.clone());
    let rsi_orgs_router = rsi_org_routes::routes(users.clone(), rsi_orgs.clone());
    let hangar_router = hangar_routes::routes(hangars);
    // Also exposed app-wide as a dyn Extension so `facts_routes` can read
    // the player's stored timezone. `preferences_routes` keeps taking the
    // concrete store by argument — this adds a reader, it does not change
    // how preferences are written.
    let preferences_dyn: Arc<dyn preferences_store::PreferencesStore> = preferences.clone();
    let preferences_router = preferences_routes::routes(preferences, devices_for_prefs);
    let magic_router = magic_link_routes::routes(users.clone(), magic_link_store);
    let totp_router = totp_routes::routes(users.clone(), recovery_store);
    let org_router = org_routes::routes(orgs.clone(), users.clone());
    let reference_router = reference_routes::routes(
        reference_store.clone(),
        reference_list_cache.clone(),
        reference_stats_cache.clone(),
        reference_vectors_cache.clone(),
    );
    // Ship Matrix admin-managed media kill-switch. The DB row is the
    // source of truth; mirror it into a hot `AtomicBool` the media proxy
    // reads per-request (no per-image DB hit). An admin PUT updates both
    // the DB and this handle, so the toggle is effective immediately.
    use crate::ship_matrix_config_store::ShipMatrixConfigStore as _;
    let ship_matrix_config_store =
        Arc::new(ship_matrix_config_store::PostgresShipMatrixConfigStore::new(pool.clone()));
    let ship_matrix_media_flag = Arc::new(AtomicBool::new(
        match ship_matrix_config_store.get_media_enabled().await {
            Ok(v) => {
                tracing::info!(media_enabled = v, "ship matrix media flag loaded from DB");
                v
            }
            Err(e) => {
                tracing::warn!(error = %e, "ship matrix media flag read failed; defaulting OFF");
                false
            }
        },
    ));
    // Ship Matrix image proxy. Separate router because it needs an HTTP
    // client + the flag handle in its state, unlike the data-only
    // reference router. The 4-deep media path never collides with the
    // reference router's 2-deep `vehicles/:class_name`.
    let ship_matrix_media_store: Arc<dyn reference_store::ReferenceStore> = reference_store.clone();
    let ship_matrix_media_router =
        ship_matrix_media_routes::routes(ship_matrix_media_store, ship_matrix_media_flag.clone());
    // Item/weapon KB image proxy. Routes `/:category/:class_name/media/:idx`
    // (5-deep) — no collision with the reference router's 3-deep
    // `/:category/:class_name` shape. Only `item` and `weapon` categories
    // are accepted; all others 404.
    let reference_media_store: Arc<dyn reference_store::ReferenceStore> = reference_store.clone();
    let reference_media_router = reference_media::routes(reference_media_store);
    let submission_router = submission_routes::routes(submissions_store.clone());
    // Admin sub-routers — gated by RequireAdmin / RequireModerator
    // extractors which read `Arc<dyn StaffRoleStore>` from request
    // extensions (layered onto the outer `app` below). admin_routes
    // exposes the extractors + a parameterless skeleton; the submission
    // moderation routes mount under it.
    let admin_router = admin_routes::router()
        .merge(admin_submission_routes::router(submissions_store))
        .merge(admin_user_routes::router(users.clone()))
        .merge(admin_restriction_routes::router(users.clone()))
        .merge(admin_org_routes::router(orgs.clone()))
        .merge(admin_reference_routes::router(reference_store.clone()))
        .merge(admin_reference_routes::sync_router(
            reference_sync_tx.clone(),
        ))
        .merge(admin_sharing_routes::router())
        .merge(admin_parser_submissions::router())
        .merge(admin_parser_rules::router())
        .merge(admin_parser_health::router())
        .merge(unknown_tag_routes::router())
        .merge(facts_routes::router())
        .merge(admin_inference_rules::router())
        .merge(smtp_admin_routes::router(
            smtp_config_store.clone(),
            users.clone(),
        ))
        .merge(ship_matrix_admin_routes::router(
            ship_matrix_config_store.clone(),
            ship_matrix_media_flag.clone(),
        ));
    let supporter_router = supporter_routes::routes(supporter_store.clone());
    let donate_state =
        revolut_routes::build_state(orders_store, supporter_store, cfg.revolut.as_ref());
    let donate_router = revolut_routes::routes(donate_state);

    // Contract ingest + public read surface. sp-ingest pushes contracts
    // to `POST /api/contracts/ingest` (gated by the shared token); the
    // `/api/contracts` reads are public. Self-contained state (store +
    // token), so no new Extension layer on the outer app.
    let contract_store: Arc<dyn contracts::ContractStore> =
        Arc::new(contracts::PostgresContractStore::new(pool.clone()));
    let contract_router = contract_routes::router(contract_store, cfg.ingest_token.clone());

    // Retention sweep needs both the pool and the audit Arc kept alive
    // past the .layer(Extension(audit)) move below. Clone here so the
    // tokio task spawned at the bottom of this function still has
    // working handles.
    let retention_pool_for_sweep = pool.clone();
    let retention_audit_for_sweep = audit.clone();
    let retention_policy_store_for_sweep = retention_policy_store.clone();

    let app = Router::new()
        .route("/healthz", get(health::live))
        .route("/readyz", get(health::ready))
        .route("/metrics", get(health::metrics))
        .route("/.well-known/jwks.json", get(well_known::jwks))
        .route(
            "/.well-known/openid-configuration",
            get(well_known::openid_configuration),
        )
        // Cap ingest payloads at 8 MB. A malicious or misconfigured
        // client could otherwise POST hundreds of MB before the server
        // rejects; axum's default is 2 MB.
        //
        // The tray now batches by BYTES as well as by count
        // (`RemoteSyncConfig::max_batch_bytes`, default 3 MB), so this
        // ceiling should never bind for a well-behaved client — the gap
        // is deliberate headroom for a backlog drain whose raw lines run
        // longer than the tray's per-envelope estimate. A 413 is not
        // fatal (the client bisects and retries) but it re-uploads
        // megabytes per split, so keeping the ceiling clear of the
        // client's target is worth the memory.
        .route(
            "/v1/ingest",
            post(ingest::handle::<PostgresStore>).layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/v1/me/events", get(query::list_events::<PostgresStore>))
        .route(
            "/v1/me/events/:seq/hide",
            post(query::hide_event::<PostgresStore>).delete(query::unhide_event::<PostgresStore>),
        )
        .route("/v1/me/summary", get(query::summary::<PostgresStore>))
        .route("/v1/me/timeline", get(query::timeline::<PostgresStore>))
        .route(
            "/v1/me/metrics/event-types",
            get(query::metrics_event_types::<PostgresStore>),
        )
        .route(
            "/v1/me/metrics/sessions",
            get(query::metrics_sessions::<PostgresStore>),
        )
        .route(
            "/v1/me/ingest-history",
            get(query::ingest_history::<PostgresStore>),
        )
        .route(
            "/v1/me/location/current",
            get(query::location_current::<PostgresStore>),
        )
        .route(
            "/v1/me/location/trace",
            get(query::location_trace::<PostgresStore>),
        )
        .route(
            "/v1/me/location/breakdown",
            get(query::location_breakdown::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/combat",
            get(query::stats_combat::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/travel",
            get(query::stats_travel::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/loadout",
            get(query::stats_loadout::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/stability",
            get(query::stats_stability::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/playtime",
            get(query::stats_playtime::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/records",
            get(query::stats_records::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/lives",
            get(query::stats_lives::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/fleet",
            get(query::stats_fleet::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/docking",
            get(query::stats_docking::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/routes",
            get(query::stats_routes::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/objectives",
            get(query::stats_objectives::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/contracts",
            get(query::stats_contracts::<PostgresStore>),
        )
        // Admin gap-surface: run-observed contract names the catalog
        // is missing, ranked by occurrence. Gated by RequireModerator
        // inside the handler (not a separate admin sub-router) because
        // it needs the same `Arc<PostgresStore>` state as the /v1/me/*
        // routes above — `Extension(staff_roles_dyn)` below covers the
        // whole merged app, so the gate still works here.
        .route(
            "/v1/admin/contracts/gaps",
            get(query::contract_catalog_gaps),
        )
        .route(
            "/v1/me/stats/spend",
            get(query::stats_spend::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/loadout-activity",
            get(query::stats_loadout_activity::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/biggest-trade",
            get(query::stats_biggest_trade::<PostgresStore>),
        )
        .route(
            "/v1/me/stats/locations",
            get(query::stats_locations::<PostgresStore>),
        )
        .route(
            "/v1/me/commerce/recent",
            get(query::commerce_recent::<PostgresStore>),
        )
        .route(
            "/v1/updater/:target/:arch/:current_version",
            get(update_routes::check_for_update),
        )
        .with_state(store)
        .merge(auth_router)
        .merge(device_router)
        .merge(sharing_router)
        .merge(rsi_router)
        .merge(profile_router)
        .merge(rsi_orgs_router)
        .merge(hangar_router)
        .merge(preferences_router)
        .merge(magic_router)
        .merge(totp_router)
        .merge(org_router)
        .merge(reference_router)
        .merge(reference_resolve::router(reference_store.clone()))
        .merge(ship_matrix_media_router)
        .merge(reference_media_router)
        .merge(parser_def_routes::routes(
            parser_rules_store.clone(),
            inference_rules_store.clone(),
        ))
        .merge(parser_submissions::routes(pool.clone()))
        .merge(event_timeline::routes(pool.clone()))
        .merge(entity_rollup::routes(entity_rollup_store))
        .merge(discover_routes::routes(discover_store))
        .merge(contract_router)
        .merge(profile_layout_routes::routes())
        .merge(share_scopes_routes::routes())
        .merge(submission_router)
        .merge(admin_router)
        .merge(retention_routes::router())
        .merge(supporter_router)
        .merge(donate_router)
        // Public-beta waitlist: unauthenticated join + moderator console.
        .merge(waitlist_router)
        // Sitewide appearance defaults: public read + moderator console.
        .merge(appearance_router)
        // Roadmap pipeline (Phases 5-8). Public read API + voting +
        // admin changelog + tray "What's new". The internal webhook +
        // CI-event routes (Phases 3-4) are added below only when
        // `cfg.roadmap` is configured — they hold HMAC-secret state.
        .merge(roadmap::public_routes::router())
        .merge(roadmap::voting_routes::router())
        .merge(roadmap::admin_changelog_routes::router())
        .merge(roadmap::whats_new_routes::router())
        // OpenAPI spec at /openapi.json — purely additive, no auth.
        .merge(openapi::router())
        .layer(Extension(verifier))
        .layer(Extension(issuer))
        .layer(Extension(audit))
        .layer(Extension(device_store_dyn))
        // Location catalog snapshot — read at query time by the journey
        // trace + current-location handlers to classify raw engine keys
        // into friendly names + KB slugs.
        .layer(Extension(location_catalog_cache.clone()))
        .layer(Extension(staff_roles_dyn))
        .layer(Extension(jwks_doc))
        .layer(Extension(discovery_cfg))
        .layer(Extension(prometheus))
        .layer(Extension(health_pool))
        .layer(Extension(spicedb))
        .layer(Extension(public_access_checker))
        .layer(Extension(supporter_store_dyn))
        .layer(Extension(share_metadata_dyn))
        .layer(Extension(admin_user_insights_dyn))
        .layer(Extension(account_restrictions_dyn))
        .layer(Extension(share_reports_dyn))
        .layer(Extension(admin_parser_submissions_store))
        .layer(Extension(submissions_store_dyn))
        .layer(Extension(parser_rules_store))
        .layer(Extension(parser_health_store.clone()))
        .layer(Extension(unknown_tag_store))
        .layer(Extension(facts_store))
        .layer(Extension(preferences_dyn))
        .layer(Extension(inference_rules_store))
        .layer(Extension(users_dyn))
        .layer(Extension(audit_query))
        .layer(Extension(profile_layout_store))
        .layer(Extension(share_scopes_store))
        .layer(Extension(minio_mirror))
        .layer(Extension(mailer))
        .layer(Extension(mailer_swap))
        .layer(Extension(rsi_client))
        .layer(Extension(kek))
        .layer(Extension(updater_cfg))
        .layer(Extension(retention_policy_store.clone()))
        .layer(Extension(roadmap_store_dyn.clone()))
        .layer(Extension(waitlist_store.clone()))
        .layer(Extension(appearance_store.clone()));

    // Mount the internal roadmap webhook + CI-event endpoints only
    // when the pipeline is configured. These routes carry HMAC-keyed
    // state and would 401 every request if mounted without secrets.
    let app =
        if let (Some(rc), Some(reader_dyn)) = (cfg.roadmap.as_ref(), roadmap_reader_dyn.as_ref()) {
            let state = roadmap::routes::RoadmapRoutesState::new(
                roadmap_store_dyn.clone(),
                reader_dyn.clone(),
                rc.gh_webhook_hmac_key.as_bytes().to_vec(),
                rc.ci_event_hmac_key.as_bytes().to_vec(),
                rc.gh_project_id.clone(),
            );
            app.merge(roadmap::routes::router(state))
        } else {
            app
        };

    // ON-DEMAND refresh of community-API-sourced reference data across
    // all four categories (vehicle / weapon / item / location).
    //
    // This used to poll every 24h. It no longer does: the upstream data
    // is reshaped and RETAINED in our own database, so a daily pull was
    // re-fetching data we already own. The registry is now authoritative
    // between syncs and only moves when someone asks it to.
    //
    // The worker still owns the fetch/reconcile/cache-prime sequence —
    // only the trigger changed, from a sleep to a channel. That keeps
    // all the captured state (store, client, three caches) in one place
    // instead of threading it through the admin router.
    //
    // There is deliberately NO sync at startup. A deploy should not
    // re-pull an upstream we already mirror; a brand-new environment is
    // seeded by triggering the endpoint once, explicitly.
    //
    // Best-effort per category: failures log and we keep serving what is
    // already stored — stale data beats no data. `reconcile_category`
    // refuses an empty batch, so a failed fetch cannot clear a category.
    const CATEGORIES: [ReferenceCategory; 4] = [
        ReferenceCategory::Vehicle,
        ReferenceCategory::Weapon,
        ReferenceCategory::Item,
        ReferenceCategory::Location,
    ];
    {
        let reference_store = reference_store.clone();
        let reference_client = reference_client.clone();
        let reference_list_cache = reference_list_cache.clone();
        let reference_stats_cache = reference_stats_cache.clone();
        let reference_vectors_cache = reference_vectors_cache.clone();
        tokio::spawn(async move {
            // Waits for a trigger rather than sleeping. `recv()` returning
            // None means every sender dropped (shutdown) — exit rather
            // than spin.
            while reference_sync_rx.recv().await.is_some() {
                let mut any_failed = false;
                for cat in CATEGORIES {
                    match reference_client.fetch_category(cat).await {
                        ReferenceFetchOutcomeCategory::Entries(entries) => {
                            // Reconcile (not just upsert): rows missing
                            // from the wiki get deleted, so the partial
                            // unique index on `(category, lower(slug))`
                            // can never trip on a stale-row collision.
                            // The wiki is the source of truth.
                            match reference_store
                                .reconcile_category(cat, "wiki_api", &entries)
                                .await
                            {
                                Ok(n) => {
                                    tracing::info!(
                                        category = cat.as_str(),
                                        rows = n,
                                        "reference data refreshed"
                                    );
                                    // Prime the listing cache so users
                                    // don't pay a rebuild on the next
                                    // request after a refresh.
                                    if let Err(e) = reference_list_cache
                                        .rebuild(cat, reference_store.as_ref())
                                        .await
                                    {
                                        tracing::warn!(
                                            error = %e,
                                            category = cat.as_str(),
                                            "reference list cache prime failed; lazy TTL will rebuild"
                                        );
                                    }
                                    if let Err(e) = reference_stats_cache
                                        .rebuild(cat, reference_store.as_ref())
                                        .await
                                    {
                                        tracing::warn!(error = %e, category = ?cat, "reference stats cache prime failed");
                                    }
                                    if let Err(e) = reference_vectors_cache
                                        .rebuild(cat, reference_store.as_ref())
                                        .await
                                    {
                                        tracing::warn!(error = %e, category = ?cat, "reference vectors cache prime failed");
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        error = %e,
                                        category = cat.as_str(),
                                        "reference reconcile failed"
                                    );
                                    any_failed = true;
                                }
                            }
                        }
                        ReferenceFetchOutcomeCategory::UpstreamUnavailable => {
                            tracing::warn!(
                                category = cat.as_str(),
                                "reference upstream unavailable; retaining cached data"
                            );
                            any_failed = true;
                        }
                    }
                }
                if any_failed {
                    tracing::warn!("reference sync finished with failures; retained prior data");
                } else {
                    tracing::info!("reference sync complete");
                }
            }
        });
    }

    // Secondary location-taxonomy enrichment cron. Pulls fresh
    // tier/subtype/placement metadata from starcitizen.tools (via
    // `ToolsWikiEnrichmentClient::fetch_all`), joins onto existing
    // reference_registry rows by slug via
    // `apply_location_taxonomies`, and refreshes the in-memory
    // catalogue snapshot the ingest classifier consults. 60s startup
    // offset so it doesn't fight the daily reference-data refresh
    // above; 24h on success, 1h on failure. Best-effort — a transient
    // wiki outage retains the cached taxonomy.
    {
        use crate::location_enrichment::{LocationEnrichmentClient, LocationEnrichmentOutcome};
        const ENRICHMENT_OFFSET: Duration = Duration::from_secs(60);
        const ENRICHMENT_OK: Duration = Duration::from_secs(86_400);
        const ENRICHMENT_FAIL: Duration = Duration::from_secs(3600);

        let reference_store = reference_store.clone();
        let enrichment_client = location_enrichment_client.clone();
        let catalog_cache = location_catalog_cache.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ENRICHMENT_OFFSET).await;
            loop {
                let next = match enrichment_client.fetch_all().await {
                    LocationEnrichmentOutcome::Entries(map) => {
                        let total = map.len();
                        // Sort + flatten so the update batch is
                        // deterministic across runs — easier to spot
                        // a regression in logs when the same rows
                        // get touched in the same order.
                        let mut items: Vec<(String, _)> = map.into_iter().collect();
                        items.sort_by(|a, b| a.0.cmp(&b.0));
                        match reference_store.apply_location_taxonomies(&items).await {
                            Ok(updated) => {
                                tracing::info!(
                                    enrichment_entries = total,
                                    rows_updated = updated,
                                    skipped_unmatched = total.saturating_sub(updated),
                                    "location enrichment applied"
                                );
                                // Roll the catalog snapshot forward so
                                // the ingest classifier picks up the
                                // newly-enriched rows on the next
                                // event without waiting 24h.
                                if let Err(e) =
                                    catalog_cache.refresh(reference_store.as_ref()).await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "location catalog cache refresh failed (enrichment cron); keeping stale snapshot"
                                    );
                                }
                                ENRICHMENT_OK
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "location enrichment upsert failed"
                                );
                                ENRICHMENT_FAIL
                            }
                        }
                    }
                    LocationEnrichmentOutcome::UpstreamUnavailable => {
                        tracing::warn!(
                            "location enrichment upstream unavailable; retaining cached taxonomy"
                        );
                        ENRICHMENT_FAIL
                    }
                };
                tokio::time::sleep(next).await;
            }
        });
    }

    // RSI Ship Matrix vehicle-enrichment cron — the first generic
    // `EnrichmentSource`. Joins the official Ship Matrix onto vehicle
    // `reference_registry` rows under `metadata.ship_matrix` via the
    // shared `run_enrichment_source` runner. 120s startup offset so it
    // settles after the primary reference refresh + location enrichment;
    // gated by `STARSTATS_SHIP_MATRIX_ENRICHMENT` (default on).
    if cfg.ship_matrix_enrichment {
        match ship_matrix_enrichment::ShipMatrixSource::new() {
            Ok(source) => {
                let store_dyn: Arc<dyn reference_store::ReferenceStore> = reference_store.clone();
                let source_dyn: Arc<dyn enrichment::EnrichmentSource> = Arc::new(source);
                tokio::spawn(enrichment::run_enrichment_source(
                    store_dyn,
                    source_dyn,
                    std::time::Duration::from_secs(120),
                ));
                tracing::info!("ship matrix enrichment cron spawned");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "ship matrix enrichment client init failed; cron not spawned"
                );
            }
        }
    } else {
        tracing::info!("ship matrix enrichment disabled (STARSTATS_SHIP_MATRIX_ENRICHMENT=false)");
    }

    // Tier-based data retention sweep. Runs every 24h (1h backoff on
    // failure) and deletes per-user `events` rows older than the
    // tier's window from `retention_policies`. The job acquires a
    // Postgres advisory lock for the duration of each pass so a
    // multi-replica deployment doesn't double-delete. Best-effort
    // audit emission per docs/ENGINEERING.md: an audit hiccup never poisons
    // the sweep.
    retention::spawn_sweep_loop(
        retention_pool_for_sweep,
        retention_policy_store_for_sweep,
        retention_audit_for_sweep,
    );

    // Nightly defense-in-depth: recompute stat_event_counts from events, correct
    // any drift, and emit a drift metric so a silently-stuck rollup is visible.
    stat_reconcile::spawn_reconcile_loop(pool.clone());

    // Daily parser-health pass: detect an event type that has stopped being
    // produced while users stayed active. Motivated by `vehicle_stowed`
    // silently going dark for three weeks after a Game.log tag rename with
    // nothing going red. Writes a heartbeat row every pass — including clean
    // ones — so "no findings" never looks like "the detector is dead".
    parser_health_job::spawn_health_loop(pool.clone(), parser_health_store);

    // Roadmap pipeline background workers (Phases 3, 6, 7). The
    // reconciler + writeback only run when GitHub credentials are
    // configured; the changelog draft-purge runs unconditionally
    // (operates on local DB state only).
    if let (Some(rc), Some(reader_dyn)) = (cfg.roadmap.as_ref(), roadmap_reader_dyn.as_ref()) {
        let reconciler_interval = std::time::Duration::from_secs(5 * 60);
        // JoinHandles dropped intentionally — the tasks detach and run
        // until process exit; we never join them.
        drop(roadmap::sync::spawn_reconciler(
            roadmap_store_dyn.clone(),
            reader_dyn.clone(),
            rc.gh_project_id.clone(),
            reconciler_interval,
        ));
        drop(roadmap::writeback::spawn_writeback(
            roadmap_store_dyn.clone(),
            reader_dyn.clone(),
            rc.gh_project_id.clone(),
            reconciler_interval,
        ));
        tracing::info!("roadmap reconciler + writeback workers spawned");
    }
    // Spec §8.4 — drafts auto-purge after 30 days. Runs hourly.
    drop(roadmap::changelog::spawn_purge_worker(
        roadmap_store_dyn.clone(),
        30,
        std::time::Duration::from_secs(60 * 60),
    ));

    tracing::info!(bind = %cfg.bind, "starstats-server listening");
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    // `into_make_service_with_connect_info` exposes the peer SocketAddr
    // to extractors. `tower_governor::SmartIpKeyExtractor` consults
    // `X-Forwarded-For`/`Forwarded`/`X-Real-IP` first (Traefik fills
    // these in prod) and falls back to the peer addr for direct hits.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // `serve` has returned, so every in-flight request has finished and
    // nothing can enqueue another audit entry — the point at which the
    // writer's backlog is drainable. Audit appends are fire-and-forget
    // (queued in memory), so without this flush a deploy would discard
    // rows whose `append` already returned Ok to the caller.
    audit_writer.shutdown_and_drain(AUDIT_DRAIN_TIMEOUT).await;

    // Drop the guard explicitly so the OTLP exporter can flush queued
    // spans before the process exits. No-op if OTEL was not configured.
    drop(otel_guard);
    Ok(())
}

/// How long shutdown waits for the audit writer to flush its queue.
///
/// Kept well under the usual 30s container SIGKILL grace so a wedged
/// Postgres delays exit rather than getting the process killed mid-drain.
const AUDIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolves when the process is asked to stop: Ctrl+C anywhere, or
/// SIGTERM on Unix (how the container runtime stops a deployed replica).
///
/// Handing this to `with_graceful_shutdown` makes `serve` stop accepting
/// connections, let in-flight requests finish, and then return.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    // Windows dev boxes have no SIGTERM; Ctrl+C alone drives shutdown there.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("SIGINT received; shutting down"),
        () = terminate => tracing::info!("SIGTERM received; shutting down"),
    }
}
