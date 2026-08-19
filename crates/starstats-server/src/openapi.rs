//! OpenAPI spec assembly + JSON serving route.
//!
//! `ApiDoc` lists every annotated handler and every type that needs
//! to surface in `components.schemas`. The macro pulls the
//! `#[utoipa::path(...)]` block off each handler and stitches them
//! together; we keep it intentionally manual (no `utoipa-axum`) so a
//! future utoipa minor doesn't yank the rug out — the worst that
//! happens is a stale entry on this list.
//!
//! The spec is exposed at `GET /openapi.json` for clients that want
//! to fetch live; the same spec is dumped to stdout by the
//! `starstats-server-openapi` bin for offline TS codegen.

use crate::admin_inference_rules;
use crate::admin_org_routes;
use crate::admin_parser_health;
use crate::admin_parser_rules;
use crate::admin_parser_submissions;
use crate::admin_reference_routes;
use crate::admin_restriction_routes;
use crate::admin_routes;
use crate::admin_sharing_routes;
use crate::admin_submission_routes;
use crate::admin_user_routes;
use crate::api_error;
use crate::appearance_routes;
use crate::auth_routes;
use crate::contract_entities;
use crate::contract_routes;
use crate::contracts;
use crate::device_routes;
use crate::discover_routes;
use crate::entity_rollup;
use crate::event_timeline;
use crate::facts_routes;
use crate::hangar_routes;
use crate::hangar_store;
use crate::health;
use crate::ingest;
use crate::magic_link_routes;
use crate::org_routes;
use crate::parser_def_routes;
use crate::parser_rules;
use crate::parser_submissions;
use crate::preferences_routes;
use crate::profile_layout;
use crate::profile_layout_routes;
use crate::profile_view_stats;
use crate::query;
use crate::reference_data;
use crate::reference_media;
use crate::reference_resolve;
use crate::reference_routes;
use crate::reference_stats;
use crate::reference_vectors;
use crate::retention_routes;
use crate::revolut_routes;
use crate::roadmap;
use crate::rsi_org_routes;
use crate::rsi_org_store;
use crate::rsi_profile_routes;
use crate::rsi_verify;
use crate::rsi_verify_routes;
use crate::share_scopes;
use crate::share_scopes_routes;
use crate::sharing_routes;
use crate::ship_matrix_admin_routes;
use crate::smtp_admin_routes;
use crate::submission_routes;
use crate::supporter_routes;
use crate::totp_routes;
use crate::unknown_tag_routes;
use crate::update_routes;
use crate::waitlist_routes;
use crate::well_known;
use axum::{response::IntoResponse, routing::get, Router};
use utoipa::{
    openapi::security::{Http, HttpAuthScheme, SecurityScheme},
    Modify, OpenApi,
};

