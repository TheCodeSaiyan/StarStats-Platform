import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Plane, SubStats, BeamChip, LogRow } from 'holo';
import type { ContractRunRow, ContractStepRow } from '@/lib/api';
import { fmtDuration, fmtNum } from '@/app/_components/widgets/kit/format';
import {
  outcomeText,
  outcomeBadgeVariant,
  stepStateLabel,
  stepBadgeVariant,
  runDurationSecs,
  type BadgeVariant,
} from '../_lib/outcome';

/**
 * One contract run.
 *
 * Every judgement here is lifted unchanged from the flat `RunCard`, because
 * each is load-bearing and easy to get subtly wrong:
 *
 *   - The outcome comes from `closed_by`, NOT the raw `state`. That
 *     distinction — an observed HUD banner versus a run inferred closed from a
 *     dead stream — is the whole point of `_lib/outcome`, and collapsing it
 *     would report a guess as a fact.
 *   - The contract NAME links only where it resolved unambiguously. An
 *     ambiguous name goes to the filtered candidate list, never to a guess.
 *   - Step `text` is the readable HUD banner wording, passed through verbatim
 *     by the server. Falls back to the raw objective id rather than an empty
 *     row — never prettified, because it is a log literal.
 *   - `partial_history` is surfaced, not hidden: a run whose start was never
 *     observed has a duration that means less than it looks like.
 */

/** `BadgeVariant` → the chip tone. `danger` is the system's `bad`. */
function tone(v: BadgeVariant): 'good' | 'warn' | 'bad' | undefined {
  if (v === 'ok') return 'good';
  if (v === 'warn') return 'warn';
  if (v === 'danger') return 'bad';
  return undefined;
}

export function RunPlane({
  run,
  href,
}: {
  run: ContractRunRow;
  href?: string | null;
}) {
  const durationSecs = runDurationSecs(run.accepted_at, run.closed_at);
  // `steps` is populated only because this page calls
  // `getContracts(..., include_steps=true)` — every other caller gets none.
  // Sorted defensively by `order`.
  const steps = [...run.steps].sort((a, b) => a.order - b.order);

  return (
    <Plane
      tilt="flat"
      cap={
        href ? (
          <Link href={href as Route} prefetch={false}>
            {run.name}
          </Link>
        ) : (
          run.name
        )
      }
      trailing={
        <BeamChip tone={tone(outcomeBadgeVariant(run.closed_by))}>
          {outcomeText(run.state, run.closed_by)}
        </BeamChip>
      }
      style={{ marginTop: 16 }}
    >
      <SubStats
        items={[
          {
            k: 'Steps',
            v: `${fmtNum(run.steps_complete)}/${fmtNum(run.step_count)}`,
          },
          ...(durationSecs !== null
            ? [{ k: 'Duration', v: fmtDuration(durationSecs) }]
            : []),
          ...(run.connected_server
            ? [{ k: 'Shard', v: run.connected_server }]
            : []),
        ]}
      />

      {/* Shipped copy, verbatim. A port redraws; it does not reword. */}
      {run.partial_history ? (
        <p className="hp-prose">
          history incomplete — this contract spans a log rotation
        </p>
      ) : null}

      {steps.length > 0 ? (
        <ol className="hp-steps hp-steps--objectives">
          {steps.map((step) => (
            <StepRow key={step.order} step={step} />
          ))}
        </ol>
      ) : null}
    </Plane>
  );
}

function StepRow({ step }: { step: ContractStepRow }) {
  // Verbatim HUD wording; the raw objective id is the fallback rather than an
  // empty row. Never rewritten — it is a log literal.
  const text = step.text ?? step.objective_id ?? '—';
  return (
    <li>
      <LogRow
        time={stepStateLabel(step.state)}
        event={text}
        tone={tone(stepBadgeVariant(step.state))}
      />
    </li>
  );
}
