//! Roadmap pipeline (spec `docs/ROADMAP-PIPELINE-SPEC.md`).
//!
//! Phase 1 lays down the data layer only: closed-vocabulary enums,
//! row structs, and a `RoadmapStore` trait with Postgres + Memory
//! implementations. No routes, no GraphQL client, no reconciler --
//! those land in phases 2-9 per
//! `docs/ROADMAP-PIPELINE-IMPLEMENTATION-PLAN.md`.
//!
//! Nothing in `main.rs` consumes the trait yet (Phase 3 wires the
//! reconciler + Extension). The blanket `#[allow(dead_code)]` keeps
//! the workspace `-D warnings` clippy gate honest while the module
//! is in flight; the lint flips on naturally as soon as the first
//! route or background task references these symbols.
#![allow(dead_code)]

pub mod admin_changelog_routes;
pub mod changelog;
pub mod events;
pub mod github_graphql;
pub mod internal_changelog_routes;
pub mod mapper;
pub mod models;
pub mod public_routes;
pub mod routes;
pub mod store;
pub mod sync;
pub mod voting_routes;
pub mod whats_new_routes;
pub mod writeback;
