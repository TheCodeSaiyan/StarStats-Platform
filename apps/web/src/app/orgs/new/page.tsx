import Link from 'next/link';
import { redirect } from 'next/navigation';
import { ApiCallError, createOrg } from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { AppSurface } from '@/components/projection/AppSurface';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import type { Calibration } from 'holo';

export const metadata = { title: "New org" };

interface SearchParams {
  error?: string;
}

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  margin: 0,
  padding: 0,
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
};

const headerTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 32,
  fontWeight: 600,
  letterSpacing: '-0.02em',
};

const headerSubtitleStyle: React.CSSProperties = {
  margin: '6px 0 0',
  color: 'var(--fg-muted)',
  fontSize: 14,
};

const cardHeaderStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 4,
  marginBottom: 16,
};

const cardTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 18,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};

const formStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  margin: 0,
  maxWidth: 480,
};

const formActionsStyle: React.CSSProperties = {
  display: 'flex',
  gap: 12,
  marginTop: 4,
};

export default async function NewOrgPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }
  if (!session) redirect('/auth/login?next=/orgs/new');

  const { error } = await props.searchParams;

  async function createAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/orgs/new');
    const name = String(formData.get('name') ?? '').trim();
    if (name === '') {
      redirect('/orgs/new?error=invalid_name');
    }
    let slug: string;
    try {
      const resp = await createOrg(session.token, { name });
      slug = resp.org.slug;
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/orgs/new');
        if (e.status === 409) redirect('/orgs/new?error=slug_collision');
        if (e.status === 400) {
          redirect(
            `/orgs/new?error=${encodeURIComponent(e.body.error)}`,
          );
        }
        if (e.status === 503) {
          redirect('/orgs/new?error=spicedb_unavailable');
        }
      }
      logger.error({ err: e }, 'create org failed');
      redirect('/orgs/new?error=unexpected');
    }
    // Outside the catch: Next implements `redirect()` by throwing a
    // NEXT_REDIRECT sentinel. Throwing it inside a try/catch caused
    // the success path to be swallowed and rerouted to the error
    // page. Same pattern as login/signup actions.
    redirect(`/orgs/${encodeURIComponent(slug)}`);
  }

  return (
    <AppSurface
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections({
        signedIn: Boolean(session),
        staffRoles: session?.staffRoles,
      })}
      crumb={[
        { label: 'Projection', href: '/me' },
        { label: 'Orgs', href: '/orgs' },
        { label: 'New org' },
      ]}
      sections={[
        {
          id: 'page',
          group: 'page',
          title: 'New org',
          ctx: 'Register an RSI org',
          node: (
    <div className="ss-screen-enter" style={mainStyle}>
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Orgs · new
        </div>
        <h1 style={headerTitleStyle}>New org</h1>
        <p style={headerSubtitleStyle}>
          Pick a display name. The URL slug is generated automatically and
          can&apos;t be changed later.
        </p>
      </header>

      {error && (
        <div className="ss-alert ss-alert--danger" role="alert">
          {labelForError(error)}
        </div>
      )}

      <section className="ss-card ss-card-pad">
        <div style={cardHeaderStyle}>
          <span className="ss-eyebrow">Identity</span>
          <h2 style={cardTitleStyle}>Name your org</h2>
        </div>
        <form action={createAction} style={formStyle}>
          <label className="ss-label">
            <span className="ss-label-text">Name</span>
            <input
              className="ss-input"
              type="text"
              name="name"
              required
              autoComplete="off"
              spellCheck={false}
              placeholder="Test Squadron - Best Squardon"
              maxLength={120}
            />
            <small style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
              2–120 characters. Slug = lowercase, dashes only.
            </small>
          </label>
          <div style={formActionsStyle}>
            <button type="submit" className="ss-btn ss-btn--primary">
              Create org
            </button>
            <Link href="/orgs" className="ss-btn ss-btn--ghost">
              Cancel
            </Link>
          </div>
        </form>
      </section>
    </div>
          ),
        },
      ]}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}

function labelForError(code: string): string {
  switch (code) {
    case 'invalid_name':
      return 'That name is empty or has no usable characters.';
    case 'slug_collision':
      return "We couldn't generate a unique URL for that name. Try a different name.";
    case 'spicedb_unavailable':
      return 'Organizations are temporarily unavailable. Try again shortly.';
    case 'unexpected':
      return 'Something went wrong. Please try again.';
    default:
      return `Couldn't complete that action (${code}).`;
  }
}
