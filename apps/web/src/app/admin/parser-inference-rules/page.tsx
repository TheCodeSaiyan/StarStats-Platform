/**
 * Admin · Inference rules management (Task 7).
 *
 * Lists every published inference rule (enabled + retracted) so a
 * rule author can retract a bad rule or re-enable a previously
 * retracted one, and links out to the `/new` authoring page (Task 6).
 *
 * Retract/re-enable both go through the same `publishAdminInferenceRule`
 * upsert endpoint used by the authoring form: a row is just re-POSTed
 * with `enabled` flipped. Unlike the #3 parser-rules page (a handful of
 * flat scalar fields), `InferenceRuleDto` is a nested structure
 * (trigger/emits/followups patterns), so instead of round-tripping each
 * field through its own hidden input, the row's full `definition` is
 * carried as one hidden JSON-stringified input and re-parsed in the
 * action — same approach the Task 6 authoring form uses to post its
 * own `definition` field.
 *
 * `role="main"`: not set here — `app/admin/layout.tsx` already wraps
 * the whole /admin surface in a single `role="main"` div, so a second
 * landmark on this page would violate the one-main-per-page rule the
 * admin surface has kept since M-W9.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component 500s with "React is not
// defined" under test without it (the prod Next build uses the
// automatic runtime and doesn't need it).
import React from 'react';
import type { Route } from 'next';
import Link from 'next/link';
import { redirect } from 'next/navigation';
import { revalidatePath } from 'next/cache';
import {
  ApiCallError,
  getAdminInferenceRules,
  publishAdminInferenceRule,
  type AdminInferenceRuleRow,
  type PublishInferenceRuleRequest,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';

interface PageProps {
  searchParams: Promise<{ published?: string; error?: string }>;
}

export default async function AdminParserInferenceRulesPage(
  props: PageProps,
) {
  const session = await getSession();
  if (!session) {
    redirect('/auth/login?next=/admin/parser-inference-rules');
  }
  const { published, error: toggleError } = await props.searchParams;

  let rules: AdminInferenceRuleRow[];
  try {
    ({ rules } = await getAdminInferenceRules(session.token));
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/parser-inference-rules');
    }
    if (e instanceof ApiCallError && e.status === 403) redirect('/me');
    throw e;
  }

  async function toggleInferenceRuleAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/parser-inference-rules');

    // The full rule definition round-trips as one JSON blob rather
    // than per-field hidden inputs (the DTO is a nested
    // trigger/emits/followups structure, not flat scalars).
    let definition: PublishInferenceRuleRequest;
    try {
      definition = JSON.parse(String(formData.get('definition') ?? '{}'));
    } catch {
      redirect(
        '/admin/parser-inference-rules?error=invalid_definition' as Route,
      );
    }

    try {
      await publishAdminInferenceRule(s.token, {
        ...definition,
        enabled: String(formData.get('next_enabled')) === 'true',
      });
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/admin/parser-inference-rules');
      }
      if (e instanceof ApiCallError && e.status === 403) redirect('/me');
      if (e instanceof ApiCallError) {
        redirect(
          `/admin/parser-inference-rules?error=${encodeURIComponent(e.body.error)}` as Route,
        );
      }
      throw e;
    }
    revalidatePath('/admin/parser-inference-rules');
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <header
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div className="ss-eyebrow">Admin · inference rules</div>
          <h1 style={{ margin: 0, fontSize: 24, fontWeight: 600 }}>
            Published inference rules
          </h1>
        </div>
        <Link
          href={'/admin/parser-inference-rules/new' as Route}
          className="hp-btn hp-btn--ghost"
        >
          New inference rule
        </Link>
      </header>

      {published && (
        <div
          role="status"
          className="hp-chip good"
          style={{ alignSelf: 'flex-start' }}
        >
          Published {published}
        </div>
      )}
      {toggleError && (
        <div
          role="status"
          className="hp-chip bad"
          style={{ alignSelf: 'flex-start' }}
        >
          Action failed: {toggleError}
        </div>
      )}

      {rules.length === 0 ? (
        <p style={{ color: 'var(--fg-muted)' }}>
          No inference rules published yet.
        </p>
      ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th align="left">rule_id</th>
              <th align="left">trigger → emits</th>
              <th align="left">confidence</th>
              <th align="left">window_secs</th>
              <th>enabled</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.map((r) => (
              <tr
                key={r.rule_id}
                style={{ borderTop: '1px solid var(--border)' }}
              >
                <td style={{ fontFamily: 'var(--font-mono, monospace)' }}>
                  {r.rule_id}
                </td>
                <td>
                  {r.definition.trigger.event_type} →{' '}
                  {r.definition.emits.event_type}
                </td>
                <td>{r.definition.confidence}</td>
                <td>{r.definition.window_secs}</td>
                <td align="center">{r.enabled ? '✓' : '—'}</td>
                <td align="right">
                  <form
                    action={toggleInferenceRuleAction}
                    style={{ display: 'inline' }}
                  >
                    <input
                      type="hidden"
                      name="definition"
                      value={JSON.stringify(r.definition)}
                    />
                    <input
                      type="hidden"
                      name="next_enabled"
                      value={r.enabled ? 'false' : 'true'}
                    />
                    <ConfirmSubmitButton
                      confirm={
                        r.enabled
                          ? 'Retract this inference rule from all collectors?'
                          : 'Re-enable this inference rule?'
                      }
                      className="hp-btn hp-btn--ghost"
                      pendingLabel="…"
                    >
                      {r.enabled ? 'Retract' : 'Enable'}
                    </ConfirmSubmitButton>
                  </form>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
