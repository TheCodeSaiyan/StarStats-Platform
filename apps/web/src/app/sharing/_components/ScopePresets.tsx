'use client';

/**
 * Audit v2.1 §A2 — scope-picker presets.
 *
 * Three chips above the `<details>` scope picker in the grant form
 * that populate the form fields in one click. Pure client-side —
 * no server roundtrip, no schema change. The form stays a native
 * HTML form so the existing server action keeps working when the
 * user submits.
 *
 * We read/write fields by name on the form element with id
 * `share-editor` (sharing/page.tsx). The presets mirror the audit
 * v2.1 brief:
 *
 *   Friend       — kind=full,       no window
 *   Org member   — kind=aggregates, window=30
 *   Public link  — kind=timeline,   window=7
 *
 * Hand-editing any field after picking a preset clears the active
 * mark — the chips describe "you picked this preset", not "the form
 * exactly matches this preset", so the moment the user diverges we
 * stop claiming the chip is still in effect.
 */

import { useEffect, useRef, useState } from 'react';

const SHARE_FORM_ID = 'share-editor';

type PresetId = 'friend' | 'org' | 'public';

type Preset = {
  id: PresetId;
  label: string;
  description: string;
  kind: 'full' | 'aggregates' | 'timeline';
  windowDays: string;
  tabs: string[];
  allowTypes: string;
  denyTypes: string;
};

const PRESETS: ReadonlyArray<Preset> = [
  {
    id: 'friend',
    label: 'Friend',
    description: 'Full manifest, no window',
    kind: 'full',
    windowDays: '',
    tabs: [],
    allowTypes: '',
    denyTypes: '',
  },
  {
    id: 'org',
    label: 'Org member',
    description: 'Aggregates only, last 30d',
    kind: 'aggregates',
    windowDays: '30',
    tabs: [],
    allowTypes: '',
    denyTypes: '',
  },
  {
    id: 'public',
    label: 'Public link',
    description: 'Timeline only, last 7d',
    kind: 'timeline',
    windowDays: '7',
    tabs: [],
    allowTypes: '',
    denyTypes: '',
  },
];

function applyPreset(preset: Preset): boolean {
  const form = document.getElementById(SHARE_FORM_ID);
  if (!(form instanceof HTMLFormElement)) return false;

  const kind = form.elements.namedItem('scope_kind');
  if (kind instanceof HTMLSelectElement) kind.value = preset.kind;

  const win = form.elements.namedItem('scope_window_days');
  if (win instanceof HTMLInputElement) win.value = preset.windowDays;

  const allow = form.elements.namedItem('scope_allow_event_types');
  if (allow instanceof HTMLInputElement) allow.value = preset.allowTypes;

  const deny = form.elements.namedItem('scope_deny_event_types');
  if (deny instanceof HTMLInputElement) deny.value = preset.denyTypes;

  // Tabs is a checkbox group; namedItem returns a RadioNodeList here.
  const tabs = form.elements.namedItem('scope_tabs');
  const tabSet = new Set(preset.tabs);
  if (tabs instanceof RadioNodeList) {
    for (const node of Array.from(tabs)) {
      if (node instanceof HTMLInputElement) node.checked = tabSet.has(node.value);
    }
  } else if (tabs instanceof HTMLInputElement) {
    tabs.checked = tabSet.has(tabs.value);
  }

  // Open the `<details>` disclosure so the user can see what landed —
  // otherwise the picker stays collapsed and the click looks like it
  // did nothing.
  const details = form.querySelector('details');
  if (details instanceof HTMLDetailsElement) details.open = true;

  return true;
}

export function ScopePresets() {
  const [active, setActive] = useState<PresetId | null>(null);
  const installed = useRef(false);

  // Clear the active mark the moment the user hand-edits any scope
  // field — the chip should only claim accuracy when the form is
  // verbatim what the preset wrote.
  useEffect(() => {
    if (installed.current) return;
    installed.current = true;
    const form = document.getElementById(SHARE_FORM_ID);
    if (!(form instanceof HTMLFormElement)) return;
    const clear = () => setActive(null);
    const fields = [
      'scope_kind',
      'scope_window_days',
      'scope_allow_event_types',
      'scope_deny_event_types',
      'scope_tabs',
    ];
    const listeners: Array<[HTMLElement, string, () => void]> = [];
    for (const name of fields) {
      const field = form.elements.namedItem(name);
      const targets =
        field instanceof RadioNodeList ? Array.from(field) : [field];
      for (const t of targets) {
        if (t instanceof HTMLElement) {
          t.addEventListener('change', clear);
          listeners.push([t, 'change', clear]);
        }
      }
    }
    return () => {
      for (const [el, ev, fn] of listeners) el.removeEventListener(ev, fn);
    };
  }, []);

  return (
    <div
      role="group"
      aria-label="Quick scope presets"
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 6,
        marginTop: 4,
      }}
    >
      <span
        style={{
          fontSize: 11,
          color: 'var(--fg-dim)',
          alignSelf: 'center',
          marginRight: 4,
        }}
      >
        Quick start:
      </span>
      {PRESETS.map((p) => {
        const isActive = active === p.id;
        return (
          <button
            key={p.id}
            type="button"
            aria-pressed={isActive}
            title={p.description}
            onClick={() => {
              if (applyPreset(p)) setActive(p.id);
            }}
            style={{
              fontSize: 12,
              padding: '4px 10px',
              borderRadius: 'var(--r-pill)',
              border: '1px solid',
              borderColor: isActive
                ? 'var(--accent)'
                : 'var(--border)',
              background: isActive
                ? 'color-mix(in oklab, var(--accent) 14%, var(--bg-elev))'
                : 'var(--bg-elev)',
              color: isActive ? 'var(--fg)' : 'var(--fg-muted)',
              cursor: 'pointer',
            }}
          >
            {p.label}
          </button>
        );
      })}
    </div>
  );
}
