/**
 * Re-parse card — re-runs the current classifier (built-ins + remote
 * rules) over every stored event line in place. Idempotent on a
 * stable rule set, so a second click is a no-op.
 *
 * Pulled out of SettingsPane to keep that component focused on
 * editable form fields. This card is purely action-oriented.
 */

import { useState } from 'react';
import { api, type ReparseStats } from '../api';
import { GhostButton, TrayCard } from './tray/primitives';

type State =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'done'; result: ReparseStats }
  | { kind: 'error'; message: string };

export function ReparseCard() {
  const [state, setState] = useState<State>({ kind: 'idle' });

  const handleClick = async () => {
    setState({ kind: 'running' });
    try {
      const result = await api.reparseEvents();
      if (result.error) {
        setState({ kind: 'error', message: result.error });
      } else {
        setState({ kind: 'done', result });
      }
    } catch (e) {
      setState({ kind: 'error', message: String(e) });
    }
  };

  return (
    <TrayCard
      title="Re-parse local store"
      kicker={state.kind === 'running' ? 'working…' : undefined}
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <p
          style={{
            margin: 0,
            fontSize: 12,
            color: 'var(--fg-muted)',
            lineHeight: 1.5,
          }}
        >
          Re-runs the current classifier over every stored event line.
          Useful after a parser update — recognises lines that were
          previously unknown, refines classifications that improved.
          Idempotent: safe to click again.
        </p>

        <div>
          <GhostButton
            type="button"
            onClick={handleClick}
            disabled={state.kind === 'running'}
          >
            {state.kind === 'running' ? 'Re-parsing…' : 'Re-parse now'}
          </GhostButton>
        </div>

        <ReparseStatusLine state={state} />
      </div>
    </TrayCard>
  );
}

function ReparseStatusLine({ state }: { state: State }) {
  if (state.kind === 'idle') {
    return null;
  }

  if (state.kind === 'running') {
    return (
      <div style={{ fontSize: 11, color: 'var(--fg-muted)' }}>
        Walking the local event store…
      </div>
    );
  }

  if (state.kind === 'error') {
    return (
      <div style={{ fontSize: 11, color: 'var(--danger)' }}>
        {state.message}
      </div>
    );
  }

  const {
    examined,
    updated,
    kept_unmatched,
    promoted_unknowns,
    bursts_collapsed,
    members_suppressed,
  } = state.result;
  const noChanges =
    updated === 0 && promoted_unknowns === 0 && bursts_collapsed === 0;
  const burstFragment =
    bursts_collapsed > 0
      ? `, collapsed ${bursts_collapsed.toLocaleString()} burst${
          bursts_collapsed === 1 ? '' : 's'
        } (suppressed ${members_suppressed.toLocaleString()} spam rows)`
      : '';
  return (
    <div
      style={{
        fontSize: 11,
        color: noChanges ? 'var(--fg-muted)' : 'var(--ok)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {noChanges
        ? `Examined ${examined.toLocaleString()} events — no changes`
        : `Updated ${updated.toLocaleString()} events, promoted ${promoted_unknowns.toLocaleString()} unknowns${burstFragment} (examined ${examined.toLocaleString()}, kept ${kept_unmatched.toLocaleString()} as-is).`}
    </div>
  );
}
