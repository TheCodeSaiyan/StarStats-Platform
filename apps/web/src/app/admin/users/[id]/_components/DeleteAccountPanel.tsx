/**
 * Admin danger zone: delete an account.
 *
 * Two modes with genuinely different consequences, so the UI states
 * what each one KEEPS as well as what it removes:
 *
 *   pseudonymise — the same thing a user gets from their own Settings.
 *                  Event rows survive with the handle replaced, so
 *                  anyone they shared with keeps a coherent timeline.
 *   purge        — the events go too. Irreversible, and it removes rows
 *                  from OTHER people's timelines. That consequence
 *                  falls on third parties and is the one an admin is
 *                  least likely to think of, so it is spelled out.
 *
 * Pseudonymise is the default. Purge has to be chosen deliberately, not
 * arrived at by clicking through.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';
import { Plane } from 'holo';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';

export function DeleteAccountPanel({
  handle,
  deleteAction,
  isAdmin,
}: {
  handle: string;
  deleteAction: (formData: FormData) => void | Promise<void>;
  isAdmin: boolean;
}) {
  return (
    <Plane tilt="flat">
      <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
        Danger zone
      </div>
      <h2
        style={{
          margin: 0,
          fontSize: 17,
          fontWeight: 600,
          color: 'var(--danger)',
        }}
      >
        Delete account
      </h2>

      {!isAdmin ? (
        <p
          style={{
            margin: '12px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
          }}
        >
          Admin role required. Moderators can restrict or suspend an
          account, but only an admin can delete one.
        </p>
      ) : (
        <form
          action={deleteAction}
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 12,
            margin: '14px 0 0',
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
            <legend className="hp-kvlabel">
              What happens to their data
            </legend>

            <label className="hp-kvvalue">
              <input
                type="radio"
                name="mode"
                value="pseudonymise"
                defaultChecked
              />
              <span>
                Pseudonymise
                <span className="hp-fine">
                  Same as the user deleting themselves. Account, devices
                  and shares are removed; event rows are kept but
                  unlinked from them, so people they shared with keep a
                  coherent timeline.
                </span>
              </span>
            </label>

            <label className="hp-kvvalue">
              <input type="radio" name="mode" value="purge" />
              <span>
                Purge
                <span className="hp-fine">
                  Everything above, and deletes their events outright.
                  Permanent. Anyone they shared with loses those rows
                  from their own timelines.
                </span>
              </span>
            </label>
          </fieldset>

          <label style={{ fontSize: 13 }}>
            Type{' '}
            <span className="mono" style={{ color: 'var(--fg)' }}>
              {handle}
            </span>{' '}
            to confirm
            <input
              type="text"
              name="confirm_handle"
              required
              autoComplete="off"
              spellCheck={false}
              className="mono"
              placeholder={handle}
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

          <ConfirmSubmitButton
            className="hp-btn hp-btn--ghost"
            style={{ color: 'var(--danger)', borderColor: 'var(--danger)' }}
            confirm={`Delete ${handle}? This cannot be undone. If you chose Purge, their events are deleted outright and anyone they shared with loses those rows.`}
          >
            Delete account
          </ConfirmSubmitButton>
        </form>
      )}
    </Plane>
  );
}
