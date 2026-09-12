import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Plane, BeamInput, BeamSelect, BeamButton } from 'holo';
import { ExpiryField } from '../ExpiryField';
import { ScopePresets } from '../_components/ScopePresets';
import { PreviewButton } from '../_components/PreviewButton';
import { SCOPE_TAB_OPTIONS } from '../scope-tabs';
import type { ShareScope } from '@/lib/api';

/**
 * Grant / edit a share.
 *
 * FIELD NAMES ARE THE CONTRACT and are unchanged: `recipient_handle`, `note`,
 * `expires_at_local` + `tz_offset_minutes` (from `ExpiryField`), `scope_kind`,
 * `scope_tabs`, `scope_window_days`, `scope_allow_event_types`,
 * `scope_deny_event_types`. `addShareAction` parses exactly these, and
 * `ScopePresets` and `PreviewButton` both READ THE LIVE FORM by field name —
 * so renaming one would silently break the presets and the preview rather
 * than failing loudly.
 *
 * The form keeps `id="share-editor"`: the edit flow navigates to
 * `/sharing?edit=<handle>#share-editor`, and the outbound section declares
 * that id as a secondary anchor so the rail opens the right group first.
 */
export function ShareEditor({
  addShareAction,
  isEditing,
  prefilledHandle,
  prefilledNote,
  prefilledExpires,
  prefilledScope,
}: {
  addShareAction: (formData: FormData) => void | Promise<void>;
  isEditing: boolean;
  prefilledHandle: string;
  prefilledNote: string;
  prefilledExpires: string;
  /** The scope this share already has, when editing. `null` for a new
   *  grant, which takes the narrow default instead. */
  prefilledScope: ShareScope | null;
}) {
  /*
   * A new grant is time-boxed by default.
   *
   * The old default was the full manifest with no window — the widest
   * thing the form could express, reached by filling in a handle and
   * pressing the button. Narrow by default, widen deliberately.
   *
   * `kind` stays `full`: kinds name SURFACES, and a recipient's profile
   * reads the summary, the heatmap AND the event feed, so a narrower
   * kind 404s parts of the page rather than trimming them. The window
   * is the clamp that actually bounds what is shared.
   *
   * When editing, every control starts from the scope the share already
   * has, so saving an untouched form is a no-op rather than a silent
   * widening.
   */
  const scopeKind = prefilledScope?.kind ?? 'full';
  const windowDays = prefilledScope?.window_days ?? (isEditing ? undefined : 30);
  const allowTypes = (prefilledScope?.allow_event_types ?? []).join(', ');
  const denyTypes = (prefilledScope?.deny_event_types ?? []).join(', ');
  const selectedTabs = new Set(prefilledScope?.tabs ?? []);
  return (
    <Plane
      tilt="flat"
      cap={isEditing ? 'Edit share' : 'Grant access'}
      hint={isEditing ? 'blank a field to clear it' : undefined}
      style={{ marginTop: 22 }}
    >
      <form id="share-editor" action={addShareAction}>
        {isEditing ? (
          <p className="hp-prose" style={{ marginTop: 0 }}>
            Editing the share with{' '}
            <span className="val">{prefilledHandle}</span> — blank out a field
            and save to clear it.
          </p>
        ) : null}

        <div className="hp-formrow">
          <BeamInput
            id="recipient-handle"
            label="RSI handle"
            type="text"
            name="recipient_handle"
            placeholder="RSI handle"
            defaultValue={prefilledHandle}
            autoComplete="off"
            spellCheck={false}
            required
            readOnly={isEditing}
          />
          <BeamInput
            id="share-note"
            label="Note"
            type="text"
            name="note"
            maxLength={280}
            defaultValue={prefilledNote}
            placeholder="Optional, max 280 chars"
          />
        </div>

        <div className="hp-formrow">
          <label className="hp-field" htmlFor="expires_at_local">
            <span>Auto-expiry</span>
            <ExpiryField prefillIso={prefilledExpires || undefined} />
          </label>
        </div>

        {/* Quick-start presets. A client control that writes into the fields
            below by name — see the contract note above. */}
        <div style={{ marginTop: 18 }}>
          <ScopePresets />
        </div>

        <p className="hp-note" style={{ marginTop: 14 }}>
          {isEditing
            ? 'Every field below starts from this share’s current scope — saving without changing anything leaves it exactly as it is.'
            : `New shares cover the last ${windowDays} days. Open More options to widen the window or restrict what is shared.`}
        </p>

        {/* Everything past this point is the long tail of the form. A
            native <details> keeps it one keystroke away and, unlike a
            JS disclosure, still works with scripting off — which the
            rest of this form deliberately does too. Open when editing a
            share that already carries a clamp, so a scope somebody set
            is never hidden from the person reviewing it. */}
        <details
          className="hp-disclosure"
          open={prefilledScope != null}
          style={{ marginTop: 14 }}
        >
          {/* No `.hp-lg` here: that is the event-log ROW
              (`grid-template-columns: 64px 1fr 92px`), and it put this label
              in the 64px track — one word per line, 104px tall for a single
              line of text. `.hp-disclosure > summary` already styles it as a
              control. */}
          <summary style={{ cursor: 'pointer' }}>
            More options — scope, window, tabs, event types
          </summary>

          <div className="hp-formrow" style={{ marginTop: 14 }}>
            <BeamSelect
              id="scope-kind"
              name="scope_kind"
              label="Scope"
              defaultValue={scopeKind}
            >
              <option value="full">Full manifest</option>
              <option value="timeline">Timeline only</option>
              <option value="aggregates">Aggregates only</option>
              <option value="tabs">Specific tabs…</option>
            </BeamSelect>
            <BeamInput
              id="scope-window-days"
              label="Window (days)"
              type="number"
              name="scope_window_days"
              min={1}
              placeholder="all"
              defaultValue={windowDays}
            />
          </div>

          {/* Scope tabs. Always rendered, not revealed by the `tabs` kind: the
              server ignores `scope_tabs` unless the kind selects it, and the
              original behaved the same way. Hiding them behind the select would
              need JavaScript to reveal, and this form is deliberately usable
              without it. */}
          <fieldset className="hp-fieldset">
            <legend>Tabs (when scope is “Specific tabs”)</legend>
            <div className="hp-checkrow">
              {SCOPE_TAB_OPTIONS.map((t) => (
                <label className="hp-check" key={t.value}>
                  <input
                    type="checkbox"
                    name="scope_tabs"
                    value={t.value}
                    defaultChecked={selectedTabs.has(t.value)}
                  />
                  <span>{t.label}</span>
                </label>
              ))}
            </div>
          </fieldset>

          <div className="hp-formrow">
            <BeamInput
              id="scope-allow"
              label="Allow event types"
              type="text"
              name="scope_allow_event_types"
              placeholder="comma-separated, blank for all"
              spellCheck={false}
              defaultValue={allowTypes}
            />
            <BeamInput
              id="scope-deny"
              label="Deny event types"
              type="text"
              name="scope_deny_event_types"
              placeholder="comma-separated"
              spellCheck={false}
              defaultValue={denyTypes}
            />
          </div>
        </details>

        <div
          style={{
            display: 'flex',
            gap: 10,
            flexWrap: 'wrap',
            marginTop: 18,
          }}
        >
          <BeamButton type="submit" variant="primary">
            {isEditing ? 'Save changes' : 'Grant access'}
          </BeamButton>
          {/* Opens a new tab showing the owner's OWN data run through the
              scope currently in the form — so the size of a grant can be
              judged before it is made. */}
          <PreviewButton />
          {isEditing ? (
            <Link href={'/sharing' as Route} className="hp-btn hp-btn--ghost">
              Cancel
            </Link>
          ) : null}
        </div>
      </form>
    </Plane>
  );
}
