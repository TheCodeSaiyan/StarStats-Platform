import { type CSSProperties, useCallback, useEffect, useState } from 'react';
import { applyUpdate, checkForUpdate, type UpdateInfo } from '../updater';
import type { ReleaseChannel } from '../api';
import { GhostButton, PrimaryButton } from './tray/primitives';

type State =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'up_to_date' }
  | { kind: 'installing'; downloaded: number; total: number | null }
  | { kind: 'error'; message: string };

const bannerStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 10,
  padding: '8px 12px',
  background: 'var(--bg-elev)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--r-sm)',
  fontSize: 12,
};
const rowStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
};
const hintStyle: CSSProperties = { fontSize: 11, color: 'var(--fg-muted)' };

/**
 * Compact update affordance for the status pane. Proactively checks the
 * configured release channel once on mount, shows an actionable
 * "Install & restart" banner when a newer release is available, and a
 * manual "Check for updates" button otherwise. Reuses the shared
 * `updater.ts` helpers, so this and the detailed Settings update section
 * stay in lockstep (one source of truth for check/install).
 */
export function UpdateBanner({
  channel,
  autoCheck,
}: {
  channel: ReleaseChannel;
  autoCheck: boolean;
}) {
  const [state, setState] = useState<State>(
    autoCheck ? { kind: 'checking' } : { kind: 'idle' },
  );

  const check = useCallback(async () => {
    setState({ kind: 'checking' });
    try {
      const result = await checkForUpdate(channel);
      setState(
        result.available
          ? { kind: 'available', info: result }
          : { kind: 'up_to_date' },
      );
    } catch (e) {
      setState({ kind: 'error', message: String(e) });
    }
  }, [channel]);

  // Proactive check on mount — but ONLY if the user hasn't turned off
  // auto-checks in Settings. The manual "Check for updates" button below
  // always works regardless, matching the Rust side's rule that a manual
  // check ignores the auto_update_check flag while the startup auto-check
  // honours it.
  useEffect(() => {
    if (autoCheck) {
      void check();
    }
  }, [autoCheck, check]);

  const install = useCallback(async () => {
    setState({ kind: 'installing', downloaded: 0, total: null });
    try {
      await applyUpdate(channel, (downloaded, total) =>
        setState({ kind: 'installing', downloaded, total }),
      );
      // applyUpdate relaunches on success; nothing runs after.
    } catch (e) {
      setState({ kind: 'error', message: String(e) });
    }
  }, [channel]);

  if (state.kind === 'available') {
    return (
      <div style={bannerStyle}>
        <span>
          ⬆ Update available — <strong>v{state.info.version}</strong>
        </span>
        <PrimaryButton type="button" onClick={() => void install()}>
          Install &amp; restart
        </PrimaryButton>
      </div>
    );
  }

  if (state.kind === 'installing') {
    const pct =
      state.total && state.total > 0
        ? Math.round((state.downloaded / state.total) * 100)
        : null;
    return (
      <div style={bannerStyle}>
        <span>Installing update{pct !== null ? ` — ${pct}%` : '…'}</span>
      </div>
    );
  }

  // idle / checking / up_to_date / error → a compact manual re-check control.
  return (
    <div style={rowStyle}>
      <GhostButton
        type="button"
        onClick={() => void check()}
        disabled={state.kind === 'checking'}
      >
        {state.kind === 'checking' ? 'Checking…' : 'Check for updates'}
      </GhostButton>
      {state.kind === 'up_to_date' && <span style={hintStyle}>✓ Up to date</span>}
      {state.kind === 'error' && (
        <span style={hintStyle}>Couldn’t check — try again</span>
      )}
    </div>
  );
}
