/**
 * Moderation controls on the admin user-detail page.
 *
 * Two things this component must not imply:
 *
 *   1. That Reinstate is an undo. A suspension DELETES the user's share
 *      grants in SpiceDB; lifting the restriction does not bring them
 *      back. The panel says so next to the button, because a moderator
 *      who believes it is reversible will use it more freely than they
 *      should.
 *
 *   2. That a restriction is only a record. It is enforced at the
 *      request path — ingest, sharing, public profile and submissions
 *      each refuse with 403. The capability labels say what actually
 *      stops, not what is "flagged".
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';
import type { AdminRestrictionDto } from '@/lib/api';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';

const CAPABILITIES: ReadonlyArray<{
  name: string;
  label: string;
  hint: string;
}> = [
  {
    name: 'ingest_blocked',
    label: 'Ingest',
    hint: 'Their tray can no longer upload events.',
  },
  {
    name: 'sharing_blocked',
    label: 'Sharing',
    hint: 'They cannot create new shares with users or orgs.',
  },
  {
    name: 'public_profile_blocked',
    label: 'Public profile',
    hint: 'Hidden from /discover and their public pages stop serving.',
  },
  {
    name: 'submissions_blocked',
    label: 'Submissions',
    hint: 'They cannot file new parser submissions.',
  },
];

export function RestrictionPanel({
  current,
  restrictAction,
  reinstateAction,
  canModerate,
}: {
  current: AdminRestrictionDto | null;
  restrictAction: (formData: FormData) => void | Promise<void>;
  reinstateAction: (formData: FormData) => void | Promise<void>;
  canModerate: boolean;
}) {
  return (
    <section className="ss-card" style={{ padding: '20px 24px' }}>
      <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
        Moderation
      </div>
      <h2
        style={{
          margin: 0,
          fontSize: 17,
          fontWeight: 600,
          letterSpacing: '-0.01em',
        }}
      >
        Account restrictions
      </h2>

      {current ? (
        <div
          className="ss-badge"
          style={{
            display: 'inline-block',
            margin: '12px 0 0',
            borderColor: 'var(--danger)',
            color: 'var(--danger)',
          }}
        >
          {current.is_suspension ? 'Suspended' : 'Limited'}
        </div>
      ) : (
        <p
          style={{
            margin: '10px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
          }}
        >
          No restrictions. This account can ingest, share, publish a
          public profile and file submissions.
        </p>
      )}

      {current && (
        <dl
          style={{
            display: 'grid',
            gridTemplateColumns: 'auto 1fr',
            gap: '8px 16px',
            margin: '12px 0 0',
            fontSize: 13,
          }}
        >
          <dt style={{ color: 'var(--fg-muted)' }}>Blocked</dt>
          <dd style={{ margin: 0 }}>
            {CAPABILITIES.filter(
              (c) => current[c.name as keyof AdminRestrictionDto],
            )
              .map((c) => c.label)
              .join(', ') || '—'}
          </dd>
          <dt style={{ color: 'var(--fg-muted)' }}>Reason</dt>
          <dd style={{ margin: 0 }}>{current.reason}</dd>
          <dt style={{ color: 'var(--fg-muted)' }}>Set by</dt>
          <dd style={{ margin: 0 }} className="mono">
            {current.restricted_by}
          </dd>
          <dt style={{ color: 'var(--fg-muted)' }}>Expires</dt>
          <dd style={{ margin: 0 }}>
            {current.expires_at
              ? current.expires_at.slice(0, 10)
              : 'until lifted'}
          </dd>
          {current.shares_revoked > 0 && (
            <>
              <dt style={{ color: 'var(--fg-muted)' }}>Shares revoked</dt>
              <dd style={{ margin: 0 }}>{current.shares_revoked}</dd>
            </>
          )}
        </dl>
      )}

      {!canModerate ? (
        <p
          style={{
            margin: '14px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
          }}
        >
          Moderator role required to change restrictions.
        </p>
      ) : (
        <>
          <form
            action={restrictAction}
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              margin: '16px 0 0',
            }}
          >
            <fieldset
              style={{
                border: '1px solid var(--border)',
                borderRadius: 0,
                padding: '12px 14px',
                display: 'flex',
                flexDirection: 'column',
                gap: 10,
              }}
            >
              <legend style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                Capabilities to block
              </legend>
              {CAPABILITIES.map((c) => (
                <label key={c.name} className="hp-kvvalue">
                  <input
                    type="checkbox"
                    name={c.name}
                    value="on"
                    defaultChecked={Boolean(
                      current?.[c.name as keyof AdminRestrictionDto],
                    )}
                  />
                  <span>
                    {c.label}
                    <span className="hp-fine">{c.hint}</span>
                  </span>
                </label>
              ))}
            </fieldset>

            <label style={{ fontSize: 13 }}>
              Reason (required — shown to the user)
              <input
                type="text"
                name="reason"
                required
                maxLength={280}
                defaultValue={current?.reason ?? ''}
                placeholder="e.g. spamming share invites"
                style={{
                  display: 'block',
                  width: '100%',
                  marginTop: 4,
                  padding: '8px 12px',
                  background: 'var(--bg-elev)',
                  border: '1px solid var(--border)',
                  borderRadius: 0,
                  color: 'var(--fg)',
                }}
              />
            </label>

            <label style={{ fontSize: 13 }}>
              Expires (optional — blank means until lifted)
              <input
                type="date"
                name="expires_on"
                defaultValue={current?.expires_at?.slice(0, 10) ?? ''}
                style={{
                  display: 'block',
                  marginTop: 4,
                  padding: '8px 12px',
                  background: 'var(--bg-elev)',
                  border: '1px solid var(--border)',
                  borderRadius: 0,
                  color: 'var(--fg)',
                }}
              />
            </label>

            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
              <ConfirmSubmitButton className="ss-btn ss-btn--primary">
                Apply restrictions
              </ConfirmSubmitButton>
              <ConfirmSubmitButton
                name="suspend_all"
                value="1"
                className="ss-btn ss-btn--danger"
                confirm="Suspend this account? All four capabilities are blocked and every existing share is revoked. Revoked shares are NOT restored if you reinstate them later."
              >
                Suspend (all capabilities)
              </ConfirmSubmitButton>
            </div>
          </form>

          {current && (
            <form action={reinstateAction} style={{ margin: '14px 0 0' }}>
              <ConfirmSubmitButton
                className="ss-btn ss-btn--ghost"
                confirm="Lift all restrictions on this account? Revoked shares are NOT restored — the user has to create them again."
              >
                Reinstate
              </ConfirmSubmitButton>
              <p
                style={{
                  margin: '8px 0 0',
                  color: 'var(--fg-dim)',
                  fontSize: 12,
                  maxWidth: 520,
                }}
              >
                Reinstating restores their capabilities. Revoked shares
                are not restored — those grants were deleted, not
                paused, so the user has to create them again.
              </p>
            </form>
          )}
        </>
      )}
    </section>
  );
}
