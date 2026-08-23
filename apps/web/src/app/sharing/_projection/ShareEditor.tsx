import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Plane, BeamInput, BeamSelect, BeamButton } from 'holo';
import { ExpiryField } from '../ExpiryField';
import { ScopePresets } from '../_components/ScopePresets';
import { PreviewButton } from '../_components/PreviewButton';
import { SCOPE_TAB_OPTIONS } from '../scope-tabs';

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
}: {
  addShareAction: (formData: FormData) => void | Promise<void>;
  isEditing: boolean;
  prefilledHandle: string;
  prefilledNote: string;
  prefilledExpires: string;
}) {
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

        <div className="hp-formrow">
          <BeamSelect
            id="scope-kind"
            name="scope_kind"
            label="Scope"
            defaultValue="full"
          >
            <option value="full">Full manifest (default)</option>
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
                <input type="checkbox" name="scope_tabs" value={t.value} />
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
          />
          <BeamInput
            id="scope-deny"
            label="Deny event types"
            type="text"
            name="scope_deny_event_types"
            placeholder="comma-separated"
            spellCheck={false}
          />
        </div>

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
