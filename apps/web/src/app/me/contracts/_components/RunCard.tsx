import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import type { ContractRunRow, ContractStepRow } from '@/lib/api';
import { ReadoutGroup, type Readout } from '@/app/_components/widgets/kit/archetypes';
import { fmtDuration, fmtNum } from '@/app/_components/widgets/kit/format';
import {
  outcomeText,
  outcomeBadgeVariant,
  stepStateLabel,
  stepBadgeVariant,
  runDurationSecs,
} from '../_lib/outcome';

/** `BadgeVariant` → the CSS custom property carrying that colour token. */
const VARIANT_COLOR: Record<string, string> = {
  ok: 'var(--ok)',
  warn: 'var(--warn)',
  danger: 'var(--danger)',
  '': 'var(--fg-dim)',
};

/**
 * One contract run: name, outcome (with *why* — see the `_lib/outcome`
 * doc), duration, step progress, and — when `include_steps` was
 * requested — the full per-step objective text. The per-step text is the
 * entire reason this page exists (see `page.tsx`'s doc); every other
 * field here is context around it.
 */
export function RunCard({ run, href }: { run: ContractRunRow; href?: string | null }) {
  const variant = outcomeBadgeVariant(run.closed_by);
  const badgeClass = variant ? `ss-badge ss-badge--${variant}` : 'ss-badge';
  const durationSecs = runDurationSecs(run.accepted_at, run.closed_at);

  const readouts: Readout[] = [
    { label: 'steps', value: `${fmtNum(run.steps_complete)}/${fmtNum(run.step_count)}` },
    ...(durationSecs !== null
      ? [{ label: 'duration', value: fmtDuration(durationSecs) } as Readout]
      : []),
    ...(run.connected_server
      ? [{ label: 'shard', value: run.connected_server, secondary: true } as Readout]
      : []),
  ];

  // `steps` is only populated when the page's `getContracts(..., true)`
  // call opted in — see `ContractRunRow`'s doc. Sorted defensively by
  // `order`, same rationale as `byAcceptedDesc` for the run list.
  const steps = [...run.steps].sort((a, b) => a.order - b.order);

  return (
    <article className="hud-tile">
      <div className="hud-tile__hd">
        <span
          className="hud-trunc"
          style={{ flex: 1, minWidth: 0, fontWeight: 700, color: 'var(--fg)' }}
        >
          {/* Linked only where the name resolved. An ambiguous name
              points at the filtered candidate list, never a guess. */}
          {href ? (
            <Link href={href as Route} prefetch={false} style={{ color: 'inherit' }}>
              {run.name}
            </Link>
          ) : (
            run.name
          )}
        </span>
        <span className={badgeClass} style={{ flexShrink: 0 }}>
          {outcomeText(run.state, run.closed_by)}
        </span>
      </div>
      <div
        className="hud-tile__body"
        style={{ marginTop: 4, display: 'flex', flexDirection: 'column', gap: 6 }}
      >
        <ReadoutGroup readouts={readouts} />
        {run.partial_history && (
          <p className="hud-note">history incomplete — this contract spans a log rotation</p>
        )}
        {steps.length > 0 && (
          <ol
            style={{
              listStyle: 'none',
              margin: 0,
              padding: 0,
              display: 'flex',
              flexDirection: 'column',
              gap: 3,
            }}
          >
            {steps.map((step) => (
              <StepRow key={step.order} step={step} />
            ))}
          </ol>
        )}
      </div>
    </article>
  );
}

function StepRow({ step }: { step: ContractStepRow }) {
  const color = VARIANT_COLOR[stepBadgeVariant(step.state)];
  // `text` is the readable HUD banner wording, passed through verbatim by
  // the server (never trimmed/cased/rewritten — see `ContractStepRow`'s
  // doc); fall back to the raw objective id rather than an empty row when
  // it's null.
  const text = step.text ?? step.objective_id ?? '—';

  return (
    <li style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
      <span
        style={{
          flexShrink: 0,
          width: 20,
          textAlign: 'right',
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          color: 'var(--fg-dim)',
        }}
      >
        {step.order + 1}
      </span>
      <span
        style={{
          flexShrink: 0,
          width: 80,
          // "IN PROGRESS" is the longest label and measures close to 68px
          // at this size; without nowrap it would wrap to two lines inside
          // a fixed-width box and desynchronise this row's height from its
          // neighbours.
          whiteSpace: 'nowrap',
          fontSize: 10,
          textTransform: 'uppercase',
          letterSpacing: '.04em',
          color,
        }}
      >
        {stepStateLabel(step.state)}
      </span>
      {/* Deliberately NOT `.hud-trunc`. The objective text is the entire
          reason this page exists, and at 375px this column is ~240px —
          about 34 characters — so truncating clips most real objectives
          with no way to recover them (hover does nothing on touch). The
          widget-tile no-scroll contract binds dashboard tiles, not a
          dedicated history page, so wrapping is free here: the span is its
          own flex column, and a second line aligns under the first rather
          than under the order/state gutters. */}
      <span style={{ flex: 1, minWidth: 0, fontSize: 12, color: 'var(--fg-muted)' }}>
        {text}
      </span>
    </li>
  );
}
