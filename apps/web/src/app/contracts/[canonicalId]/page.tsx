/**
 * Contract detail page. Fetches one contract by canonical id and
 * renders its structured extraction: identity, reward/fees, timeframe,
 * objectives, captured DETAILS attributes, and the ordered step list.
 *
 * Location strings (step locations + "…LOCATION" attribute values) are
 * wrapped in `<EntityLink>` against the locations catalogue so they
 * cross-link into the KB where a match exists.
 *
 * Server component. `not_found` → 404 page; a transient backend error
 * throws so the Next error boundary can offer a retry (not a
 * misleading permanent 404). `raw_text` never reaches this layer — the
 * server projects only the structured fields.
 */

import React from 'react';
import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import { notFound } from 'next/navigation';
import {
  getContractDetail,
  formatReward,
  formatAdditionalRewards,
  missionTimerBadge,
  type ContractDetail,
} from '@/lib/contracts';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';

interface PageProps {
  params: Promise<{ canonicalId: string }>;
}

export async function generateMetadata(props: PageProps): Promise<Metadata> {
  const { canonicalId } = await props.params;
  const outcome = await getContractDetail(canonicalId);
  if (outcome.kind !== 'ok') return { title: 'Not found — Contracts' };
  const name = outcome.contract.contract.display_name ?? canonicalId;
  return { title: `${name} — Contracts` };
}

