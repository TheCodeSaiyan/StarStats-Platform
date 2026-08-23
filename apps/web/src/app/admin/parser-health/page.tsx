/**
 * Admin · Parser health.
 *
 * Surfaces event types that have stopped being produced while users stayed
 * active. Motivating incident: a `Game.log` shell tag renamed in the
 * ~2026-07-15 patch and `vehicle_stowed` fell from ~200/day to zero for
 * three weeks with nothing going red.
 *
 * The page LEADS with the last detector run, not the findings list. An empty
 * findings list is only good news if the detector actually ran — otherwise
 * "all clear" and "the alarm is broken" render identically, which is the
 * failure mode this whole feature exists to catch.
 *
 * `role="main"`: not set here — `app/admin/layout.tsx` already wraps the
 * whole /admin surface in a single `role="main"` div, so a second landmark
 * would violate the one-main-per-page rule the admin surface has kept since
 * M-W9.
 */

// Explicit React import: this repo's vitest uses the classic JSX runtime, so
// a JSX-rendering component 500s with "React is not defined" under test
// without it (the prod Next build uses the automatic runtime).
import React from 'react';
import { redirect } from 'next/navigation';
import { revalidatePath } from 'next/cache';
import {
  ApiCallError,
  acknowledgeParserHealthFinding,
  getAdminParserHealth,
  resolveParserHealthFinding,
  type ParserHealthFindingView,
  type ParserHealthRun,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { runStaleness } from './staleness';

function pct(n: number): string {
  return `${(n * 100).toFixed(2)}%`;
}

function fmtAge(hours: number | null): string {
  if (hours == null) return 'never';
  if (hours < 1) return 'less than an hour ago';
  if (hours < 48) return `${Math.round(hours)}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

export default async function AdminParserHealthPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/parser-health');

  let last_run: ParserHealthRun | null | undefined;
  let findings: ParserHealthFindingView[];
  try {
    ({ last_run, findings } = await getAdminParserHealth(session.token));
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/parser-health');
    }
    if (e instanceof ApiCallError && e.status === 403) redirect('/me');
    throw e;
  }

  async function acknowledgeAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/parser-health');
    const eventType = String(formData.get('event_type') ?? '');
    const note = String(formData.get('note') ?? '').trim();
    try {
      await acknowledgeParserHealthFinding(
        s.token,
        eventType,
        note || undefined,
      );
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/admin/parser-health');
      }
      if (e instanceof ApiCallError && e.status === 403) redirect('/me');
      throw e;
    }
    revalidatePath('/admin/parser-health');
  }

  async function resolveAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/parser-health');
    try {
      await resolveParserHealthFinding(
        s.token,
        String(formData.get('event_type') ?? ''),
      );
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/admin/parser-health');
      }
      if (e instanceof ApiCallError && e.status === 403) redirect('/me');
      throw e;
    }
    revalidatePath('/admin/parser-health');
  }

  const staleness = runStaleness(last_run, Date.now());
  const open = findings.filter((f) => f.finding.status === 'open');
  const other = findings.filter((f) => f.finding.status !== 'open');

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <header style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div className="ss-eyebrow">Admin · parser health</div>
        <h1 style={{ margin: 0, fontSize: 24, fontWeight: 600 }}>
          Classifier coverage
        </h1>
      </header>

      <RunBanner run={last_run} staleness={staleness} />

      <section style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <h2 style={{ margin: 0, fontSize: 18, fontWeight: 600 }}>
          Open findings{open.length > 0 ? ` (${open.length})` : ''}
        </h2>
        {open.length === 0 ? (
          <p style={{ color: 'var(--fg-muted)', margin: 0 }}>
            {staleness.state === 'ok'
              ? 'No event type has collapsed. Every type that had a baseline is still being produced at a comparable share.'
              : 'Nothing flagged — but the detector state above is not healthy, so treat this as unknown rather than clear.'}
          </p>
        ) : (
          <FindingsTable
            findings={open}
            acknowledgeAction={acknowledgeAction}
            resolveAction={resolveAction}
          />
        )}
      </section>

      {other.length > 0 && (
        <section style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <h2 style={{ margin: 0, fontSize: 18, fontWeight: 600 }}>
            Acknowledged and resolved ({other.length})
          </h2>
          <FindingsTable
            findings={other}
            acknowledgeAction={acknowledgeAction}
            resolveAction={resolveAction}
          />
        </section>
      )}
    </div>
  );
}

function RunBanner({
  run,
  staleness,
}: {
  run: ParserHealthRun | null | undefined;
  staleness: ReturnType<typeof runStaleness>;
}) {
  const copy: Record<typeof staleness.state, string> = {
    never:
      'The detector has not completed a pass yet. It runs a few minutes after startup, then daily.',
    stale: `Last pass ${fmtAge(staleness.ageHours)} — older than expected. Findings below may be out of date.`,
    failed: `Last pass failed: ${run?.error ?? 'unknown error'}`,
    ok: `Last pass ${fmtAge(staleness.ageHours)} · ${run?.types_examined ?? 0} event types examined`,
  };
  const healthy = staleness.state === 'ok';
  return (
    <div
      role="status"
      style={{
        border: '1px solid var(--border)',
        borderLeftWidth: 3,
        borderLeftColor: healthy ? 'var(--ok, #3fb950)' : 'var(--warn, #d29922)',
        borderRadius: 0,
        padding: '10px 14px',
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
      }}
    >
      <strong style={{ fontSize: 13 }}>
        {healthy ? 'Detector healthy' : 'Detector needs attention'}
      </strong>
      <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
        {copy[staleness.state]}
      </span>
    </div>
  );
}

function FindingsTable({
  findings,
  acknowledgeAction,
  resolveAction,
}: {
  findings: ParserHealthFindingView[];
  acknowledgeAction: (formData: FormData) => Promise<void>;
  resolveAction: (formData: FormData) => Promise<void>;
}) {
  return (
    <div style={{ overflowX: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
        <thead>
          <tr style={{ textAlign: 'left' }}>
            <th style={{ padding: '6px 8px' }}>Event type</th>
            <th style={{ padding: '6px 8px' }}>Severity</th>
            <th style={{ padding: '6px 8px' }}>Share before → after</th>
            <th style={{ padding: '6px 8px' }}>Users affected</th>
            <th style={{ padding: '6px 8px' }}>Status</th>
            <th style={{ padding: '6px 8px' }}>Actions</th>
          </tr>
        </thead>
        <tbody>
          {findings.map((f) => {
            const d = f.finding;
            return (
            <React.Fragment key={d.event_type}>
            <tr style={{ borderTop: '1px solid var(--border)' }}>
              <td style={{ padding: '6px 8px', fontFamily: 'var(--font-mono)' }}>
                {d.event_type}
              </td>
              <td style={{ padding: '6px 8px' }}>{d.severity}</td>
              <td style={{ padding: '6px 8px' }}>
                {pct(d.share_baseline)} &rarr; {pct(d.share_recent)}
                <span style={{ color: 'var(--fg-muted)' }}>
                  {' '}
                  ({d.baseline_events.toLocaleString()} &rarr;{' '}
                  {d.recent_events.toLocaleString()} events)
                </span>
              </td>
              <td style={{ padding: '6px 8px' }}>
                {d.affected_handles} of {d.carried_handles} still-active
                {d.carried_handles === 1 && (
                  <span style={{ color: 'var(--fg-muted)' }}>
                    {' '}
                    &middot; single user, weak evidence
                  </span>
                )}
              </td>
              <td style={{ padding: '6px 8px' }}>
                {d.status}
                {d.note && (
                  <div style={{ color: 'var(--fg-muted)' }}>{d.note}</div>
                )}
              </td>
              <td style={{ padding: '6px 8px' }}>
                {d.status !== 'resolved' && (
                  <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                    {d.status === 'open' && (
                      <form action={acknowledgeAction}>
                        <input type="hidden" name="event_type" value={d.event_type} />
                        <input
                          type="text"
                          name="note"
                          placeholder="Why is this expected?"
                          aria-label={`Note for ${d.event_type}`}
                          style={{ marginRight: 6, fontSize: 12 }}
                        />
                        <ConfirmSubmitButton className="ss-btn">
                          Acknowledge
                        </ConfirmSubmitButton>
                      </form>
                    )}
                    <form action={resolveAction}>
                      <input type="hidden" name="event_type" value={d.event_type} />
                      <ConfirmSubmitButton
                        className="ss-btn"
                        confirm={`Mark ${d.event_type} resolved?`}
                      >
                        Resolve
                      </ConfirmSubmitButton>
                    </form>
                  </div>
                )}
              </td>
            </tr>
            {f.candidates.length > 0 && (
              <tr>
                <td colSpan={6} style={{ padding: '0 8px 8px' }}>
                  <div style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
                    Log tags first seen around{' '}
                    {d.last_event_at
                      ? new Date(d.last_event_at).toISOString().slice(0, 10)
                      : 'the collapse'}
                    , most likely cause first:
                    <ul style={{ margin: '4px 0 0', paddingLeft: 18 }}>
                      {f.candidates.map((c) => (
                        <li key={c.shell_tag}>
                          <code>{c.shell_tag}</code> &middot;{' '}
                          {c.occurrences.toLocaleString()} lines
                          {c.handle_count > 1 &&
                            ` · reported by ${c.handle_count} users`}
                        </li>
                      ))}
                    </ul>
                  </div>
                </td>
              </tr>
            )}
            </React.Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
