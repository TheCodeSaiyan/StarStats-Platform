import { useState } from 'react';
import { GhostButton } from './tray/primitives';

export interface InlineCheckResult {
  ok: boolean;
  message: string;
}

interface Props {
  label: string;
  value: string;
  onCheck: (value: string) => Promise<InlineCheckResult>;
}

type State =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'ok'; detail: string }
  | { kind: 'err'; detail: string };

export function InlineCheck({ label, value, onCheck }: Props) {
  const [state, setState] = useState<State>({ kind: 'idle' });
  const disabled = state.kind === 'running' || !value.trim();

  const run = async () => {
    setState({ kind: 'running' });
    try {
      const r = await onCheck(value);
      setState(r.ok ? { kind: 'ok', detail: r.message } : { kind: 'err', detail: r.message });
    } catch (e) {
      setState({ kind: 'err', detail: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 6 }}>
      <GhostButton
        type="button"
        onClick={run}
        disabled={disabled}
        style={{ padding: '3px 10px', fontSize: 11, alignSelf: 'flex-start' }}
      >
        {state.kind === 'running' ? `Testing…` : label}
      </GhostButton>
      {state.kind === 'ok' && (
        <span style={{ fontSize: 11, color: 'var(--ok)' }}>✓ {state.detail}</span>
      )}
      {state.kind === 'err' && (
        <span style={{ fontSize: 11, color: 'var(--danger)' }}>✗ {state.detail}</span>
      )}
    </div>
  );
}