/// Modifier that injects the single `BearerAuth` scheme that protected
/// handlers reference via `security(("BearerAuth" = []))`.
pub struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);
        components.add_security_scheme(
            "BearerAuth",
            SecurityScheme::Http(
                Http::builder()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "StarStats API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Ingest + read API for StarStats. \
                       All `/v1/*` routes besides `/v1/auth/login`, \
                       `/v1/auth/signup`, `/v1/auth/email/verify`, and \
                       `/v1/auth/devices/redeem` require a bearer token.",
    ),
    paths(
        health::live,
        health::ready,
        health::metrics,
        well_known::jwks,
        well_known::openid_configuration,
        ingest::handle,
        query::list_events,
        query::hide_event,
        query::unhide_event,
        query::summary,
        query::timeline,
        query::metrics_event_types,
        query::metrics_sessions,
        query::ingest_history,
        query::location_current,
        query::location_trace,
        query::location_breakdown,
        query::stats_combat,
        query::stats_travel,
        query::stats_loadout,
        query::stats_stability,
        query::stats_playtime,
        query::stats_locations,
        query::stats_lives,
        query::stats_fleet,
        query::stats_docking,
        query::stats_routes,
        query::stats_objectives,
        query::stats_contracts,
        query::stats_spend,
        query::stats_loadout_activity,
        query::commerce_recent,
        auth_routes::signup,
        auth_routes::login,
        auth_routes::verify_email,
        auth_routes::change_password,
        auth_routes::resend_verification,
        admin_user_routes::delete_user_admin,
        admin_restriction_routes::set_restrictions,
        admin_restriction_routes::clear_restrictions,
        auth_routes::delete_account,
        auth_routes::get_me,
        auth_routes::password_reset_start,
        auth_routes::password_reset_complete,
        auth_routes::email_change_start,
        auth_routes::email_change_verify,
        device_routes::start,
        device_routes::list,
        device_routes::revoke,
        device_routes::redeem,
        device_routes::set_sync,
        rsi_verify_routes::start,
        rsi_verify_routes::verify,
        rsi_profile_routes::refresh,
        rsi_profile_routes::me,
        rsi_profile_routes::public_profile,
        rsi_profile_routes::profile_views_me,
        rsi_org_routes::refresh,
        rsi_org_routes::me,
        rsi_org_routes::public_orgs,
        hangar_routes::push,
        hangar_routes::me,
        preferences_routes::get,
        preferences_routes::put,
        profile_layout_routes::get_profile_layout,
        profile_layout_routes::put_profile_layout,
        share_scopes_routes::get_share_scopes,
        share_scopes_routes::put_share_scopes,
        share_scopes_routes::public_share_scopes,
        magic_link_routes::start,
        magic_link_routes::redeem,
        totp_routes::setup,
        totp_routes::confirm,
        totp_routes::disable,
        totp_routes::regenerate_recovery,
        totp_routes::verify_login,
        sharing_routes::set_visibility,
        sharing_routes::get_visibility,
        sharing_routes::add_share,
        sharing_routes::delete_share,
        sharing_routes::list_shares,
        sharing_routes::list_shared_with_me,
        sharing_routes::share_with_org,
        sharing_routes::unshare_with_org,
        sharing_routes::public_summary,
        sharing_routes::public_timeline,
        sharing_routes::friend_summary,
        sharing_routes::friend_timeline,
        sharing_routes::friend_scope,
        sharing_routes::preview_summary,
        sharing_routes::preview_timeline,
        sharing_routes::report_share,
        org_routes::create_org,
        org_routes::list_orgs,
        org_routes::get_org,
        org_routes::delete_org,
        org_routes::add_member,
        org_routes::remove_member,
        update_routes::check_for_update,
        reference_routes::list_vehicles,
        reference_routes::get_vehicle,
        reference_routes::list_entries,
        reference_routes::get_entry,
        reference_routes::get_entry_by_slug,
        reference_routes::get_entry_by_class_name,
        reference_routes::get_category_stats,
        reference_routes::get_compare,
        reference_routes::get_cohort,
        reference_media::proxy_reference_media,
        reference_resolve::resolve_reference_names::<crate::reference_store::PostgresReferenceStore>,
        submission_routes::list,
        submission_routes::create,
        submission_routes::detail,
        submission_routes::vote,
        submission_routes::flag,
        submission_routes::withdraw,
        parser_submissions::submit,
        event_timeline::list_sessions,
        event_timeline::list_session_events,
        entity_rollup::list_entities,
        entity_rollup::entity_history,
        discover_routes::list_discover_profiles,
        contract_routes::ingest,
        contract_routes::list_contracts,
        contract_routes::search_contracts,
        contract_routes::get_contract,
        contract_routes::delete_all_contracts,
        contract_routes::delete_one_contract,
        query::contract_catalog_gaps,
        admin_routes::list_audit,
        admin_sharing_routes::get_overview,
        admin_sharing_routes::get_scope_histogram,
        admin_sharing_routes::get_reports,
        admin_sharing_routes::resolve_report,
        admin_sharing_routes::get_user_sharing_context,
        admin_sharing_routes::get_org_sharing_context,
        admin_submission_routes::accept,
        admin_submission_routes::reject,
        admin_submission_routes::dismiss_flag,
        admin_submission_routes::queue,
        admin_parser_submissions::list_submissions,
        admin_parser_submissions::get_submission,
        admin_parser_submissions::patch_submission,
        admin_parser_submissions::publish_to_community,
        facts_routes::get_facts,
        unknown_tag_routes::report_tags,
        admin_parser_health::get_health,
        admin_parser_health::acknowledge,
        admin_parser_health::resolve,
        admin_parser_rules::publish_rule,
        admin_parser_rules::list_rules,
        admin_inference_rules::publish_rule,
        admin_inference_rules::list_rules,
        admin_inference_rules::list_event_types,
        admin_user_routes::list_users_admin::<crate::users::PostgresUserStore>,
        admin_user_routes::get_user_admin::<crate::users::PostgresUserStore>,
        admin_user_routes::grant_role::<crate::users::PostgresUserStore>,
        admin_user_routes::revoke_role::<crate::users::PostgresUserStore>,
        admin_org_routes::list_orgs_admin::<crate::orgs::PostgresOrgStore>,
        admin_org_routes::get_org_admin::<crate::orgs::PostgresOrgStore>,
        admin_org_routes::delete_org_admin::<crate::orgs::PostgresOrgStore>,
        admin_reference_routes::list_reference_categories::<crate::reference_store::PostgresReferenceStore>,
        admin_reference_routes::list_reference_entries::<crate::reference_store::PostgresReferenceStore>,
        admin_reference_routes::trigger_reference_sync,
        smtp_admin_routes::get_smtp,
        smtp_admin_routes::put_smtp,
        smtp_admin_routes::test_smtp,
        ship_matrix_admin_routes::get_config,
        ship_matrix_admin_routes::put_config,
        supporter_routes::get_me,
        revolut_routes::list_tiers,
        revolut_routes::checkout,
        revolut_routes::webhook,
        retention_routes::list_policies,
        retention_routes::trigger_purge,
        // Public-beta waitlist
        waitlist_routes::join,
        waitlist_routes::status,
        waitlist_routes::admin_list,
        waitlist_routes::admin_admit,
        waitlist_routes::admin_resend,
        waitlist_routes::admin_delete,
        waitlist_routes::admin_get_config,
        waitlist_routes::admin_set_config,
        // Sitewide appearance defaults
        appearance_routes::public_get,
        appearance_routes::admin_get,
        appearance_routes::admin_put,
        roadmap::public_routes::list_roadmap,
        roadmap::public_routes::get_roadmap_item,
        roadmap::public_routes::list_changelog,
        // M-S2: annotated handlers that were absent from the spec (bypassing
        // the OpenAPI drift gate and omitted from the generated TS client).
        roadmap::whats_new_routes::whats_new,
        roadmap::whats_new_routes::mark_seen,
        roadmap::voting_routes::cast_vote,
        roadmap::voting_routes::retract_vote,
        roadmap::voting_routes::subscribe,
        roadmap::voting_routes::unsubscribe,
        roadmap::admin_changelog_routes::list_drafts,
        roadmap::admin_changelog_routes::edit_draft,
        roadmap::admin_changelog_routes::publish_draft,
        parser_def_routes::get_manifest,
    ),
    components(schemas(
        // Shared error envelope (single canonical type for all routes)
        api_error::ApiErrorBody,
        // Health
        health::HealthResponseSchema,
        health::ReadyResponseSchema,
        health::ReadyChecksSchema,
        // Well-known
        well_known::JwksDocument,
        well_known::Jwk,
        well_known::OidcDiscovery,
        // Ingest
        ingest::IngestResponse,
        ingest::IngestBatchSchema,
        ingest::EventEnvelopeSchema,
        ingest::ResolvedLocationSchema,
        ingest::LocationTierSchema,
        ingest::ClassificationSourceSchema,
        ingest::EventMetadataSchema,
        ingest::EntityRefSchema,
        ingest::EntityKindSchema,
        ingest::EventSourceSchema,
        ingest::FieldProvenanceSchema,
        // GameEvent discriminated union + nested enum schemas
        ingest::GameEventSchema,
        ingest::ServerPhaseSchema,
        ingest::QuantumTargetPhaseSchema,
        ingest::SessionEndKindSchema,
        ingest::MissionMarkerKindSchema,
        ingest::LauncherCategorySchema,
        // GameEvent variant payload schemas (29 variants)
        ingest::ProcessInitSchema,
        ingest::LegacyLoginSchema,
        ingest::JoinPuSchema,
        ingest::ChangeServerSchema,
        ingest::SeedSolarSystemSchema,
        ingest::ResolveSpawnSchema,
        ingest::ActorDeathSchema,
        ingest::PlayerDeathSchema,
        ingest::PlayerIncapacitatedSchema,
        ingest::VehicleDestructionSchema,
        ingest::HudNotificationSchema,
        ingest::LocationInventoryRequestedSchema,
        ingest::PlanetTerrainLoadSchema,
        ingest::QuantumTargetSelectedSchema,
        ingest::AttachmentReceivedSchema,
        ingest::VehicleStowedSchema,
        ingest::GameCrashSchema,
        ingest::LauncherActivitySchema,
        ingest::MissionStartSchema,
        ingest::MissionEndSchema,
        ingest::ShopBuyRequestSchema,
        ingest::ShopFlowResponseSchema,
        ingest::CommodityBuyRequestSchema,
        ingest::CommoditySellRequestSchema,
        ingest::SessionEndSchema,
        ingest::RemoteMatchSchema,
        ingest::BurstSummarySchema,
        ingest::LocationChangedSchema,
        ingest::ShopRequestTimedOutSchema,
        // Query
        query::EventsListResponse,
        query::EventDto,
        query::HideToggleResponse,
        query::SummaryResponse,
        query::TypeCount,
        query::TimelineResponse,
        query::TimelineBucket,
        query::EventTypeBreakdownResponse,
        query::EventTypeStatsDto,
        query::SessionsResponse,
        query::SessionDto,
        query::IngestHistoryResponse,
        query::IngestBatchDto,
        query::CurrentLocationResponse,
        crate::locations::ResolvedLocation,
        query::TraceResponse,
        query::TraceEntry,
        query::BreakdownResponse,
        query::BreakdownEntry,
        query::StatsBucket,
        query::CombatStatsResponse,
        query::SpendLifetime,
        query::TravelStatsResponse,
        query::LoadoutStatsResponse,
        query::StabilityStatsResponse,
        query::PlaytimeStatsResponse,
        query::LocationsStatsResponse,
        query::LivesResponse,
        query::LivesWindow,
        query::LifeRow,
        query::FleetResponse,
        query::FleetShipRow,
        query::FleetLifetime,
        query::FleetPrevious,
        query::DockingResponse,
        query::DockKindCounts,
        query::DockSizeCounts,
        query::DockingLifetime,
        query::DockingPrevious,
        query::RoutesResponse,
        query::RouteRow,
        query::RoutesLifetime,
        query::RoutesPrevious,
        query::ObjectivesResponse,
        query::ObjectivesLifetime,
        query::ObjectivesPrevious,
        query::ContractsResponse,
        query::ContractRunRow,
        query::ContractStepRow,
        query::ContractGapDto,
        query::ContractCatalogGapsResponse,
        query::SpendResponse,
        query::SpendLifetime,
        query::SpendPrevious,
        query::LoadoutActivityResponse,
        query::LoadoutItemRow,
        query::CommerceRecentResponse,
        query::CommerceTransactionDto,
        // Submissions
        submission_routes::SubmissionDto,
        submission_routes::ListResponse,
        submission_routes::CreateSubmissionRequest,
        submission_routes::CreateSubmissionResponse,
        submission_routes::VoteRequest,
        submission_routes::VoteResponse,
        submission_routes::FlagRequest,
        submission_routes::FlagResponse,
        submission_routes::WithdrawResponse,
        // Per-event timeline (sharing-grant gated)
        event_timeline::SessionSummary,
        event_timeline::SessionsListResponse,
        event_timeline::SessionEventSchema,
        event_timeline::SessionEventsResponseSchema,
        // Discover (Piece 3 of public-profile UX)
        discover_routes::DiscoverProfile,
        discover_routes::DiscoverProfilesResponse,
        // Contract ingest (sp-ingest push) + public read surface
        contracts::PublishBundleReq,
        contracts::AdminReviewPacketReq,
        contracts::ExtractionReq,
        contracts::ExtractedContractReq,
        contracts::ExtractedStepReq,
        contracts::RewardReq,
        contracts::AdditionalRewardReq,
        contracts::StepEntityReq,
        contract_entities::NameResolution,
        contract_entities::EntityRow,
        contract_routes::ResolveNamesResponse,
        contracts::FeeReq,
        contracts::TimeframeReq,
        contracts::AttributeReq,
        contracts::UpdateSuggestionReq,
        contract_routes::IngestAccepted,
        contract_routes::DeleteResult,
        contract_routes::ContractSummary,
        contract_routes::ContractListResponse,
        contract_routes::ContractDetail,
        contract_routes::ContractStepView,
        // Cross-session entity rollup (share_event_timeline gated)
        entity_rollup::EntitySummary,
        entity_rollup::EntitiesListResponse,
        entity_rollup::EntitySessionBucket,
        entity_rollup::EntityHistoryResponseSchema,
        // Parser submissions (tray-promoted unknown-line submissions)
        parser_submissions::ContextExampleSchema,
        parser_submissions::ParserSubmissionSchema,
        parser_submissions::ParserSubmissionBatchSchema,
        parser_submissions::ParserSubmissionResponseSchema,
        // Admin submission moderation
        admin_routes::AuditEntryDto,
        admin_routes::AuditListResponse,
        // Admin sharing overview
        admin_sharing_routes::AdminSharingOverview,
        admin_sharing_routes::TopGranter,
        admin_sharing_routes::ScopeHistogram,
        admin_sharing_routes::ShareReportRowDto,
        admin_sharing_routes::ShareReportListResponse,
        admin_sharing_routes::ResolveReportRequest,
        admin_sharing_routes::UserShareEdge,
        admin_sharing_routes::UserSharingContext,
        admin_sharing_routes::OrgMemberSharingSlice,
        admin_sharing_routes::OrgSharingContext,
        sharing_routes::ReportShareRequest,
        sharing_routes::ReportShareResponse,
        admin_restriction_routes::AdminRestrictionDto,
        admin_restriction_routes::RestrictionRequest,
        admin_user_routes::AdminDeleteUserRequest,
        admin_user_routes::AdminDeleteUserResponse,
        admin_user_routes::DeleteMode,
        admin_user_routes::AdminUserDto,
        admin_user_routes::AdminUserDetailDto,
        admin_user_routes::AdminUserDeviceDto,
        admin_user_routes::AdminUserEventTypeCountDto,
        admin_user_routes::AdminUserRetentionDto,
        admin_user_routes::AdminUserListResponse,
        admin_user_routes::GrantRoleRequest,
        admin_user_routes::RoleTransitionResponse,
        admin_org_routes::AdminOrgDto,
        admin_org_routes::AdminOrgListResponse,
        admin_org_routes::AdminOrgDeleteResponse,
        admin_reference_routes::AdminReferenceCategoryDto,
        admin_reference_routes::AdminReferenceCategoriesResponse,
        admin_reference_routes::AdminReferenceEntryDto,
        admin_reference_routes::AdminReferenceEntriesResponse,
        admin_reference_routes::ReferenceSyncResponse,
        admin_submission_routes::SubmissionTransitionResponse,
        admin_submission_routes::RejectRequest,
        admin_submission_routes::AdminQueueResponse,
        // Admin parser-submissions moderation (W6)
        admin_parser_submissions::AdminSubmissionSummary,
        admin_parser_submissions::AdminSubmissionsListResponse,
        admin_parser_submissions::AdminSubmissionDetail,
        admin_parser_submissions::AdminSubmissionPatch,
        admin_parser_submissions::ContextExampleSchema,
        admin_parser_submissions::ParserSubmissionPayloadSchema,
        admin_parser_submissions::PublishCommunityRequest,
        admin_parser_submissions::PublishCommunityResponse,
        // Admin parser-rules authoring (rule-authoring UI)
        admin_parser_health::ParserHealthResponse,
        admin_parser_health::FindingView,
        unknown_tag_routes::ReportTagsRequest,
        unknown_tag_routes::ReportTagsResponse,
        facts_routes::FactsResponse,
        crate::facts::Fact,
        crate::facts::FactEvidence,
        crate::facts::FactScope,
        crate::facts::FactUnit,
        crate::unknown_tags::TagSighting,
        crate::unknown_tags::TagCandidate,
        admin_parser_health::AcknowledgeRequest,
        crate::parser_health::Finding,
        crate::parser_health::Severity,
        crate::parser_health_store::StoredFinding,
        crate::parser_health_store::FindingStatus,
        crate::parser_health_store::HealthRun,
        admin_parser_rules::PublishRuleRequest,
        admin_parser_rules::PublishRuleResponse,
        admin_parser_rules::AdminParserRulesListResponse,
        parser_rules::AdminParserRuleRow,
        // Admin inference-rules authoring (inference-rule publishing UI)
        admin_inference_rules::EventPatternDto,
        admin_inference_rules::EventTemplateDto,
        admin_inference_rules::InferenceRuleDto,
        admin_inference_rules::PublishInferenceRuleRequest,
        admin_inference_rules::PublishInferenceRuleResponse,
        admin_inference_rules::AdminInferenceRuleRow,
        admin_inference_rules::AdminInferenceRulesListResponse,
        admin_inference_rules::EventTypesResponse,
        // Admin SMTP config
        smtp_admin_routes::SmtpConfigResponse,
        smtp_admin_routes::SmtpConfigRequest,
        smtp_admin_routes::TestSendResponse,
        smtp_admin_routes::TestSendRequest,
        ship_matrix_admin_routes::ShipMatrixConfigResponse,
        ship_matrix_admin_routes::ShipMatrixConfigRequest,
        // Supporter (donate) status
        supporter_routes::SupporterStatusDto,
        // Donate / Revolut
        revolut_routes::TierDto,
        revolut_routes::TierListResponse,
        revolut_routes::CheckoutRequest,
        revolut_routes::CheckoutResponse,
        revolut_routes::WebhookAck,
        // Data retention purge (admin surface; sweep runs on a tokio loop)
        retention_routes::RetentionPolicyDto,
        retention_routes::RetentionPoliciesResponse,
        retention_routes::RetentionPurgeResponse,
        // Auth
        auth_routes::SignupRequest,
        auth_routes::LoginRequest,
        auth_routes::AuthResponse,
        auth_routes::VerifyEmailRequest,
        auth_routes::VerifyEmailResponse,
        auth_routes::ChangePasswordRequest,
        auth_routes::ChangePasswordResponse,
        auth_routes::ResendVerificationResponse,
        auth_routes::DeleteAccountRequest,
        auth_routes::DeleteAccountResponse,
        auth_routes::MeResponse,
        auth_routes::PasswordResetStartRequest,
        auth_routes::PasswordResetStartResponse,
        auth_routes::PasswordResetCompleteRequest,
        auth_routes::PasswordResetCompleteResponse,
        auth_routes::EmailChangeStartRequest,
        auth_routes::EmailChangeStartResponse,
        auth_routes::EmailChangeVerifyRequest,
        auth_routes::EmailChangeVerifyResponse,
        // Devices
        device_routes::StartRequest,
        device_routes::StartResponse,
        device_routes::RedeemRequest,
        device_routes::RedeemResponse,
        device_routes::DeviceListResponse,
        device_routes::DeviceDto,
        device_routes::SetSyncRequest,
        device_routes::SetSyncResponse,
        // RSI verify
        rsi_verify_routes::RsiStartResponse,
        rsi_verify_routes::RsiVerifyResponse,
        // RSI profile
        rsi_profile_routes::ProfileResponse,
        rsi_verify::Badge,
        // Profile-view counters (Piece 2 of public-profile UX)
        profile_view_stats::ProfileViewSource,
        profile_view_stats::ProfileViewDay,
        profile_view_stats::ProfileViewTotals,
        profile_view_stats::ProfileViewStats,
        // RSI orgs
        rsi_verify::RsiOrg,
        rsi_org_store::RsiOrgsSnapshot,
        // Hangar
        hangar_store::HangarSnapshot,
        hangar_routes::HangarPushRequestSchema,
        hangar_routes::HangarShipSchema,
        // Preferences
        preferences_routes::UserPreferencesSchema,
        preferences_routes::RemoteSyncPrefsSchema,
        // Profile layout
        profile_layout::LayoutEntry,
        profile_layout::WidgetSize,
        profile_layout::LayoutSurface,
        profile_layout_routes::ProfileLayoutResponse,
        profile_layout_routes::UpdateProfileLayoutRequest,
        // Share scopes (Plan 3b Option A — per-widget visibility toggles)
        share_scopes::WidgetShareScopes,
        // Magic link
        magic_link_routes::MagicLinkStartRequest,
        magic_link_routes::MagicLinkStartResponse,
        magic_link_routes::MagicLinkRedeemRequest,
        // TOTP
        totp_routes::TotpSetupResponse,
        totp_routes::TotpConfirmRequest,
        totp_routes::TotpConfirmResponse,
        totp_routes::TotpDisableRequest,
        totp_routes::TotpDisableResponse,
        totp_routes::RegenerateRecoveryRequest,
        totp_routes::RegenerateRecoveryResponse,
        totp_routes::VerifyLoginRequest,
        // Sharing
        sharing_routes::VisibilityRequest,
        sharing_routes::VisibilityResponse,
        sharing_routes::ShareScope,
        sharing_routes::ShareRequest,
        sharing_routes::ShareResponse,
        sharing_routes::RevokeShareResponse,
        sharing_routes::ShareEntry,
        sharing_routes::OrgShareEntry,
        sharing_routes::ShareOrgRequest,
        sharing_routes::ShareOrgResponse,
        sharing_routes::RevokeOrgShareResponse,
        sharing_routes::ListSharesResponse,
        sharing_routes::SharedWithMeEntry,
        sharing_routes::ListSharedWithMeResponse,
        sharing_routes::PublicSummaryResponse,
        sharing_routes::PublicSupporterInfo,
        sharing_routes::PublicTypeCount,
        sharing_routes::PublicTimelineResponse,
        sharing_routes::PublicTimelineBucket,
        // Orgs
        org_routes::CreateOrgRequest,
        org_routes::CreateOrgResponse,
        org_routes::OrgDto,
        org_routes::ListOrgsResponse,
        org_routes::OrgMemberDto,
        org_routes::GetOrgResponse,
        org_routes::DeleteOrgResponse,
        org_routes::AddMemberRequest,
        org_routes::AddMemberResponse,
        org_routes::RemoveMemberResponse,
        // Updater
        update_routes::UpdateManifest,
        update_routes::PlatformBundle,
        // Public-beta waitlist
        waitlist_routes::WaitlistJoinRequest,
        waitlist_routes::WaitlistJoinResponse,
        waitlist_routes::WaitlistStatusResponse,
        waitlist_routes::WaitlistEntryApi,
        waitlist_routes::AdmitRequest,
        waitlist_routes::AdmitResponse,
        waitlist_routes::ResendResponse,
        waitlist_routes::WaitlistDeleteResponse,
        waitlist_routes::WaitlistConfigApi,
        // Sitewide appearance defaults
        appearance_routes::AppearanceConfigApi,
        // Reference resolve (batch class-name → rich entry)
        reference_resolve::ResolveRequest,
        reference_resolve::ResolvedEntry,
        reference_resolve::ResolveResponse,
        // Reference data
        reference_data::VehicleReference,
        reference_routes::VehicleListResponse,
        reference_data::ReferenceCategory,
        reference_data::ReferenceEntry,
        reference_routes::ReferenceListResponse,
        reference_routes::ReferenceListEntry,
        reference_routes::ReferenceEntryDetail,
        reference_data::Summary,
        reference_data::VehicleSummary,
        reference_data::WeaponSummary,
        reference_data::ItemSummary,
        reference_data::LocationSummary,
        reference_data::PlacementSchema,
        // Reference stats
        reference_stats::ReferenceStatsResponseSchema,
        reference_stats::QuantilesSchema,
        // Reference vectors (compare + cohort endpoints)
        reference_vectors::CompareEntry,
        reference_vectors::CompareResponse,
        reference_routes::CohortSchema,
        // Roadmap pipeline (Phases 5 + 7 public surface)
        roadmap::public_routes::RoadmapItemPublic,
        roadmap::public_routes::ChannelStatusPublic,
        roadmap::public_routes::RoadmapListResponse,
        roadmap::public_routes::ChangelogEntryPublic,
        roadmap::public_routes::ChangelogResponse,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "Liveness, readiness, metrics"),
        (name = "well-known", description = "JWKS + OIDC discovery"),
        (name = "auth", description = "Email + password account flow"),
        (name = "devices", description = "Device pairing flow"),
        (name = "rsi-verify", description = "RSI handle ownership verification via public bio"),
        (name = "rsi-profile", description = "Public RSI citizen profile snapshots"),
        (name = "rsi-orgs", description = "User's RSI organisation memberships"),
        (name = "hangar", description = "User-owned ship hangar snapshots"),
        (name = "preferences", description = "Per-user UI preferences (theme, etc.)"),
        (name = "profile-layout", description = "Per-user profile widget layout (order, enabled, size)"),
        (name = "share-scopes", description = "Per-widget visitor visibility toggles (Plan 3b Option A)"),
        (name = "totp", description = "TOTP 2FA setup, verification, and recovery codes"),
        (name = "ingest", description = "Client → server event batches"),
        (name = "contracts", description = "sp-ingest contract push + public contract read/search surface"),
        (name = "query", description = "Read-side per-user query API"),
        (name = "sharing", description = "Public visibility + per-user share management"),
        (name = "orgs", description = "Organizations + membership"),
        (name = "updater", description = "Tauri auto-update manifest"),
        (name = "reference", description = "Star Citizen vehicle/item reference data (community-API-sourced)"),
        (name = "supporter", description = "Donate-status surface (read-only)"),
        (name = "donate", description = "Revolut hosted-checkout donate flow"),
        (name = "parser-submissions", description = "Tray-promoted unknown-line submissions for rule-author review"),
        (name = "event-timeline", description = "Per-session per-event timeline (share_event_timeline grant required)"),
        (name = "entity-rollup", description = "Cross-session per-entity rollup (share_event_timeline grant required)"),
        (name = "discover", description = "Public-profile listing for the /discover surface"),
        (name = "admin", description = "Site-wide staff endpoints (moderator/admin role required)"),
        (name = "roadmap", description = "Public roadmap pipeline (items, channel statuses, changelog)"),
        (name = "appearance", description = "Sitewide appearance defaults (theme-switch wave speed)"),
    )
)]
pub struct ApiDoc;

