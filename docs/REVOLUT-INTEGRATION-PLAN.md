# Wave 9 — Donate via Revolut Business: integration plan

The user has chosen Revolut Business as the payment processor (the
project's earlier plan assumed Stripe; that path is dropped). This
document is the implementation plan a future wave will execute.

**Status as of this writing:** the full Revolut wiring is shipped,
gated on env vars. When `REVOLUT_API_KEY` / `REVOLUT_WEBHOOK_SECRET`
are absent the routes mount but return `503 not_configured` and the
`/donate` page falls back to a "coming soon" panel. As soon as those
env vars are set the tier buttons light up; nothing else needs a
deploy. The Revolut Business dashboard still needs configuring (API
key, webhook URL, signing secret) but the code path is in place.

Shipped components:
- Migration `0018_revolut_orders.sql` (orders + webhook-event dedup tables).
- `revolut.rs` — Merchant API client + HMAC-SHA256 webhook verification
  with constant-time compare and ±5 minute timestamp drift tolerance.
- `orders.rs` — order-tracking store (trait + Postgres + Memory).
- `revolut_routes.rs` — `POST /v1/donate/checkout`,
  `POST /v1/webhooks/revolut`, `GET /v1/donate/tiers`.
- `supporters.rs::mark_payment_received` — UPSERT that flips state
  to `active`, sets `became_supporter_at` on first payment only via
  COALESCE, and preserves the user's name plate when the order didn't
  carry one.
- `/donate/page.tsx` — tier grid + Server Action; redirects to Revolut
  hosted checkout. Falls back to the "coming soon" panel when the
  server's tier list 503s.
- `/donate/return/page.tsx` — landing page Revolut redirects to after
  checkout. Polls `GET /v1/me/supporter` via meta-refresh until the
  webhook lands and state flips to `active`.

## Scope

A donate flow with three lifecycle states, none of which ever drop a
user's account features:

| State | Effects | Triggered by |
|---|---|---|
| `none` | Default. Standard free-tier posture. | Account creation. |
| `active` | Supporter pill on profile. Optional 28-char "name plate". Optional retention extension on stored events. Accent-tinted UI affordances. | Successful Revolut payment (one-time or recurring). |
| `lapsed` | Pill + name plate **stay**. Retention extension + accent revert to free-tier. | Cancellation OR payment failure after grace period. |

The "lapsed never strips the pill" rule is intentional: someone who
supported the project once should keep their visible recognition, with
only the active retention/accent perks tied to ongoing payment.

## Schema (shipped — migration 0017)

```sql
CREATE TABLE supporter_status (
    user_id      UUID    PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    state        TEXT    NOT NULL DEFAULT 'none',
    name_plate   TEXT,                          -- max 28 chars at API layer
    became_supporter_at TIMESTAMPTZ,
    last_payment_at     TIMESTAMPTZ,
    grace_until         TIMESTAMPTZ,            -- payment-failure grace window
    cancelled_at        TIMESTAMPTZ,
    revolut_customer_id TEXT,                   -- nullable: filled on first checkout
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (state IN ('none', 'active', 'lapsed'))
);
```

## Endpoints

### Shipped

- `GET /v1/me/supporter` — returns the caller's supporter row. Always
  succeeds for an authenticated user (returns `{ state: 'none' }` when
  no row exists yet).

### Shipped — gated on env vars

- `GET /v1/donate/tiers` — static tier list (key, amount_minor,
  currency, label, description). Always available; the `/donate` page
  uses success-or-coming-soon to decide which UI to render.

- `POST /v1/donate/checkout` — body `{ tier_key, name_plate?: string }`.
  Creates a local pending order, calls Revolut Merchant API to mint a
  hosted-checkout URL, returns that URL. The order's
  `merchant_order_ext_ref` carries our local UUID so the webhook can
  correlate. Returns `503 not_configured` when env vars missing.

- `POST /v1/webhooks/revolut` — Revolut event sink. Verifies the
  `Revolut-Signature` HMAC against `REVOLUT_WEBHOOK_SECRET` (constant-
  time compare via `subtle::ConstantTimeEq`, supports comma-separated
  multiple `v1=` entries during secret rotation). On `ORDER_COMPLETED`
  flips state to `active` and records `last_payment_at`. On
  `ORDER_CANCELLED` / `ORDER_FAILED` updates the order row's state
  but leaves the supporter row alone. Redeliveries deduped by the
  `revolut_webhook_events` PK on `(revolut_order_id, event_type)`.

### Still to ship

- `PUT /v1/me/supporter/name-plate` — standalone plate edit endpoint
  (today the plate is set at checkout time only). Low priority — the
  user can change it on the next donation.

## Env vars

Required for the live Revolut wiring (absence → 503):

- `REVOLUT_API_KEY` — Bearer token from the Merchant API section of
  the Revolut Business dashboard.
- `REVOLUT_WEBHOOK_SECRET` — HMAC signing secret from the dashboard's
  webhook configuration. Must be the same value Revolut shows there.

Optional (sensible defaults):

- `REVOLUT_API_BASE` — base URL with no `/api/1.0` suffix. Defaults to
  `https://sandbox-merchant.revolut.com`. Switch to
  `https://merchant.revolut.com` for production.
- `REVOLUT_API_VERSION` — pinned to `2024-09-01` by default so a
  future Revolut breaking change can't silently land in production.
- `REVOLUT_RETURN_URL` — where Revolut redirects after a hosted
  checkout completes (e.g. `https://app.example.com/donate/return`).
  Falls back to the merchant default in the dashboard if unset.

### Setup checklist (when you provision credentials)

1. In the Revolut Business dashboard, navigate to
   **Merchant → APIs → Merchant API → API Keys** and issue a new
   key. Copy it — only shown once. This is `REVOLUT_API_KEY`.

2. Provision the webhook **via the API** (the dashboard no longer
   has a Webhooks UI — webhook management is API-only as of the
   `2024-09-01` Merchant API version). Run against sandbox first:

   ```bash
   curl -X POST https://sandbox-merchant.revolut.com/api/1.0/webhooks \
     -H "Authorization: Bearer $REVOLUT_API_KEY" \
     -H "Revolut-Api-Version: 2024-09-01" \
     -H "Content-Type: application/json" \
     -d '{
       "url": "https://api.example.com/v1/webhooks/revolut",
       "events": ["ORDER_COMPLETED", "ORDER_FAILED", "ORDER_CANCELLED"]
     }'
   ```

   For production swap the host to `https://merchant.revolut.com`.
   The response carries `id` (webhook_id, needed later for PATCH /
   DELETE / signing-secret rotation) and `signing_secret`.

3. Copy `response.signing_secret` into `REVOLUT_WEBHOOK_SECRET`. The
   secret is also retrievable later via
   `GET /api/1.0/webhooks/{id}` if you lose it before storing —
   unlike the API key, which is genuinely one-shot.

4. Set the env vars on the deploy. The server logs
   `Revolut Business merchant API configured` at boot when the keys
   resolve; absence logs the matching "not configured" line and the
   donate routes 503.

Other Merchant API webhook events the server tolerates but does not
act on: `ORDER_AUTHORISED` (informational — for the instant-capture
donate flow it precedes `ORDER_COMPLETED` by milliseconds with no
useful action between them), plus the broader
`ORDER_PAYMENT_*` / `ORDER_INCREMENTAL_AUTHORISATION_*` families
that aren't relevant to one-shot hosted checkouts. Subscribing
to the three terminal events above is sufficient.

## Tiers

Defined as a `const` table in `revolut_routes.rs::TIERS`. Today's set:

| Tier key | Amount | Effects |
|---|---|---|
| `coffee` | £3 one-time | `active` for 30 days |
| `standard` | £5 one-time | `active` for 30 days |
| `generous` | £15 one-time | `active` for 30 days |

Edit the const + run `cargo test` + redeploy to change. Renaming a
tier key breaks historical reporting (existing orders carry the old
key) — add a new key instead of renaming.

**Recurring orders are not wired yet.** All tiers are one-time
payments. The supporter row's `grace_until` extends 30 days from each
payment, so a user who donates monthly stays `active` continuously.
Real subscription support (Revolut's recurring orders) is a future
follow-up — needs a `subscription_id` column and a separate webhook
event handler.

## Decisions made (and what they mean)

1. **Currency: GBP only.** The TIERS const carries `currency: "GBP"`
   on every tier. Multi-currency support deferred until there's
   evidence of demand — adding a currency picker is one PR.
2. **One-time only for now.** Recurring orders need a different
   Merchant API path and a `subscriptions` table. The 30-day coverage
   window means a monthly cadence still works for engaged supporters.
3. **Grace window: 30 days.** Hard-coded as `COVERAGE_DAYS` in
   `revolut_routes.rs`. Move to `RevolutConfig` if it needs to be
   env-tunable.
4. **Webhook dedup: PK on `(revolut_order_id, event_type)`.** The
   `revolut_webhook_events` table keys on this pair, so a redelivered
   `ORDER_COMPLETED` is INSERTed with `ON CONFLICT DO NOTHING` and
   the route layer skips side effects when the insert returns 0 rows.
5. **Refund handling: state flips, supporter row left alone.** If
   Revolut sends `ORDER_REFUNDED` we mark the order row `refunded`
   but do NOT downgrade the supporter row. The "lapse never strips
   the pill" rule applies; the user's dashboard is the authoritative
   place to claw back perks if needed.

## Open follow-ups (not blockers)

- Reaper: a daily job that deletes `revolut_orders` rows still
  `pending` after 24h. Today they accumulate on every abandoned
  checkout. Low priority — they don't affect correctness, just
  hygiene.
- Plate edit endpoint (`PUT /v1/me/supporter/name-plate`): standalone
  edit without re-paying.
- Currency picker.
- Subscription support (Revolut recurring orders).
