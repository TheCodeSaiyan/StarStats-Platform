/**
 * Widget contract for the `/u/[handle]` profile page.
 *
 * Each section on the profile becomes one Widget. The page reads
 * `users.profile_layout` (NULL-safe — defaults to DEFAULT_LAYOUT),
 * projects against the registry, filters by viewer + sharing + data
 * presence, then renders each surviving entry via `WidgetFrame`.
 *
 * Phase 1+2 scope: NO edit mode. WidgetFrame is a server component
 * with no client-side controls. `size` is read-only from the stored
 * layout. Edit-mode chrome lands in Phase 3.
 */

import type { ReactElement } from 'react';
import type { ShareScope, WidgetShareScopesApi } from '@/lib/api';
import type { RangeId } from '@/lib/range';

export type WidgetId =
  | 'sessions'
  | 'heatmap'
  | 'orgs'
  | 'entities'
  | 'combat_mission'
  | 'economy'
  | 'travel'
  | 'journey'
  | 'records'
  | 'recent_activity'
  | 'hangar'
  | 'loadout'
  | 'lives'
  | 'fleet'
  | 'docking'
  | 'objectives'
  | 'spend'
  | 'routes'
  | 'locations'
  | 'corridors'
  | 'contracts'
  | 'facts';

export type WidgetSize = 'compact' | 'expanded';

/**
 * Per-widget visitor visibility toggles (Plan 3b Option A).
 *
 * Re-exported from the generated API schema so this type stays in sync
 * with the server definition automatically after each `pnpm ... generate`.
 */
export type { WidgetShareScopesApi as WidgetShareScopes } from '@/lib/api';

/** All-false default — used as fallback when the fetch fails. */
export const DEFAULT_SHARE_SCOPES: WidgetShareScopesApi = {
  combat_mission: false,
  economy: false,
  travel: false,
  records: false,
  recent_activity: false,
};

/**
 * Per-render context. Carries the viewer's identity (or `null` for
 * unauthenticated visitors), the owner's handle, and the data needed
 * to check share toggles. Each widget chooses what it needs from this.
 */
export interface ViewerCtx {
  /** The handle whose profile is being rendered. */
  ownerHandle: string;
  /** The signed-in viewer's claimed handle (lower-cased), or null. */
  viewerHandle: string | null;
  /** True iff `viewerHandle === ownerHandle.toLowerCase()`. */
  isOwner: boolean;
  /** Bearer token for downstream API calls. Null for unauthed. */
  token: string | null;
  /**
   * Owner's per-widget sharing toggles (Plan 3b Option A).
   *
   * Fetched once at page render and threaded here so each widget's
   * `isAvailable` check is a synchronous field read rather than a
   * per-widget async API call. Defaults to all-false when the fetch
   * fails — conservative, never over-shares.
   */
  shareScopes: WidgetShareScopesApi;
  /**
   * Visitor's per-recipient ShareScope clamp on this profile (Plan 3b
   * Option B). `null` when no clamp is set — equivalent to
   * pass-through. Owners and unauthed visitors always see `null` here.
   *
   * Widgets that opt in to per-recipient overrides check this in their
   * `isAvailable`: deny when `deny_widgets` lists the widget id, or
   * when `allow_widgets` is set and doesn't include the id. The
   * `widget_allowed_for_scope` semantics in `starstats-server` is the
   * authoritative reference; this is the client-side mirror.
   *
   * No widget currently reads this — the field is plumbed but
   * unconsumed. Per-widget gates land in a follow-up PR.
   */
  recipientScopes: ShareScope | null;
  /**
   * Active global time range (Plan 2). Range-aware widgets re-query
   * for this window; snapshot/lifetime widgets ignore it. Driven by
   * the page's `?range=` param via `parseRange` (default '30d').
   */
  range: RangeId;
}

export interface WidgetDef {
  id: WidgetId;
  defaultSize: WidgetSize;
  /** Human-readable category label (eyebrow). */
  eyebrow: string;
  /** True if this widget re-queries when `ctx.range` changes. Snapshot
   *  / lifetime widgets omit it (treated as false). */
  rangeAware?: boolean;
  /** Decides whether this widget renders at all for this viewer.
   *  Returns false to hide entirely (no card chrome, no placeholder).
   *  Used for share-toggle + role gates — NOT for "no data" cases
   *  (the renderer itself handles empty-data internally). */
  isAvailable(ctx: ViewerCtx): Promise<boolean> | boolean;
  /** Returns the inner content (NOT including the card shell or
   *  eyebrow — that's WidgetFrame's job). May return null to indicate
   *  "no data yet" (WidgetFrame renders the empty placeholder). */
  render(ctx: ViewerCtx, size: WidgetSize): Promise<ReactElement | null>;
  /**
   * The widget's fetch + normalise step, WITHOUT its flat presentation.
   *
   * Present on every widget built with `defineWidget` (all but `journey`).
   * The projection surface (`/me`) reuses these loaders and draws the result
   * in the holographic language instead — so the endpoints, empty checks,
   * trend maths and provenance caveats stay in exactly one place while the
   * two surfaces render them differently.
   *
   * Returns `null` for "no data / error", same contract as inside `render`.
   * Typed as `unknown` because each widget's shape is its own; the projection
   * element that consumes it narrows to the shape it knows.
   */
  load?(ctx: ViewerCtx): Promise<unknown | null>;
}