async fn openapi_json() -> impl IntoResponse {
    // `axum::Json` over the spec works, but utoipa's `to_pretty_json`
    // produces stable key ordering which matters for our drift-detection
    // CI step. Wrap in axum's IntoResponse via tuple to set the content
    // type explicitly.
    let body = ApiDoc::openapi()
        .to_json()
        .unwrap_or_else(|_| "{}".to_string());
    ([("content-type", "application/json")], body)
}

/// Returns a router exposing `GET /openapi.json` only. Merge into the
/// main router via `.merge(openapi::router())`.
pub fn router() -> Router {
    Router::new().route("/openapi.json", get(openapi_json))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference sync worker waits on a channel that only
    /// `POST /v1/admin/reference/sync` writes to — there is no daily
    /// poll and no sync at boot. If that path is missing from the
    /// spec it is missing from the generated TS client, so no admin
    /// surface can call it and reference data silently stops
    /// refreshing forever.
    ///
    /// The CI drift check cannot catch this: it compares the spec
    /// against the committed `schema.ts`, and an endpoint absent from
    /// BOTH agrees with itself. Drift detects disagreement, never a
    /// shared omission — so the guard has to be an assertion about
    /// presence, which is what this is.
    ///
    /// Scoped deliberately: this covers the one route whose absence
    /// is silent and unrecoverable, not every route. A handler that
    /// merely 404s is loud enough to find without a test.
    #[test]
    fn reference_sync_trigger_is_documented() {
        let spec = ApiDoc::openapi().to_json().expect("spec serialises");
        assert!(
            spec.contains("/v1/admin/reference/sync"),
            "reference sync path missing from the OpenAPI spec — it will \
             not reach the generated client, leaving the sync worker \
             unreachable. Add the handler to the `paths(...)` list."
        );
        assert!(
            spec.contains("ReferenceSyncResponse"),
            "ReferenceSyncResponse schema missing from the spec — the \
             generated client cannot type the response. Add it to the \
             `components(schemas(...))` list."
        );
    }
}
