/**
 * Admin · Parser shapes detail + moderation form (W6).
 *
 * Surfaces the full ParserSubmission payload for one shape so a
 * rule author can inspect raw examples, partial structured fields,
 * context lines, tray-supplied hints, and stamp their decision via
 * the PATCH form on the side panel.
 *
 * Server-action posture matches admin/sharing/reports: the form
 * action reads the session cookie server-side (bearer never crosses
 * the boundary), calls patchAdminParserSubmission, then
 * revalidates the page + list path so the next render reflects the
 * new state. 401 -> login; 403 -> dashboard; everything else
 * re-throws so the page's error boundary catches it.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { notFound, redirect } from 'next/navigation';
import { revalidatePath } from 'next/cache';
import {
  ApiCallError,
  getAdminParserSubmission,
  patchAdminParserSubmission,
  publishAdminParserRule,
  publishSubmissionToCommunity,
  type AdminParserSubmissionDetail,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { parseFieldsInput } from './fields';

const STATUS_OPTIONS = [
  { value: 'pending', label: 'Pending' },
  { value: 'drafting', label: 'Drafting' },
  { value: 'rule_written', label: 'Rule written' },
  { value: 'dismissed', label: 'Dismissed' },
] as const;

interface PageProps {
  params: Promise<{ id: string }>;
  searchParams: Promise<{
    published?: string;
    error?: string;
    community?: string;
  }>;
}

export default async function AdminParserSubmissionDetailPage(props: PageProps) {
  const session = await getSession();
  const { id: idRaw } = await props.params;
  const {
    published,
    error: publishError,
    community,
  } = await props.searchParams;
  if (!session) {
    redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
  }
  const id = Number.parseInt(idRaw, 10);
  if (!Number.isFinite(id) || id <= 0) {
    notFound();
  }

  let detail: AdminParserSubmissionDetail;
  try {
    detail = await getAdminParserSubmission(session.token, id);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 404) notFound();
    if (e instanceof ApiCallError && e.status === 401) {
      redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    throw e;
  }

  // Server action — closes over `id` (already validated above). The
  // session cookie is re-read inside the action so a stale form
  // submission from an expired session redirects to login rather
  // than 401-ing the user mid-edit.
  async function saveAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) {
      redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
    }
    const status = String(formData.get('status') ?? '').trim();
    const reviewerNotesRaw = formData.get('reviewer_notes');
    const ruleIdRaw = formData.get('rule_id');

    // Build a minimal patch: omit fields the form left unchanged so
    // the server's "leave column alone" semantics kick in.
    const body: {
      status?: string;
      reviewer_notes?: string;
      rule_id?: string;
    } = {};
    if (status && status !== detail.status) {
      body.status = status;
    }
    if (typeof reviewerNotesRaw === 'string') {
      // The textarea always submits something — even empty string —
      // so we always send the field. The server treats Some("") as
      // "clear notes" which is exactly the UI affordance.
      body.reviewer_notes = reviewerNotesRaw;
    }
    if (typeof ruleIdRaw === 'string') {
      body.rule_id = ruleIdRaw;
    }

    try {
      await patchAdminParserSubmission(s.token, id, body);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
      }
      if (e instanceof ApiCallError && e.status === 403) {
        redirect('/me');
      }
      throw e;
    }
    revalidatePath(`/admin/parser-submissions/${idRaw}`);
    revalidatePath('/admin/parser-submissions');
  }

  // Second server action, beside `saveAction`. Publishes a rule into
  // the served manifest, then auto-links + advances this submission
  // — but only once the publish itself has succeeded, so a failed
  // POST never leaves the shape marked done-but-ruleless. `redirect`
  // throws (NEXT_REDIRECT), so it must run outside the try/catch or
  // it would be swallowed as an error.
  async function publishRuleAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) {
      redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
    }
    const body = {
      rule_id: String(formData.get('rule_id') ?? '').trim(),
      event_name: String(formData.get('event_name') ?? '').trim(),
      match_kind: String(formData.get('match_kind') ?? 'event_name'),
      body_regex: String(formData.get('body_regex') ?? ''),
      fields: parseFieldsInput(String(formData.get('fields') ?? '')),
      enabled: true,
    };

    let response: Awaited<ReturnType<typeof publishAdminParserRule>>;
    try {
      response = await publishAdminParserRule(s.token, body);
      // Auto-link + advance — only after a successful publish, so a
      // failed POST never leaves the shape marked done-but-ruleless.
      await patchAdminParserSubmission(s.token, id, {
        rule_id: response.rule_id,
        status: 'rule_written',
      });
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
      }
      if (e instanceof ApiCallError && e.status === 403) {
        redirect('/me');
      }
      if (e instanceof ApiCallError) {
        redirect(
          `/admin/parser-submissions/${idRaw}?error=${encodeURIComponent(e.body.error)}`,
        );
      }
      throw e;
    }
    revalidatePath(`/admin/parser-submissions/${idRaw}`);
    revalidatePath('/admin/parser-submissions');
    // Chip derived from the API response, never the submitted form value.
    redirect(
      `/admin/parser-submissions/${idRaw}?published=${encodeURIComponent(response.rule_id)}`,
    );
  }

  // Third server action, beside `publishRuleAction`. Promotes this
  // shape into the public community queue via the idempotent publish
  // endpoint. Same posture: re-read the session inside the action,
  // build the body from formData, and `redirect` OUTSIDE the
  // try/catch so NEXT_REDIRECT isn't swallowed. The success chip is
  // derived from the API response (`community_submission_id`), never
  // the submitted form.
  async function publishCommunityAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) {
      redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
    }
    const body = {
      proposed_label: String(formData.get('proposed_label') ?? '').trim(),
      description: String(formData.get('description') ?? '').trim(),
      pattern: String(formData.get('pattern') ?? '').trim(),
      // A moderator override: when checked, publish anonymously under
      // the `community` system account even if the shape is attributed.
      force_anonymous: formData.get('force_anonymous') === 'on',
    };

    let response: Awaited<ReturnType<typeof publishSubmissionToCommunity>>;
    try {
      response = await publishSubmissionToCommunity(s.token, id, body);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect(`/auth/login?next=/admin/parser-submissions/${idRaw}`);
      }
      if (e instanceof ApiCallError && e.status === 403) {
        redirect('/me');
      }
      if (e instanceof ApiCallError) {
        redirect(
          `/admin/parser-submissions/${idRaw}?error=${encodeURIComponent(e.body.error)}`,
        );
      }
      throw e;
    }
    revalidatePath(`/admin/parser-submissions/${idRaw}`);
    revalidatePath('/admin/parser-submissions');
    // Chip derived from the API response, never the submitted form value.
    redirect(
      `/admin/parser-submissions/${idRaw}?community=${encodeURIComponent(response.community_submission_id)}`,
    );
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <header style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div className="ss-eyebrow">Admin · parser shapes</div>
        <h1
          style={{
            margin: 0,
            fontSize: 24,
            fontWeight: 600,
            letterSpacing: '-0.02em',
            fontFamily: 'var(--font-mono, ui-monospace, monospace)',
            wordBreak: 'break-all',
          }}
        >
          {detail.shape_hash}
        </h1>
        <div
          style={{
            display: 'flex',
            gap: 14,
            color: 'var(--fg-muted)',
            fontSize: 13,
            flexWrap: 'wrap',
          }}
        >
          <span>
            <strong style={{ color: 'var(--fg)' }}>{detail.submitter_count}</strong> install{detail.submitter_count === 1 ? '' : 's'}
          </span>
          <span>
            <strong style={{ color: 'var(--fg)' }}>{detail.total_occurrence_count}</strong> occurrences
          </span>
          <span>Current status: {detail.status}</span>
          <Link
            href={'/admin/parser-submissions?status=pending' as Route}
            prefetch={false}
            style={{ color: 'var(--accent, #6aa9ff)' }}
          >
            ← back to queue
          </Link>
        </div>
      </header>

      {published && (
        <div
          role="status"
          className="ss-badge ss-badge--ok"
          style={{ alignSelf: 'flex-start' }}
        >
          Published rule {published} — submission linked + marked rule
          written.
        </div>
      )}
      {publishError && (
        <div role="status" className="ss-badge ss-badge--danger" style={{ alignSelf: 'flex-start' }}>
          Publish failed: {publishError}
        </div>
      )}
      {community && (
        <div
          role="status"
          className="ss-badge ss-badge--ok"
          style={{ alignSelf: 'flex-start' }}
        >
          Published to community —{' '}
          <Link href={`/submissions/${community}` as Route}>view →</Link>
        </div>
      )}

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'minmax(0, 2fr) minmax(280px, 1fr)',
          gap: 20,
          alignItems: 'start',
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
          <PayloadSection title="Raw examples">
            {detail.payload.raw_examples.length === 0 ? (
              <Empty>No raw examples on this row.</Empty>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {detail.payload.raw_examples.map((line, i) => (
                  <pre
                    key={i}
                    data-testid="raw-example"
                    style={{
                      margin: 0,
                      padding: '10px 14px',
                      background: 'var(--bg-elev)',
                      border: '1px solid var(--border)',
                      borderRadius: 0,
                      fontFamily:
                        'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                      fontSize: 12,
                      whiteSpace: 'pre-wrap',
                      wordBreak: 'break-all',
                    }}
                  >
                    {line}
                  </pre>
                ))}
              </div>
            )}
          </PayloadSection>

          <PayloadSection title="Metadata">
            <KvTable
              rows={[
                ['shell_tag', detail.payload.shell_tag],
                ['suggested_event_name', detail.payload.suggested_event_name],
                ['channel', detail.payload.channel],
                ['game_build', detail.payload.game_build],
                ['notes (submitter)', detail.payload.notes],
              ]}
            />
          </PayloadSection>

          {detail.payload.partial_structured &&
            Object.keys(detail.payload.partial_structured).length > 0 && (
              <PayloadSection title="Partial structured fields">
                <KvTable
                  rows={Object.entries(detail.payload.partial_structured).map(
                    ([k, v]) => [k, v],
                  )}
                />
              </PayloadSection>
            )}

          {detail.payload.suggested_field_names &&
            Object.keys(detail.payload.suggested_field_names).length > 0 && (
              <PayloadSection title="Suggested field names">
                <KvTable
                  rows={Object.entries(detail.payload.suggested_field_names).map(
                    ([k, v]) => [k, v],
                  )}
                />
              </PayloadSection>
            )}

          {detail.payload.context_examples &&
            detail.payload.context_examples.length > 0 && (
              <PayloadSection title="Context lines">
                <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  {detail.payload.context_examples.map((ctx, i) => (
                    <div
                      key={i}
                      style={{
                        border: '1px solid var(--border)',
                        borderRadius: 0,
                        overflow: 'hidden',
                      }}
                    >
                      <ContextBlock label="Before" lines={ctx.before} />
                      <ContextBlock
                        label="After"
                        lines={ctx.after}
                        borderTop
                      />
                    </div>
                  ))}
                </div>
              </PayloadSection>
            )}
        </div>

        <aside
          className="ss-card"
          style={{
            padding: 16,
            display: 'flex',
            flexDirection: 'column',
            gap: 12,
            position: 'sticky',
            top: 12,
          }}
        >
          <h2 style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>
            Moderation
          </h2>
          <form
            action={saveAction}
            data-testid="parser-submission-save-form"
            style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
          >
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                Status
              </span>
              <select
                name="status"
                defaultValue={detail.status}
                data-testid="parser-submission-status-select"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                }}
              >
                {STATUS_OPTIONS.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
            </label>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                Reviewer notes
              </span>
              <textarea
                name="reviewer_notes"
                defaultValue={detail.reviewer_notes ?? ''}
                rows={5}
                data-testid="parser-submission-notes-input"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                  fontFamily: 'inherit',
                  resize: 'vertical',
                }}
              />
            </label>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                Manifest rule id
              </span>
              <input
                name="rule_id"
                type="text"
                defaultValue={detail.rule_id ?? ''}
                placeholder="e.g. combat.kill_v3"
                data-testid="parser-submission-rule-id-input"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                  fontFamily:
                    'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                }}
              />
            </label>
            <button
              type="submit"
              data-testid="parser-submission-save"
              className="ss-btn"
              style={{
                padding: '8px 16px',
                borderRadius: 0,
                border: '1px solid var(--border-strong)',
                background: 'var(--bg-elev)',
                color: 'var(--fg)',
                fontSize: 13,
                cursor: 'pointer',
              }}
            >
              Save
            </button>
          </form>

          <div
            style={{
              borderTop: '1px solid var(--border)',
              margin: '4px 0',
            }}
          />

          <h2 style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>
            Publish rule
          </h2>
          <form
            action={publishRuleAction}
            data-testid="parser-submission-publish-form"
            style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
          >
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                rule_id
              </span>
              <input
                name="rule_id"
                type="text"
                required
                pattern="[A-Za-z0-9_.\-]+"
                defaultValue={detail.rule_id ?? ''}
                placeholder="e.g. combat.kill_v3"
                data-testid="parser-submission-publish-rule-id-input"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                  fontFamily:
                    'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                }}
              />
            </label>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                event_name
              </span>
              <input
                name="event_name"
                type="text"
                required
                data-testid="parser-submission-publish-event-name-input"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                }}
              />
            </label>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                match_kind
              </span>
              <select
                name="match_kind"
                defaultValue="event_name"
                data-testid="parser-submission-publish-match-kind-select"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                }}
              >
                <option value="event_name">event_name</option>
                <option value="body_keyword">body_keyword</option>
              </select>
            </label>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                body_regex
              </span>
              <input
                name="body_regex"
                type="text"
                placeholder="(?P<who>\w+)"
                data-testid="parser-submission-publish-body-regex-input"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                  fontFamily:
                    'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                }}
              />
            </label>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                fields (comma / newline separated)
              </span>
              <textarea
                name="fields"
                rows={2}
                data-testid="parser-submission-publish-fields-input"
                style={{
                  padding: '8px 10px',
                  borderRadius: 0,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-elev)',
                  color: 'var(--fg)',
                  fontSize: 13,
                  fontFamily: 'inherit',
                  resize: 'vertical',
                }}
              />
            </label>
            <ConfirmSubmitButton
              confirm="Publish this rule to all collectors?"
              pendingLabel="Publishing…"
              data-testid="parser-submission-publish-submit"
              className="ss-btn ss-btn--primary"
              style={{
                padding: '8px 16px',
                borderRadius: 0,
                fontSize: 13,
              }}
            >
              Publish rule
            </ConfirmSubmitButton>
          </form>

          <div
            style={{
              borderTop: '1px solid var(--border)',
              margin: '4px 0',
            }}
          />

          <h2 style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>
            Publish to community
          </h2>
          <p
            style={{ margin: 0, fontSize: 12, color: 'var(--fg-muted)' }}
            data-testid="parser-submission-submitter-attribution"
          >
            {detail.submitter_handle ? (
              <>
                Submitter:{' '}
                <strong
                  style={{
                    color: 'var(--fg)',
                    fontFamily:
                      'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                  }}
                >
                  @{detail.submitter_handle}
                </strong>{' '}
                (attributed)
              </>
            ) : (
              'Submitter: anonymous'
            )}
          </p>
          {detail.community_submission_id ? (
            <div
              role="status"
              className="ss-badge ss-badge--ok"
              style={{ alignSelf: 'flex-start' }}
            >
              Published to community —{' '}
              <Link
                href={`/submissions/${detail.community_submission_id}` as Route}
                data-testid="parser-submission-community-view-link"
              >
                view →
              </Link>
            </div>
          ) : (
            <form
              action={publishCommunityAction}
              data-testid="parser-submission-community-form"
              style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
            >
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                  proposed_label
                </span>
                <input
                  name="proposed_label"
                  type="text"
                  required
                  defaultValue={detail.payload.suggested_event_name ?? ''}
                  placeholder="e.g. combat.kill"
                  data-testid="parser-submission-community-label-input"
                  style={{
                    padding: '8px 10px',
                    borderRadius: 0,
                    border: '1px solid var(--border)',
                    background: 'var(--bg-elev)',
                    color: 'var(--fg)',
                    fontSize: 13,
                    fontFamily:
                      'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                  }}
                />
              </label>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                  pattern
                </span>
                <input
                  name="pattern"
                  type="text"
                  required
                  defaultValue={detail.payload.raw_examples[0] ?? ''}
                  placeholder="a representative raw line"
                  data-testid="parser-submission-community-pattern-input"
                  style={{
                    padding: '8px 10px',
                    borderRadius: 0,
                    border: '1px solid var(--border)',
                    background: 'var(--bg-elev)',
                    color: 'var(--fg)',
                    fontSize: 13,
                    fontFamily:
                      'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                  }}
                />
              </label>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                  description
                </span>
                <textarea
                  name="description"
                  required
                  rows={3}
                  placeholder="What this shape represents — shown on the public community entry."
                  data-testid="parser-submission-community-description-input"
                  style={{
                    padding: '8px 10px',
                    borderRadius: 0,
                    border: '1px solid var(--border)',
                    background: 'var(--bg-elev)',
                    color: 'var(--fg)',
                    fontSize: 13,
                    fontFamily: 'inherit',
                    resize: 'vertical',
                  }}
                />
              </label>
              <label
                style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}
              >
                <input
                  name="force_anonymous"
                  type="checkbox"
                  data-testid="parser-submission-community-force-anonymous-input"
                  style={{ marginTop: 3 }}
                />
                <span
                  style={{ display: 'flex', flexDirection: 'column', gap: 2 }}
                >
                  <span style={{ fontSize: 13, color: 'var(--fg)' }}>
                    Publish anonymously — overrides the submitter&apos;s
                    attribution
                  </span>
                  <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                    Attributed submissions show the submitter&apos;s handle;
                    anonymous ones show @community.
                  </span>
                </span>
              </label>
              <ConfirmSubmitButton
                confirm="Publish this shape to the public community queue?"
                pendingLabel="Publishing…"
                data-testid="parser-submission-community-submit"
                className="ss-btn ss-btn--primary"
                style={{
                  padding: '8px 16px',
                  borderRadius: 0,
                  fontSize: 13,
                }}
              >
                Publish to community
              </ConfirmSubmitButton>
            </form>
          )}
        </aside>
      </div>
    </div>
  );
}

function PayloadSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section
      className="ss-card"
      style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 10 }}
    >
      <h2 style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>{title}</h2>
      {children}
    </section>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
      {children}
    </p>
  );
}

function KvTable({ rows }: { rows: Array<[string, string | null | undefined]> }) {
  return (
    <table
      style={{
        width: '100%',
        borderCollapse: 'collapse',
        fontSize: 13,
      }}
    >
      <tbody>
        {rows.map(([k, v]) => (
          <tr key={k} style={{ borderTop: '1px solid var(--border)' }}>
            <td
              style={{
                padding: '6px 0',
                width: '40%',
                color: 'var(--fg-muted)',
                fontFamily:
                  'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
                fontSize: 12,
              }}
            >
              {k}
            </td>
            <td style={{ padding: '6px 0' }}>
              {v ?? <span style={{ color: 'var(--fg-muted)' }}>—</span>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ContextBlock({
  label,
  lines,
  borderTop,
}: {
  label: string;
  lines: string[];
  borderTop?: boolean;
}) {
  return (
    <div
      style={{
        padding: 10,
        borderTop: borderTop ? '1px solid var(--border)' : undefined,
        background: 'var(--bg-elev)',
      }}
    >
      <div
        style={{
          fontSize: 11,
          color: 'var(--fg-muted)',
          letterSpacing: '0.06em',
          textTransform: 'uppercase',
          marginBottom: 4,
        }}
      >
        {label}
      </div>
      {lines.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 12 }}>
          (no lines)
        </p>
      ) : (
        <pre
          style={{
            margin: 0,
            fontFamily:
              'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
            fontSize: 12,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-all',
          }}
        >
          {lines.join('\n')}
        </pre>
      )}
    </div>
  );
}
