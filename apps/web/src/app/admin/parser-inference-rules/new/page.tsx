/**
 * Admin · author a new inference rule (Task 6).
 *
 * Server component: reads the session, fetches the known event-type
 * keys server-side (so the trigger/followup/emit `<select>`s in the
 * client form are populated without a client-side round trip), and
 * renders the structured `<InferenceRuleForm>`.
 *
 * `publishInferenceRuleAction` is the `'use server'` action the form
 * posts to. It mirrors the #3 `publishRuleAction` posture in
 * `apps/web/src/app/admin/parser-submissions/[id]/page.tsx`: the
 * session is re-read inside the action (a stale form submission from
 * an expired session redirects to login rather than 401-ing the user
 * mid-edit), and `redirect()` — which throws `NEXT_REDIRECT` — always
 * runs outside the try/catch so it isn't swallowed as an error.
 */

import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminEventTypes,
  publishAdminInferenceRule,
  type PublishInferenceRuleRequest,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { InferenceRuleForm } from '@/components/admin/InferenceRuleForm';

interface PageProps {
  searchParams: Promise<{ error?: string }>;
}

export default async function NewInferenceRulePage(props: PageProps) {
  const session = await getSession();
  if (!session) {
    redirect('/auth/login?next=/admin/parser-inference-rules/new');
  }
  const { error: publishError } = await props.searchParams;

  let eventTypes: string[];
  try {
    const resp = await getAdminEventTypes(session.token);
    eventTypes = resp.event_types;
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/parser-inference-rules/new');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    throw e;
  }

  async function publishInferenceRuleAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) {
      redirect('/auth/login?next=/admin/parser-inference-rules/new');
    }
    let parsed: PublishInferenceRuleRequest;
    try {
      parsed = JSON.parse(String(formData.get('definition') ?? '{}'));
    } catch {
      redirect(
        '/admin/parser-inference-rules/new?error=invalid_definition' as Route,
      );
    }

    try {
      await publishAdminInferenceRule(s.token, parsed);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/admin/parser-inference-rules/new');
      }
      if (e instanceof ApiCallError && e.status === 403) {
        redirect('/me');
      }
      if (e instanceof ApiCallError) {
        redirect(
          `/admin/parser-inference-rules/new?error=${encodeURIComponent(e.body.error)}` as Route,
        );
      }
      throw e;
    }
    // Chip derived from the id that was actually submitted + accepted,
    // never a hardcoded string.
    redirect(
      `/admin/parser-inference-rules?published=${encodeURIComponent(parsed.id)}` as Route,
    );
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div className="ss-eyebrow">Admin · inference rules</div>
        <h1
          style={{
            margin: 0,
            fontSize: 24,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Author inference rule
        </h1>
      </header>

      {publishError && (
        <div
          role="status"
          className="ss-badge ss-badge--danger"
          style={{ alignSelf: 'flex-start' }}
        >
          Publish failed: {publishError}
        </div>
      )}

      <div className="ss-card" style={{ padding: 16, maxWidth: 720 }}>
        <InferenceRuleForm
          eventTypes={eventTypes}
          action={publishInferenceRuleAction}
        />
      </div>
    </div>
  );
}
