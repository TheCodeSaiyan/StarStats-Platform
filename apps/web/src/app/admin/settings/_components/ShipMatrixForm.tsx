'use client';

/**
 * Admin Ship Matrix config form. Client component (interactive toggle +
 * pending state). The save action is passed in as a prop rather than
 * imported from `@/lib/api` (which is `server-only` — the bearer token
 * must never reach the browser bundle).
 */

import { useState, useTransition } from 'react';

/** Local mirror of the API response — duplicated here so this client
 *  module doesn't import the server-only `@/lib/api`. Pinned to the
 *  OpenAPI codegen by the action signature the parent page passes in. */
export interface ShipMatrixConfigView {
  media_enabled: boolean;
}

export type ActionResult =
  | { kind: 'saved'; config: ShipMatrixConfigView }
  | { kind: 'error'; message: string };

export function ShipMatrixForm({
  initial,
  saveAction,
}: {
  initial: ShipMatrixConfigView;
  saveAction: (mediaEnabled: boolean) => Promise<ActionResult>;
}) {
  const [mediaEnabled, setMediaEnabled] = useState(initial.media_enabled);
  const [savedEnabled, setSavedEnabled] = useState(initial.media_enabled);
  const [banner, setBanner] = useState<
    { kind: 'ok' | 'error'; text: string } | null
  >(null);
  const [pending, startTransition] = useTransition();

  const dirty = mediaEnabled !== savedEnabled;

  function onSave() {
    setBanner(null);
    startTransition(async () => {
      const res = await saveAction(mediaEnabled);
      if (res.kind === 'saved') {
        setSavedEnabled(res.config.media_enabled);
        setMediaEnabled(res.config.media_enabled);
        setBanner({
          kind: 'ok',
          text: res.config.media_enabled
            ? 'Saved. Official ship images are now visible.'
            : 'Saved. Ship images are now hidden.',
        });
      } else {
        setBanner({ kind: 'error', text: res.message });
      }
    });
  }

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 16,
        maxWidth: 560,
      }}
    >
      <label
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          gap: 12,
          cursor: 'pointer',
          padding: 16,
          border: '1px solid var(--border)',
          borderRadius: 'var(--r-card, 12px)',
          background: 'var(--bg-elev)',
        }}
      >
        <input
          type="checkbox"
          checked={mediaEnabled}
          disabled={pending}
          onChange={(e) => setMediaEnabled(e.target.checked)}
          style={{ marginTop: 2, width: 18, height: 18 }}
        />
        <span style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ fontWeight: 600, fontSize: 15 }}>
            Surface RSI ship images
          </span>
          <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
            When on, vehicle KB pages render the official Ship Matrix
            image gallery through the server proxy. When off, images are
            hidden (the proxy 404s); specs and descriptions are
            unaffected either way.
          </span>
        </span>
      </label>

      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <button
          type="button"
          className="ss-btn ss-btn--primary"
          onClick={onSave}
          disabled={pending || !dirty}
        >
          {pending ? 'Saving…' : 'Save'}
        </button>
        {dirty && !pending && (
          <span style={{ color: 'var(--fg-dim)', fontSize: 13 }}>
            Unsaved change
          </span>
        )}
      </div>

      {banner && (
        <div
          role="status"
          style={{
            fontSize: 13,
            padding: '8px 12px',
            borderRadius: 'var(--r-pill)',
            border: '1px solid',
            borderColor:
              banner.kind === 'ok' ? 'var(--border)' : 'var(--danger, #c33)',
            color:
              banner.kind === 'ok' ? 'var(--fg-muted)' : 'var(--danger, #c33)',
          }}
        >
          {banner.text}
        </div>
      )}
    </div>
  );
}
