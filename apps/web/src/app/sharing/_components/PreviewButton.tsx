'use client';

/**
 * Audit v2.1 §B1 — "Preview as @handle" trigger.
 *
 * Sits next to the Grant submit button. Reads the current scope_*
 * form-field values + the recipient handle, builds a ShareScope JSON
 * payload, opens a new tab to /sharing/preview with the scope in the
 * URL.
 *
 * The preview page calls the simulated-render endpoints that don't
 * write audit rows, so previewing is free of trail noise — see the
 * `preview_summary` / `preview_timeline` Rust handlers.
 */

import { useCallback } from 'react';

const SHARE_FORM_ID = 'share-editor';

type ShareScope = {
  kind?: string;
  tabs?: string[];
  window_days?: number;
  allow_event_types?: string[];
  deny_event_types?: string[];
};

function readScopeFromForm(): { scope: ShareScope; recipient: string } | null {
  const form = document.getElementById(SHARE_FORM_ID);
  if (!(form instanceof HTMLFormElement)) return null;

  const recipientEl = form.elements.namedItem('recipient_handle');
  const recipient =
    recipientEl instanceof HTMLInputElement ? recipientEl.value.trim() : '';

  const kindEl = form.elements.namedItem('scope_kind');
  const kind =
    kindEl instanceof HTMLSelectElement ? kindEl.value : 'full';

  const winEl = form.elements.namedItem('scope_window_days');
  const winRaw = winEl instanceof HTMLInputElement ? winEl.value.trim() : '';

  const allowEl = form.elements.namedItem('scope_allow_event_types');
  const allowRaw =
    allowEl instanceof HTMLInputElement ? allowEl.value.trim() : '';

  const denyEl = form.elements.namedItem('scope_deny_event_types');
  const denyRaw =
    denyEl instanceof HTMLInputElement ? denyEl.value.trim() : '';

  const tabsField = form.elements.namedItem('scope_tabs');
  const tabs: string[] = [];
  if (tabsField instanceof RadioNodeList) {
    for (const node of Array.from(tabsField)) {
      if (node instanceof HTMLInputElement && node.checked) tabs.push(node.value);
    }
  } else if (tabsField instanceof HTMLInputElement && tabsField.checked) {
    tabs.push(tabsField.value);
  }

  const splitCsv = (raw: string): string[] =>
    raw
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);

  const scope: ShareScope = { kind };
  if (winRaw !== '') {
    const n = Number.parseInt(winRaw, 10);
    if (Number.isFinite(n) && n > 0) scope.window_days = n;
  }
  if (tabs.length > 0) scope.tabs = tabs;
  const allowList = splitCsv(allowRaw);
  if (allowList.length > 0) scope.allow_event_types = allowList;
  const denyList = splitCsv(denyRaw);
  if (denyList.length > 0) scope.deny_event_types = denyList;

  return { scope, recipient };
}

export function PreviewButton() {
  const onClick = useCallback(() => {
    const got = readScopeFromForm();
    if (!got) return;
    const qs = new URLSearchParams();
    qs.set('scope', JSON.stringify(got.scope));
    // Recipient handle is cosmetic — server doesn't use it; the
    // banner does, so the previewer remembers who they're sizing
    // the share for. Falls back to the literal "friend" when empty
    // so the button still works before a handle is typed.
    qs.set('as', got.recipient || 'friend');
    window.open(`/sharing/preview?${qs.toString()}`, '_blank', 'noopener');
  }, []);

  return (
    <button
      type="button"
      onClick={onClick}
      className="ss-btn ss-btn--ghost"
      title="Preview what the recipient would see with the current scope"
    >
      Preview
    </button>
  );
}
