/**
 * Re-ingest card — re-walks every rotated `Game-*.log` on disk and
 * feeds each line back through the current classifier. Distinct from
 * the Re-parse card next to it:
 *
 *   - Re-parse only sees the local event store. It can't recover
 *     events that an older parser silently dropped (returned `None`
 *     from classify), because those lines never made it into the
 *     events table in the first place.
 *   - Re-ingest goes back to the source-of-truth — the rotated log
 *     files in the Star Citizen install dir — and feeds them through
 *     the v0.3.2+ classifier. Idempotency keys on the events table
 *     dedupe so the run is safe to repeat.
 *
 * Typical use: after upgrading to a release that added a new event
 * variant (e.g. PlayerDeath in v0.3.1), click Re-ingest to recover
 * historical occurrences, then click Re-parse to back-fill zone
 * enrichment on the new rows.
 */

import { useState } from 'react';
import { api, type ReingestStats } from '../api';
import { GhostButton, TrayCard } from './tray/primitives';

type State =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'done'; result: ReingestStats }
  | { kind: 'error'; message: string };

export function ReingestCard() {
  const [state, setState] = useState<State>({ kind: 'idle' });

  const handleClick = async () => {
    if (
      !window.confirm(
        'Re-walk every rotated Game.log file and feed each line through the current classifier?\n\n' +
          'This may take a few minutes if you have many archived logs. Idempotency keys dedupe ' +
          'already-known events, so it only adds NEW classifications. Click Re-parse afterwards ' +
          'to back-fill zone enrichment on the new rows.',
      )
    ) {
      return;
    }
    setState({ kind: 'running' });
    try {
      const result = await api.reingestRotatedLogs();
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
      title="Re-ingest rotated logs"
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
          Re-walks every rotated <code>Game-*.log</code> file on disk through
          the current classifier. Use after a parser update that added new
          event types — recovers historical events that older parsers silently
          dropped. Idempotent: safe to click again, but slower than Re-parse.
        </p>

        <div>
          <GhostButton
            type="button"
            onClick={handleClick}
            disabled={state.kind === 'running'}
          >
            {state.kind === 'running' ? 'Re-ingesting…' : 'Re-ingest now'}
          </GhostButton>
        </div>

        <ReingestStatusLine state={state} />
      </div>
    </TrayCard>
  );
}

function ReingestStatusLine({ state }: { state: State }) {
  if (state.kind === 'idle') {
    return null;
  }

  if (state.kind === 'running') {
    return (
      <div style={{ fontSize: 11, color: 'var(--fg-muted)' }}>
        Walking rotated logs… (may take a few minutes)
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

  const { files_walked, files_failed, lines_processed, events_recognised } =
    state.result;
  const noNewEvents = events_recognised === 0;
  const failedSuffix =
    files_failed > 0 ? `, ${files_failed.toLocaleString()} failed` : '';
  return (
    <div
      style={{
        fontSize: 11,
        color: noNewEvents ? 'var(--fg-muted)' : 'var(--ok)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      Walked {files_walked.toLocaleString()} file{files_walked === 1 ? '' : 's'}
      {failedSuffix}, processed {lines_processed.toLocaleString()} lines,
      recognised {events_recognised.toLocaleString()} event
      {events_recognised === 1 ? '' : 's'}.
      {!noNewEvents && ' Click Re-parse next to enrich zones.'}
    </div>
  );
}
