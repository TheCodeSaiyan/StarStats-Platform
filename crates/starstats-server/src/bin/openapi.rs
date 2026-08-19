//! Tiny helper bin: print the OpenAPI spec to stdout.
//!
//! Used by the TS codegen pipeline so we don't need a running
//! Postgres / SpiceDB / MinIO stack just to dump the spec. Imports
//! the same `openapi::ApiDoc` the live server serves at
//! `/openapi.json`.
//!
//! Usage:
//!   cargo run -p starstats-server --bin starstats-server-openapi > openapi.json

// The bin only consumes `ApiDoc::openapi()`; the rest of the modules
// exist solely so the macro derive sees the annotated handlers and
// schemas. Hence the dead-code blanket — every "unused" function is
// referenced by a different bin (the live server).
#![allow(dead_code)]
#![allow(unused_imports)]

// We re-declare the same module tree as `main.rs` because Cargo bins
// each have their own crate root. The compile cost is the same as
// the main bin's: utoipa's derive macros walk these modules to
// emit the schema.

#[path = "../account_restrictions.rs"]
mod account_restrictions;
#[path = "../admin_inference_rules.rs"]
mod admin_inference_rules;
#[path = "../admin_org_routes.rs"]
mod admin_org_routes;
#[path = "../admin_parser_health.rs"]
mod admin_parser_health;
#[path = "../admin_parser_rules.rs"]
mod admin_parser_rules;
#[path = "../admin_parser_submissions.rs"]
mod admin_parser_submissions;
#[path = "../admin_reference_routes.rs"]
mod admin_reference_routes;
#[path = "../admin_restriction_routes.rs"]
mod admin_restriction_routes;
#[path = "../admin_routes.rs"]
mod admin_routes;
#[path = "../admin_sharing_routes.rs"]
mod admin_sharing_routes;
#[path = "../admin_submission_routes.rs"]
mod admin_submission_routes;
#[path = "../admin_user_insights.rs"]
mod admin_user_insights;
#[path = "../admin_user_routes.rs"]
mod admin_user_routes;
#[path = "../api_error.rs"]
mod api_error;
#[path = "../appearance.rs"]
mod appearance;
#[path = "../appearance_routes.rs"]
mod appearance_routes;
#[path = "../audit.rs"]
mod audit;
#[path = "../audit_mirror.rs"]
mod audit_mirror;
#[path = "../auth.rs"]
mod auth;
#[path = "../auth_routes.rs"]
mod auth_routes;
#[path = "../config.rs"]
mod config;
#[path = "../contract_entities.rs"]
mod contract_entities;
#[path = "../contract_routes.rs"]
mod contract_routes;
#[path = "../contracts.rs"]
mod contracts;
#[path = "../device_routes.rs"]
mod device_routes;
#[path = "../devices.rs"]
mod devices;
#[path = "../discover_routes.rs"]
mod discover_routes;
#[path = "../entity_rollup.rs"]
mod entity_rollup;
#[path = "../event_timeline.rs"]
mod event_timeline;
#[path = "../facts.rs"]
mod facts;
#[path = "../facts_routes.rs"]
mod facts_routes;
#[path = "../facts_store.rs"]
mod facts_store;
#[path = "../hangar_routes.rs"]
mod hangar_routes;
#[path = "../hangar_store.rs"]
mod hangar_store;
#[path = "../health.rs"]
mod health;
#[path = "../inference_rules.rs"]
mod inference_rules;
#[path = "../ingest.rs"]
mod ingest;
#[path = "../kek.rs"]
mod kek;
#[path = "../location_catalog_cache.rs"]
mod location_catalog_cache;
#[path = "../locations.rs"]
mod locations;
#[path = "../magic_link.rs"]
mod magic_link;
#[path = "../magic_link_routes.rs"]
mod magic_link_routes;
#[path = "../mail.rs"]
mod mail;
#[path = "../openapi.rs"]
mod openapi;
#[path = "../orders.rs"]
mod orders;
#[path = "../org_routes.rs"]
mod org_routes;
#[path = "../orgs.rs"]
mod orgs;
#[path = "../parser_def_routes.rs"]
mod parser_def_routes;
#[path = "../parser_health.rs"]
mod parser_health;
#[path = "../parser_health_store.rs"]
mod parser_health_store;
#[path = "../parser_rules.rs"]
mod parser_rules;
#[path = "../parser_submissions.rs"]
mod parser_submissions;
#[path = "../preferences_routes.rs"]
mod preferences_routes;
#[path = "../preferences_store.rs"]
mod preferences_store;
#[path = "../profile_layout.rs"]
mod profile_layout;
#[path = "../profile_layout_routes.rs"]
mod profile_layout_routes;
#[path = "../profile_store.rs"]
mod profile_store;
#[path = "../profile_view_stats.rs"]
mod profile_view_stats;
#[path = "../query.rs"]
mod query;
#[path = "../recovery_codes.rs"]
mod recovery_codes;
#[path = "../reference_data.rs"]
mod reference_data;
#[path = "../reference_media.rs"]
mod reference_media;
#[path = "../reference_resolve.rs"]
mod reference_resolve;
#[path = "../reference_routes.rs"]
mod reference_routes;
#[path = "../reference_stats.rs"]
mod reference_stats;
#[path = "../reference_store.rs"]
mod reference_store;
#[path = "../reference_vectors.rs"]
mod reference_vectors;
#[path = "../repo.rs"]
mod repo;
#[path = "../restriction_guard.rs"]
mod restriction_guard;
#[path = "../retention.rs"]
mod retention;
#[path = "../retention_routes.rs"]
mod retention_routes;
#[path = "../revolut.rs"]
mod revolut;
#[path = "../revolut_routes.rs"]
mod revolut_routes;
#[path = "../unknown_tag_routes.rs"]
mod unknown_tag_routes;
#[path = "../unknown_tags.rs"]
mod unknown_tags;
// Roadmap pipeline module tree (in-flight Phase 1-7). Each phase
// declares its routes via #[utoipa::path] so the openapi bin must
// see them to emit the spec; tests live in the same files, so this
// declaration is also what gates `cargo test` from seeing them.
// `mod.rs` enumerates the submodules; we only need the top-level mod
// here. main.rs adds its own copy when it wires the routes (post-
// Phase 7 follow-up).
#[path = "../roadmap/mod.rs"]
mod roadmap;
#[path = "../rsi_org_routes.rs"]
mod rsi_org_routes;
#[path = "../rsi_org_store.rs"]
mod rsi_org_store;
#[path = "../rsi_profile_routes.rs"]
mod rsi_profile_routes;
#[path = "../rsi_verify.rs"]
mod rsi_verify;
#[path = "../rsi_verify_routes.rs"]
mod rsi_verify_routes;
#[path = "../share_metadata.rs"]
mod share_metadata;
#[path = "../share_reports.rs"]
mod share_reports;
#[path = "../share_scopes.rs"]
mod share_scopes;
#[path = "../share_scopes_routes.rs"]
mod share_scopes_routes;
#[path = "../sharing_routes.rs"]
mod sharing_routes;
#[path = "../ship_matrix_admin_routes.rs"]
mod ship_matrix_admin_routes;
#[path = "../ship_matrix_config_store.rs"]
mod ship_matrix_config_store;
#[path = "../smtp_admin_routes.rs"]
mod smtp_admin_routes;
#[path = "../smtp_config_store.rs"]
mod smtp_config_store;
#[path = "../spicedb.rs"]
mod spicedb;
#[path = "../staff_roles.rs"]
mod staff_roles;
#[path = "../submission_routes.rs"]
mod submission_routes;
#[path = "../submissions.rs"]
mod submissions;
#[path = "../supporter_routes.rs"]
mod supporter_routes;
#[path = "../supporters.rs"]
mod supporters;
#[path = "../telemetry.rs"]
mod telemetry;
#[path = "../totp.rs"]
mod totp;
#[path = "../totp_routes.rs"]
mod totp_routes;
#[path = "../update_routes.rs"]
mod update_routes;
#[path = "../users.rs"]
mod users;
#[path = "../validation.rs"]
mod validation;
#[path = "../waitlist.rs"]
mod waitlist;
#[path = "../waitlist_routes.rs"]
mod waitlist_routes;
#[path = "../well_known.rs"]
mod well_known;

use utoipa::OpenApi;

fn main() {
    let json = openapi::ApiDoc::openapi()
        .to_pretty_json()
        .expect("serialize OpenAPI spec");
    println!("{json}");
}
