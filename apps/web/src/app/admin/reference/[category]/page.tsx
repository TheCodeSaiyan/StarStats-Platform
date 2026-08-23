/**
 * Admin · Reference entries within one category.
 *
 * Paged + searchable list of rows from `reference_registry` for a
 * single category. Used to spot-check the wiki-sync output ("did
 * the Hercules land in the vehicle table?", "why does
 * comp_armor_helm_xyz still have no display_name?").
 *
 * Read-only. The class_name + display_name are the cron's primary
 * outputs; the metadata blob is rendered as a collapsed JSON cell so
 * the page stays scannable even when entries carry rich data.
 */

import Link from 'next/link';
import { AdminPageHeader } from '../../_components/AdminPageHeader';
import type { Route } from 'next';
import { notFound, redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminReferenceEntries,
  type AdminReferenceEntriesResponse,
  type AdminReferenceEntryDto,
} from '@/lib/api';
import { getSession } from '@/lib/session';

interface PageProps {
  params: Promise<{ category: string }>;
  searchParams: Promise<{ q?: string; offset?: string }>;
}

const VALID_CATEGORIES = new Set(['vehicle', 'weapon', 'item', 'location']);

const CATEGORY_LABEL: Record<string, string> = {
  vehicle: 'Vehicles',
  weapon: 'Weapons',
  item: 'Items',
  location: 'Locations',
};

const PAGE_SIZE = 100;

export default async function AdminReferenceCategoryPage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/reference');

  const { category } = await props.params;
  if (!VALID_CATEGORIES.has(category)) notFound();

  const params = await props.searchParams;
  const q = params.q?.trim() ?? '';
  const offset = parsePositiveInt(params.offset);

  let data: AdminReferenceEntriesResponse;
  try {
    data = await getAdminReferenceEntries(session.token, category, {
      q: q || undefined,
      limit: PAGE_SIZE,
      offset,
    });
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/reference');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    if (e instanceof ApiCallError && e.status === 400) {
      notFound();
    }
    throw e;
  }

  const label = CATEGORY_LABEL[category] ?? category;

  const buildHref = (newOffset: number): Route => {
    const qs = new URLSearchParams();
    if (q) qs.set('q', q);
    if (newOffset > 0) qs.set('offset', String(newOffset));
    const s = qs.toString();
    const base = `/admin/reference/${encodeURIComponent(category)}`;
    return (s ? `${base}?${s}` : base) as Route;
  };

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <Link
        href={'/admin/reference' as Route}
        style={{ fontSize: 13, color: 'var(--accent)', textDecoration: 'none' }}
      >
        ← All categories
      </Link>

      <AdminPageHeader
        eyebrow={<>Admin · reference · {category}</>}
        title={label}
        lede={<>{data.total.toLocaleString()} total entries</>}
      />

      <form
        method="GET"
        action={`/admin/reference/${encodeURIComponent(category)}`}
        style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}
      >
        <input
          type="search"
          name="q"
          defaultValue={q}
          placeholder="Search class_name or display_name…"
          autoComplete="off"
          spellCheck={false}
          className="mono"
          style={{
            flex: '1 1 260px',
            padding: '8px 12px',
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            borderRadius: 0,
            color: 'var(--fg)',
          }}
        />
        <button type="submit" className="ss-btn ss-btn--primary">
          Search
        </button>
        {q && (
          <Link
            href={(`/admin/reference/${encodeURIComponent(category)}`) as Route}
            className="ss-btn ss-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            Clear
          </Link>
        )}
      </form>

      <section className="ss-card" style={{ padding: 0, overflow: 'hidden' }}>
        {data.entries.length === 0 ? (
          <p
            style={{
              margin: 0,
              padding: '40px 24px',
              textAlign: 'center',
              color: 'var(--fg-muted)',
              fontSize: 14,
            }}
          >
            No entries match this search.
          </p>
        ) : (
          <table
            style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}
          >
            <thead>
              <tr style={{ background: 'var(--bg-elev)' }}>
                <Th>Class name</Th>
                <Th>Display name</Th>
                <Th>Metadata</Th>
              </tr>
            </thead>
            <tbody>
              {data.entries.map((e) => (
                <EntryRow key={e.class_name} entry={e} />
              ))}
            </tbody>
          </table>
        )}
      </section>

      <nav
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
          {data.entries.length === 0
            ? 'Nothing on this page'
            : `Showing ${offset + 1}–${offset + data.entries.length} of ${data.total.toLocaleString()}`}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          {offset > 0 ? (
            <Link
              href={buildHref(Math.max(0, offset - PAGE_SIZE))}
              className="ss-btn ss-btn--ghost"
            >
              ← Prev
            </Link>
          ) : (
            <span
              className="ss-btn ss-btn--ghost"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              ← Prev
            </span>
          )}
          {offset + data.entries.length < data.total ? (
            <Link
              href={buildHref(offset + PAGE_SIZE)}
              className="ss-btn ss-btn--ghost"
            >
              Next →
            </Link>
          ) : (
            <span
              className="ss-btn ss-btn--ghost"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              Next →
            </span>
          )}
        </div>
      </nav>
    </div>
  );
}

function parsePositiveInt(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 0) return 0;
  return n;
}

function Th({ children }: { children: React.ReactNode }) {
  return (
    <th
      style={{
        textAlign: 'left',
        padding: '10px 14px',
        fontWeight: 600,
        color: 'var(--fg-muted)',
        fontSize: 11,
        letterSpacing: '0.06em',
        textTransform: 'uppercase',
        borderBottom: '1px solid var(--border)',
      }}
    >
      {children}
    </th>
  );
}

function EntryRow({ entry }: { entry: AdminReferenceEntryDto }) {
  const hasMetadata =
    entry.metadata &&
    typeof entry.metadata === 'object' &&
    !Array.isArray(entry.metadata) &&
    Object.keys(entry.metadata as Record<string, unknown>).length > 0;

  return (
    <tr style={{ borderBottom: '1px solid var(--border)' }}>
      <td style={{ padding: '10px 14px', verticalAlign: 'top' }}>
        <span className="mono" style={{ fontSize: 12 }}>
          {entry.class_name}
        </span>
      </td>
      <td style={{ padding: '10px 14px', verticalAlign: 'top' }}>
        {entry.display_name || (
          <span style={{ color: 'var(--fg-dim)' }}>—</span>
        )}
      </td>
      <td style={{ padding: '10px 14px', verticalAlign: 'top' }}>
        {hasMetadata ? (
          <details>
            <summary
              style={{
                cursor: 'pointer',
                fontSize: 12,
                color: 'var(--fg-muted)',
              }}
            >
              {Object.keys(entry.metadata as Record<string, unknown>).length}{' '}
              field
              {Object.keys(entry.metadata as Record<string, unknown>).length ===
              1
                ? ''
                : 's'}
            </summary>
            <pre
              className="mono"
              style={{
                marginTop: 6,
                padding: '8px 10px',
                background: 'var(--bg-elev)',
                border: '1px solid var(--border)',
                borderRadius: 0,
                fontSize: 11,
                color: 'var(--fg-muted)',
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
                maxWidth: 480,
              }}
            >
              {JSON.stringify(entry.metadata, null, 2)}
            </pre>
          </details>
        ) : (
          <span style={{ color: 'var(--fg-dim)', fontSize: 12 }}>—</span>
        )}
      </td>
    </tr>
  );
}
