/**
 * /admin/news — write and publish announcements to every player.
 *
 * A published post appears in the tray's What's New (with an unread badge
 * and a desktop notification) and at the top of /changelog. Posts are
 * plain text: what you type is what readers see, with line breaks kept and
 * no formatting, so nothing here can inject markup into either client.
 */

import React from 'react';
import { redirect } from 'next/navigation';
import { adminListNews, statusOf, type NewsPost } from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import {
  createNewsAction,
  deleteNewsAction,
  setPublishedAction,
  updateNewsAction,
} from './actions';

export const metadata = { title: 'News' };

const STATUS: Record<string, string> = {
  published: 'Published. Players see it in What’s New and on /changelog.',
  drafted: 'Saved as a draft. Nobody sees it until you publish.',
  saved: 'Changes saved.',
  unpublished: 'Unpublished. It is a draft again.',
  deleted: 'Deleted.',
};

const ERRORS: Record<string, string> = {
  title_required: 'A title is required.',
  title_too_long: 'The title is too long (120 characters at most).',
  body_required: 'The post needs some text.',
  body_too_long: 'The post is too long (4,000 characters at most).',
  invalid_link: 'The link must start with https:// and contain no spaces.',
  news_not_found: 'That post no longer exists.',
  unexpected: 'Something went wrong. Try again.',
};

const fieldStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 4,
};

function PostFields({ post }: { post?: NewsPost }) {
  const id = post?.id ?? 'new';
  return (
    <>
      <label style={fieldStyle} htmlFor={`title-${id}`}>
        Title
        <input
          id={`title-${id}`}
          name="title"
          required
          maxLength={120}
          defaultValue={post?.title ?? ''}
        />
      </label>
      <label style={fieldStyle} htmlFor={`body-${id}`}>
        Post
        <textarea
          id={`body-${id}`}
          name="body"
          required
          maxLength={4000}
          rows={6}
          defaultValue={post?.body ?? ''}
        />
      </label>
      <label style={fieldStyle} htmlFor={`link-${id}`}>
        Link (optional, https only)
        <input
          id={`link-${id}`}
          name="link_url"
          type="url"
          pattern="https://.*"
          defaultValue={post?.link_url ?? ''}
        />
      </label>
    </>
  );
}

function when(iso: string | null | undefined): string {
  if (!iso) return '';
  return new Date(iso).toLocaleString('en-GB', {
    dateStyle: 'medium',
    timeStyle: 'short',
    timeZone: 'UTC',
  });
}

export default async function AdminNewsPage(props: {
  searchParams: Promise<{ status?: string; error?: string }>;
}) {
  const session = await getSession();
  // The admin layout gates on staff roles; this narrows the type.
  if (!session) redirect('/auth/login?next=/admin/news');
  const params = await props.searchParams;

  let posts: NewsPost[] | null = null;
  try {
    posts = await adminListNews(session.token);
  } catch (err) {
    logger.warn({ err, status: statusOf(err), call: 'admin.news.list' }, 'admin news fetch failed');
  }

  const notice = params.status ? STATUS[params.status] : undefined;
  const error = params.error ? (ERRORS[params.error] ?? ERRORS.unexpected) : undefined;

  return (
    <div>
      <h1
        style={{
          margin: '0 0 var(--s2)',
          fontSize: 'clamp(28px, 4vw, 40px)',
          fontWeight: 600,
          letterSpacing: 'var(--tracking-tight)',
        }}
      >
        News
      </h1>
      <p style={{ color: 'var(--fg-muted)', marginTop: 0 }}>
        Announcements for every player. A published post shows in the
        tray&rsquo;s What&rsquo;s New, with a desktop notification, and at the top
        of /changelog. Plain text only: line breaks are kept, nothing else is
        formatted.
      </p>

      {notice ? (
        <p role="status" className="ss-card" style={{ padding: 'var(--s4)' }}>
          {notice}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="ss-card" style={{ padding: 'var(--s4)' }}>
          {error}
        </p>
      ) : null}

      <section className="ss-card" style={{ padding: 'var(--s5)', marginBottom: 'var(--s5)' }}>
        <h2 style={{ marginTop: 0 }}>New post</h2>
        <form action={createNewsAction} style={{ display: 'grid', gap: 12, maxWidth: 640 }}>
          <PostFields />
          <label style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <input type="checkbox" name="publish" />
            Publish now (otherwise saved as a draft)
          </label>
          <ConfirmSubmitButton className="ss-btn ss-btn--primary" style={{ justifySelf: 'start' }}>
            Save
          </ConfirmSubmitButton>
        </form>
      </section>

      {posts === null ? (
        // Never show an empty list on a failed read: "no posts" would be a
        // statement about the data, and it is not one we can make.
        <p role="alert" className="ss-card" style={{ padding: 'var(--s5)' }}>
          Could not load posts. Check the API logs, then refresh.
        </p>
      ) : posts.length === 0 ? (
        <p style={{ color: 'var(--fg-muted)' }}>No posts yet.</p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0, margin: 0, display: 'grid', gap: 12 }}>
          {posts.map((p) => {
            const live = p.published_at != null;
            return (
              <li
                key={p.id}
                className="ss-card"
                style={{ padding: 'var(--s4)' }}
                data-testid="news-admin-row"
              >
                <div style={{ display: 'flex', gap: 12, alignItems: 'baseline', flexWrap: 'wrap' }}>
                  <strong>{p.title}</strong>
                  <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                    {live ? `Published ${when(p.published_at)} UTC` : 'Draft'} · by {p.created_by}
                  </span>
                </div>
                <p style={{ whiteSpace: 'pre-wrap', margin: '8px 0' }}>{p.body}</p>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <form action={setPublishedAction}>
                    <input type="hidden" name="id" value={p.id} />
                    <input type="hidden" name="published" value={live ? 'false' : 'true'} />
                    <ConfirmSubmitButton
                      className={live ? 'ss-btn ss-btn--ghost' : 'ss-btn ss-btn--primary'}
                      confirm={
                        live
                          ? `Unpublish "${p.title}"? Players stop seeing it.`
                          : `Publish "${p.title}" to every player now?`
                      }
                    >
                      {live ? 'Unpublish' : 'Publish'}
                    </ConfirmSubmitButton>
                  </form>
                  <form action={deleteNewsAction}>
                    <input type="hidden" name="id" value={p.id} />
                    <ConfirmSubmitButton confirm={`Delete "${p.title}"? This cannot be undone here.`}>
                      Delete
                    </ConfirmSubmitButton>
                  </form>
                </div>
                <details style={{ marginTop: 8 }}>
                  <summary>Edit</summary>
                  <form
                    action={updateNewsAction}
                    style={{ display: 'grid', gap: 12, maxWidth: 640, marginTop: 8 }}
                  >
                    <input type="hidden" name="id" value={p.id} />
                    <PostFields post={p} />
                    <ConfirmSubmitButton className="ss-btn" style={{ justifySelf: 'start' }}>
                      Save changes
                    </ConfirmSubmitButton>
                  </form>
                </details>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
