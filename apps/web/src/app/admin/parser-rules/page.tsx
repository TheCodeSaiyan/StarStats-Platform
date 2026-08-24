/**
 * Admin · Parser rules management (Task 6).
 *
 * Lists every published parser rule (enabled + retracted) so a rule
 * author can retract a bad rule or re-enable a previously-retracted
 * one without touching the manifest directly. Retract/re-enable both
 * go through the same `publishAdminParserRule` upsert endpoint used
 * by the parser-submissions "publish" flow — a row is just re-POSTed
 * with `enabled` flipped, so every field is round-tripped through
 * hidden form inputs rather than looked up server-side again.
 *
 * `role="main"`: not set here — `app/admin/layout.tsx` already wraps
 * the whole /admin surface in a single `role="main"` div (see the
 * comment there), so a second landmark on this page would violate
 * the one-main-per-page rule the admin surface has kept since M-W9.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component 500s with "React is not
// defined" under test without it (the prod Next build uses the
// automatic runtime and doesn't need it).
import React from 'react';
import { redirect } from 'next/navigation';
import { revalidatePath } from 'next/cache';
import {
  ApiCallError,
  getAdminParserRules,
  publishAdminParserRule,
  type AdminParserRuleRow,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';

export default async function AdminParserRulesPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/parser-rules');

  let rules: AdminParserRuleRow[];
  try {
    ({ rules } = await getAdminParserRules(session.token));
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/parser-rules');
    }
    if (e instanceof ApiCallError && e.status === 403) redirect('/me');
    throw e;
  }

  async function toggleRuleAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/parser-rules');
    // Re-publish the same rule with `enabled` flipped. All fields are
    // carried on the form as hidden inputs so the row round-trips
    // without a second lookup.
    try {
      await publishAdminParserRule(s.token, {
        rule_id: String(formData.get('rule_id') ?? ''),
        event_name: String(formData.get('event_name') ?? ''),
        match_kind: String(formData.get('match_kind') ?? 'event_name'),
        body_regex: String(formData.get('body_regex') ?? ''),
        fields: String(formData.get('fields') ?? '')
          .split('\n')
          .map((f) => f.trim())
          .filter(Boolean),
        enabled: String(formData.get('next_enabled')) === 'true',
      });
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/admin/parser-rules');
      }
      if (e instanceof ApiCallError && e.status === 403) redirect('/me');
      throw e;
    }
    revalidatePath('/admin/parser-rules');
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <header style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div className="ss-eyebrow">Admin · parser rules</div>
        <h1 style={{ margin: 0, fontSize: 24, fontWeight: 600 }}>
          Published rules
        </h1>
      </header>

      {rules.length === 0 ? (
        <p style={{ color: 'var(--fg-muted)' }}>No rules published yet.</p>
      ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th align="left">rule_id</th>
              <th align="left">event</th>
              <th align="left">match</th>
              <th>enabled</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.map((r) => (
              <tr key={r.rule_id} style={{ borderTop: '1px solid var(--border)' }}>
                <td style={{ fontFamily: 'var(--font-mono, monospace)' }}>
                  {r.rule_id}
                </td>
                <td>{r.event_name}</td>
                <td>{r.match_kind}</td>
                <td align="center">{r.enabled ? '✓' : '—'}</td>
                <td align="right">
                  <form action={toggleRuleAction} style={{ display: 'inline' }}>
                    <input type="hidden" name="rule_id" value={r.rule_id} />
                    <input type="hidden" name="event_name" value={r.event_name} />
                    <input type="hidden" name="match_kind" value={r.match_kind} />
                    <input type="hidden" name="body_regex" value={r.body_regex} />
                    <input type="hidden" name="fields" value={r.fields.join('\n')} />
                    <input
                      type="hidden"
                      name="next_enabled"
                      value={r.enabled ? 'false' : 'true'}
                    />
                    <ConfirmSubmitButton
                      confirm={
                        r.enabled
                          ? 'Retract this rule from all collectors?'
                          : 'Re-enable this rule?'
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