export default async function ContractDetailPage(props: PageProps) {
  const { canonicalId } = await props.params;
  const outcome = await getContractDetail(canonicalId);
  if (outcome.kind === 'not_found') notFound();
  if (outcome.kind === 'error') {
    throw new Error(`Failed to load contract ${canonicalId}: ${outcome.reason}`);
  }

  const detail = outcome.contract;
  const c = detail.contract;
  const steps = detail.steps ?? [];

  // No reference catalogue is fetched here. Entities arrive already
  // resolved on the detail payload, so a render costs no bundle read —
  // the vehicles bundle alone is ~4 MB.

  const title = c.display_name ?? canonicalId;
  const reward = formatReward(c.reward);
  // Non-aUEC awards (MG Scrip, ...). Read out of the DETAILS prose, so a
  // contract can have these and no aUEC figure at all — the Reward
  // section must open for them on their own.
  const additionalRewards = formatAdditionalRewards(c.reward);
  // Resolved KB entities, keyed for lookup by the values rendered below.
  const entities = detail.entities ?? [];
  const entityKey = (kind: string, value: string) =>
    `${kind}:${value.trim().replace(/\s+/g, ' ').toLowerCase()}`;
  const entityByKey = new Map(entities.map((e) => [entityKey(e.kind, e.raw_value), e]));

  const timerBadge = missionTimerBadge(c.timeframe);

  const readouts: { k: string; v: React.ReactNode }[] = [];
  if (reward) readouts.push({ k: 'reward', v: reward });
  if (c.confidence_score != null)
    readouts.push({ k: 'confidence', v: `${Math.round(c.confidence_score * 100)}%` });
  if (c.patch_version) readouts.push({ k: 'patch', v: c.patch_version });

  const contextParts = [c.contract_type, c.subcategory].filter(Boolean) as string[];

  return (
    <main style={{ maxWidth: 920, display: 'flex', flexDirection: 'column', gap: 20 }}>
      <Link
        href={'/contracts' as Route}
        prefetch={false}
        style={{ fontSize: 13, color: 'var(--accent)', textDecoration: 'none' }}
      >
        ← Contracts
      </Link>

      <InstrumentStrip
        size="hero"
        title={
          <h1 className="hud-tile__title" style={{ margin: 0, fontSize: 'inherit' }}>
            {title}
          </h1>
        }
        context={contextParts.join(' · ') || undefined}
        readouts={readouts}
      />

      {/* Identity strip */}
      <section className="ss-card ss-card-pad">
        <dl style={identityGridStyle}>
          <Field label="Issuer" value={c.issuer} />
          <Field label="Faction" value={c.faction} />
          <Field label="Legal status" value={c.legal_status} />
          <Field label="Gameplay loop" value={c.gameplay_loop} />
          <Field label="Required reputation" value={c.required_reputation} />
          <Field label="Reputation rank" value={c.reputation_rank} />
        </dl>
      </section>

      {/* Requirements — deliberately ABOVE reward. They decide whether a
          player can take the contract at all, so they matter before the
          payout does. Frequently stated only in the description, which
          is why they are extracted rather than read off a panel. */}
      {c.requirements && c.requirements.length > 0 && (
        <section>
          <SectionHeading>Requirements</SectionHeading>
          <ul style={listStyle}>
            {c.requirements.map((req, i) => (
              <li key={i} style={{ fontSize: 13, color: 'var(--fg-muted)' }}>
                {req}
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Reward + fees */}
      {(reward ||
        additionalRewards.length > 0 ||
        (c.fees && c.fees.length > 0) ||
        c.net_estimated_profit != null) && (
        <section>
          <SectionHeading>Reward</SectionHeading>
          {reward && <p style={{ fontSize: 16, color: 'var(--accent)', margin: '0 0 8px' }}>{reward}</p>}
          {additionalRewards.length > 0 && (
            <ul style={listStyle}>
              {additionalRewards.map((line, i) => (
                <li key={i} style={{ fontSize: 14, color: 'var(--accent)' }}>
                  {line.text}
                </li>
              ))}
            </ul>
          )}
          {c.net_estimated_profit != null && (
            <p style={{ fontSize: 13, color: 'var(--fg-muted)', margin: '0 0 8px' }}>
              Net estimated profit: {c.net_estimated_profit.toLocaleString()} aUEC
            </p>
          )}
          {c.fees && c.fees.length > 0 && (
            <ul style={listStyle}>
              {c.fees.map((fee, i) => (
                <li key={i} style={{ fontSize: 13, color: 'var(--fg-muted)' }}>
                  {(fee.type ?? 'fee')}: {fee.amount != null ? `${fee.amount.toLocaleString()} ${fee.currency ?? 'aUEC'}` : '—'}
                  {fee.refundable != null && (
                    <span style={{ color: 'var(--fg-dim)' }}>
                      {' '}({fee.refundable ? 'refundable' : 'non-refundable'})
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      {/* Timeframe */}
      {c.timeframe && (c.timeframe.deadline_text || c.timeframe.duration_minutes != null || c.timeframe.has_time_limit != null) && (
        <section>
          <SectionHeading>Timeframe</SectionHeading>
          <p style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', fontSize: 13, color: 'var(--fg-muted)', margin: 0 }}>
            {timerBadge && <Chip tone={timerBadge.tone}>⏱ {timerBadge.label}</Chip>}
            {(() => {
              // Concrete details (deadline / duration), independent of the badge.
              const details = [
                c.timeframe.deadline_text,
                c.timeframe.duration_minutes != null ? `${c.timeframe.duration_minutes} min` : null,
              ]
                .filter(Boolean)
                .join(' · ');
              const text = c.timeframe.has_time_limit === false ? 'No time limit' : details;
              if (text) return <span>{text}</span>;
              // Timed but no concrete detail → the badge already says it; only
              // fall back to bare text when there's no badge either.
              return timerBadge ? null : <span>Time-limited</span>;
            })()}
          </p>
        </section>
      )}

      {/* Warnings */}
      {(c.failure_penalty || c.cargo_loss_penalty || c.rep_loss_warning) && (
        <section>
          <SectionHeading>Warnings</SectionHeading>
          <ul style={listStyle}>
            {c.failure_penalty && <li style={warnItemStyle}>{c.failure_penalty}</li>}
            {c.cargo_loss_penalty && <li style={warnItemStyle}>{c.cargo_loss_penalty}</li>}
            {c.rep_loss_warning && <li style={warnItemStyle}>{c.rep_loss_warning}</li>}
          </ul>
        </section>
      )}

      {/* Primary objectives */}
      {c.primary_objectives && c.primary_objectives.length > 0 && (
        <section>
          <SectionHeading>Primary objectives</SectionHeading>
          <ul style={listStyle}>
            {c.primary_objectives.map((o, i) => (
              <li key={i} style={{ fontSize: 13, color: 'var(--fg)' }}>{o}</li>
            ))}
          </ul>
        </section>
      )}

      {/* Captured details */}
      {c.attributes && c.attributes.length > 0 && (
        <section>
          <SectionHeading>Details</SectionHeading>
          <dl style={identityGridStyle}>
            {c.attributes.map((attr, i) => (
              <div key={i}>
                <dt style={dtStyle}>{attr.label ?? '—'}</dt>
                <dd style={ddStyle}>
                  {isLocationLabel(attr.label) && attr.value ? (
                    // Displayed text is always the verbatim value; the
                    // link is added only where the entity resolved.
                    <KbValue entity={entityByKey.get(entityKey('location', attr.value))}>
                      {attr.value}
                    </KbValue>
                  ) : (
                    (attr.value ?? '—')
                  )}
                </dd>
              </div>
            ))}
          </dl>
        </section>
      )}

      {/* Steps */}
      {steps.length > 0 && (
        <section>
          <SectionHeading>Steps</SectionHeading>
          <ol style={{ ...listStyle, listStyle: 'none', paddingLeft: 0 }}>
            {[...steps]
              .sort((a, b) => (a.order ?? 0) - (b.order ?? 0))
              .map((step, i) => (
                <li
                  key={i}
                  className="hud-tile"
                  style={{ padding: '10px 12px', marginBottom: 8, display: 'flex', flexDirection: 'column', gap: 4 }}
                >
                  <span style={{ display: 'flex', gap: 8, alignItems: 'baseline', flexWrap: 'wrap' }}>
                    <span style={{ fontSize: 12, color: 'var(--fg-dim)' }}>
                      {step.order != null ? `${step.order}.` : '•'}
                    </span>
                    <span style={{ fontSize: 14, fontWeight: 600 }}>
                      {step.summary ?? step.step_type ?? 'Step'}
                    </span>
                    {step.step_type && <Chip>{step.step_type}</Chip>}
                    {step.risk && <Chip tone={riskTone(step.risk)}>{step.risk} risk</Chip>}
                    {step.optional && <Chip>optional</Chip>}
                    {step.guidance && <Chip>tip step</Chip>}
                  </span>
                  {step.location && (
                    <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                      {/* Deliberately plain text. Step locations are
                          descriptive phrasing ("Caterpillar wreck site
                          near microTech"), not names the KB can match —
                          the canonical names live in `entities`. */}
                      <span style={{ color: 'var(--fg-dim)' }}>location: </span>
                      {step.location}
                    </span>
                  )}
                  {step.entities && step.entities.length > 0 && (
                    <span
                      style={{
                        display: 'flex',
                        gap: 6,
                        flexWrap: 'wrap',
                        fontSize: 12,
                        color: 'var(--fg-muted)',
                      }}
                    >
                      <span style={{ color: 'var(--fg-dim)' }}>refers to: </span>
                      {step.entities.map((se, j) =>
                        se.kind && se.name ? (
                          <KbValue
                            key={j}
                            entity={entityByKey.get(entityKey(se.kind, se.name))}
                          >
                            {se.name}
                          </KbValue>
                        ) : null,
                      )}
                    </span>
                  )}
                  {step.tip && (
                    <span style={{ fontSize: 12, color: 'var(--fg-muted)', fontStyle: 'italic' }}>
                      {step.tip}
                    </span>
                  )}
                  {step.failure_condition && (
                    <span style={{ fontSize: 12, color: 'var(--danger, #e5484d)' }}>
                      Fails if: {step.failure_condition}
                    </span>
                  )}
                  {requirementLine(step) && (
                    <span style={{ fontSize: 12, color: 'var(--fg-dim)' }}>
                      {requirementLine(step)}
                    </span>
                  )}
                </li>
              ))}
          </ol>
        </section>
      )}

    </main>
  );
}

// ---------------------------------------------------------------------
// Small presentational helpers.
// ---------------------------------------------------------------------

/** Render a value, linked into the knowledge base as far as the server
 *  could justify — and no further.
 *
 *  Three tiers, the same rule the contract-name links follow:
 *
 *  | Registry matches | Destination |
 *  |---|---|
 *  | exactly one | that KB entry |
 *  | several      | a KB search for the name |
 *  | none         | plain text |
 *
 *  The middle tier matters more than it looks. The registry holds
 *  genuine duplicates — "Sunset Berries" exists three times with slugs
 *  `sunset-berries`, `-2` and `-3` — so refusing to link them at all
 *  was the strict rule doing the wrong thing for the right reason. A
 *  search cannot assert a wrong identity, and it lands somewhere useful.
 *
 *  No match stays plain text deliberately: a search we have not
 *  confirmed has results is a dead end, and guessing one would be
 *  exactly what this rule exists to prevent. */
function KbValue({
  entity,
  children,
}: {
  entity?: {
    ref_slug?: string | null;
    ref_category?: string | null;
    ref_match_count?: number | null;
    kind?: string | null;
  };
  children: React.ReactNode;
}) {
  if (!entity) return <>{children}</>;

  const linkStyle = { color: 'var(--accent)', textDecoration: 'none' };

  if (entity.ref_slug && entity.ref_category) {
    return (
      <Link
        href={`/kb/${entity.ref_category}/${entity.ref_slug}` as Route}
        prefetch={false}
        style={linkStyle}
      >
        {children}
      </Link>
    );
  }

  // Several matches: we know the KB holds entries for this name but not
  // which one is meant, so send the reader to the candidates.
  const count = entity.ref_match_count ?? 0;
  if (count > 1 && entity.kind && typeof children === 'string') {
    const qs = new URLSearchParams({ q: children }).toString();
    return (
      <Link
        href={`/kb/${entity.kind}?${qs}` as Route}
        prefetch={false}
        style={{ ...linkStyle, textDecorationLine: 'underline', textDecorationStyle: 'dotted' }}
        title={`${count} knowledge-base entries share this name`}
      >
        {children}
      </Link>
    );
  }

  return <>{children}</>;
}

function Field({ label, value }: { label: string; value?: string | null }) {
  if (!value) return null;
  return (
    <div>
      <dt style={dtStyle}>{label}</dt>
      <dd style={ddStyle}>{value}</dd>
    </div>
  );
}

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <h2
      style={{
        fontSize: 12,
        color: 'var(--fg-dim)',
        letterSpacing: '0.06em',
        textTransform: 'uppercase',
        margin: '0 0 8px',
      }}
    >
      {children}
    </h2>
  );
}

function Chip({ children, tone }: { children: React.ReactNode; tone?: string }) {
  return (
    <span
      style={{
        fontSize: 10,
        padding: '2px 6px',
        borderRadius: 999,
        border: `1px solid ${tone ?? 'var(--border)'}`,
        color: tone ?? 'var(--fg-muted)',
        letterSpacing: '0.03em',
        textTransform: 'uppercase',
      }}
    >
      {children}
    </span>
  );
}

function riskTone(risk: string): string {
  const r = risk.toLowerCase();
  if (r === 'high') return 'var(--danger, #e5484d)';
  if (r === 'medium') return 'var(--warn, #f5a623)';
  return 'var(--border)';
}

/** True for DETAILS labels that name a place — used to decide whether
 *  to wrap the value in a location EntityLink. */
function isLocationLabel(label?: string | null): boolean {
  if (!label) return false;
  return /location|system|destination|area/i.test(label);
}

/** Compact "requires X, Y" line from a step's required_* fields. */
function requirementLine(step: ContractDetail['steps'][number]): string | null {
  const reqs = [
    step.required_item,
    step.required_cargo,
    step.required_vehicle,
    step.required_equipment,
  ].filter(Boolean) as string[];
  return reqs.length > 0 ? `requires: ${reqs.join(', ')}` : null;
}

const identityGridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
  gap: 12,
  margin: 0,
};

const dtStyle: React.CSSProperties = {
  fontSize: 11,
  color: 'var(--fg-dim)',
  letterSpacing: '0.03em',
};

const ddStyle: React.CSSProperties = {
  fontSize: 13,
  color: 'var(--fg)',
  margin: '2px 0 0',
};

const listStyle: React.CSSProperties = {
  margin: 0,
  paddingLeft: 18,
  display: 'flex',
  flexDirection: 'column',
  gap: 4,
};

const warnItemStyle: React.CSSProperties = {
  fontSize: 13,
  color: 'var(--danger, #e5484d)',
};
