'use client';

import React, { useState } from 'react';
import type { WaitlistConfigApi, WaitlistEntryApi } from '@/lib/api';
import {
  admitWaitlistAction,
  deleteWaitlistAction,
  resendWaitlistAction,
  saveWaitlistConfigAction,
} from '@/app/_actions/waitlist-admin';

/** Name at most this many blocked rows in a notice before falling back to
 * "+N more" — an admin skimming a delete notice needs to recognise a
 * couple of addresses, not read an unbounded list. */
const MAX_NAMED_BLOCKED = 3;

/** id -> email for every row currently rendered, so a `blocked` id coming
 * back from the server can be named in the notice without an extra fetch —
 * the console already has both tables in props. */
function buildEmailIndex(...groups: WaitlistEntryApi[][]): Map<string, string> {
  const map = new Map<string, string>();
  for (const group of groups) {
    for (const e of group) map.set(e.id, e.email);
  }
  return map;
}

/** Renders blocked ids as "a@x.com, b@x.com, +2 more" (or just the emails
 * when there are few enough to list in full). Falls back to the raw id for
 * a row this render never saw — e.g. it fell off the page's `limit`. */
function describeBlocked(ids: string[], emailById: Map<string, string>): string {
  const names = ids.map((id) => emailById.get(id) ?? id);
  if (names.length <= MAX_NAMED_BLOCKED) return names.join(', ');
  const shown = names.slice(0, MAX_NAMED_BLOCKED);
  return `${shown.join(', ')}, +${names.length - MAX_NAMED_BLOCKED} more`;
}

/**
 * Interactive half of /admin/waitlist: batch admission, invite resend for
 * already-admitted rows, plus the cap and gate switch. The page itself is
 * a server component; this owns the selection state and the actions.
 */
export function WaitlistConsole({
  queued,
  admitted,
  admittedCount,
  config,
}: {
  queued: WaitlistEntryApi[];
  admitted: WaitlistEntryApi[];
  admittedCount: number;
  config: WaitlistConfigApi;
}) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [resendSelected, setResendSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [cap, setCap] = useState(String(config.cap));
  const [gate, setGate] = useState(config.gate_enabled);

  const atCap = admittedCount >= config.cap;
  const emailById = buildEmailIndex(queued, admitted);

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleResend(id: string) {
    setResendSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function admit() {
    if (busy || selected.size === 0) return;
    setBusy(true);
    setNotice(null);
    try {
      const res = await admitWaitlistAction([...selected]);
      if (!res.ok) {
        setNotice('Admit failed — nobody was admitted and no mail went out.');
        return;
      }
      // Report what actually happened, not what was asked for: rows
      // already admitted are skipped, so the two can differ.
      const asked = selected.size;
      setNotice(
        res.admitted === asked
          ? `Admitted ${res.admitted}. Invites sent.`
          : `Admitted ${res.admitted} of ${asked} — the rest were already admitted.`,
      );
      setSelected(new Set());
    } finally {
      setBusy(false);
    }
  }

  async function resend() {
    if (busy || resendSelected.size === 0) return;
    setBusy(true);
    setNotice(null);
    try {
      const asked = resendSelected.size;
      const res = await resendWaitlistAction([...resendSelected]);
      if (!res.ok) {
        setNotice('Resend failed — no invites went out. Check the API logs.');
        return;
      }
      // `resent` counts SUCCESSFUL sends. Anything short of what was asked
      // means the transport is still failing — say so plainly rather than
      // let a partial send read as done.
      setNotice(
        res.resent === asked
          ? `Re-sent ${res.resent} invite${asked === 1 ? '' : 's'}.`
          : `Re-sent ${res.resent} of ${asked} — the rest failed to send. The mail transport may still be broken; check the API logs.`,
      );
      setResendSelected(new Set());
    } finally {
      setBusy(false);
    }
  }

  // Shared by both tables: the Queue's `selected` and the Admitted table's
  // `resendSelected` each drive their own Delete button, but the request,
  // confirm copy, and notice wording are identical either way. Safe to
  // point at either set — the server refuses (and reports in `blocked`)
  // any row whose invite was already redeemed, so this has no client-side
  // guard beyond "something is selected". The Admitted table also disables
  // the checkbox for a row this render already knows is redeemed, but
  // that's a UX hint, not a guard: the hint can be stale (the row could be
  // redeemed a moment after this page loaded), so `blocked` here is always
  // read from the response, never assumed from what got selected.
  async function remove(ids: Set<string>, clearSelection: () => void) {
    if (busy || ids.size === 0) return;
    if (
      !window.confirm(
        `Permanently delete ${ids.size} signup(s)? This cannot be undone.`,
      )
    )
      return;
    setBusy(true);
    setNotice(null);
    try {
      const asked = ids.size;
      const res = await deleteWaitlistAction([...ids]);
      if (!res.ok) {
        setNotice('Delete failed — nothing was removed.');
        return;
      }
      // Report what was actually removed, not what was asked for: rows
      // whose invite was already redeemed are refused by the server (and
      // land in `blocked`) — named, not just counted, so an admin isn't
      // left guessing which one survived. But `deleted.length +
      // blocked.length` can ALSO fall short of `asked` — a second
      // moderator or a stale tab can delete a row out from under this one
      // between selection and submit — and that must not read as a clean
      // sweep either.
      const deletedCount = res.deleted.length;
      const blockedCount = res.blocked.length;
      const accounted = deletedCount + blockedCount;
      const blockedNames = describeBlocked(res.blocked, emailById);
      setNotice(
        accounted < asked
          ? `Deleted ${deletedCount} of ${asked}` +
              (blockedCount > 0
                ? ` (${blockedCount} skipped — already redeemed: ${blockedNames})`
                : '') +
              ' — the rest were already removed.'
          : blockedCount > 0
            ? `Deleted ${deletedCount}. Skipped ${blockedCount} already redeemed: ${blockedNames}.`
            : `Deleted ${deletedCount}.`,
      );
      clearSelection();
    } finally {
      setBusy(false);
    }
  }

  async function saveConfig() {
    if (busy) return;
    setBusy(true);
    setNotice(null);
    try {
      const parsed = Number.parseInt(cap, 10);
      if (!Number.isFinite(parsed) || parsed < 0) {
        setNotice('Cap must be a number of 0 or more.');
        return;
      }
      const res = await saveWaitlistConfigAction({
        cap: parsed,
        gate_enabled: gate,
      });
      if (!res.ok) {
        setNotice('Save failed — the gate and cap are unchanged.');
        return;
      }
      // Reflect what the server stored; it clamps.
      setCap(String(res.config.cap));
      setGate(res.config.gate_enabled);
      setNotice(
        res.config.gate_enabled
          ? `Saved. Gate is ON — signup requires an invite. Cap ${res.config.cap}.`
          : `Saved. Gate is OFF — signup is open to anyone. Cap ${res.config.cap} (unused while the gate is off).`,
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <section
        className="ss-card"
        style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s5)' }}
      >
        <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
          Gate
        </div>
        <p style={{ marginTop: 0, color: 'var(--fg-muted)' }}>
          {gate ? (
            <>
              Signup requires an invite. <strong>{admittedCount}</strong> of{' '}
              <strong>{config.cap}</strong> admitted.{' '}
              {atCap ? (
                // A queue that has silently stopped moving must not look
                // like an empty one.
                <strong>
                  Auto-admit is paused — the cap is full and {queued.length}{' '}
                  {queued.length === 1 ? 'person is' : 'people are'} waiting.
                </strong>
              ) : (
                <>Auto-admit is on: the next {config.cap - admittedCount} get
                straight in.</>
              )}
            </>
          ) : (
            <>
              Gate is <strong>off</strong> — signup is open to anyone and
              invites are not required. This is the default; the waitlist
              still collects addresses.
            </>
          )}
        </p>

        <div
          style={{
            display: 'flex',
            gap: 'var(--s4)',
            alignItems: 'flex-end',
            flexWrap: 'wrap',
            marginTop: 'var(--s4)',
          }}
        >
          <div>
            <label
              htmlFor="waitlist-cap"
              style={{
                display: 'block',
                fontSize: 'var(--fs-sm)',
                color: 'var(--fg-muted)',
                marginBottom: 'var(--s2)',
              }}
            >
              Admission cap
            </label>
            <input
              id="waitlist-cap"
              type="number"
              min={0}
              value={cap}
              disabled={busy}
              onChange={(e) => setCap(e.target.value)}
              style={{ width: '8rem' }}
            />
          </div>
          <label style={{ display: 'flex', gap: 'var(--s2)', alignItems: 'center' }}>
            <input
              type="checkbox"
              checked={gate}
              disabled={busy}
              onChange={(e) => setGate(e.target.checked)}
            />
            Require an invite to sign up
          </label>
          <button
            type="button"
            className="ss-btn"
            onClick={saveConfig}
            disabled={busy}
          >
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </section>

      {notice ? (
        <p role="status" style={{ marginTop: 'var(--s4)' }}>
          {notice}
        </p>
      ) : null}

      <section
        className="ss-card"
        style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s5)' }}
      >
        <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
          Queue · {queued.length}
        </div>

        {queued.length === 0 ? (
          <p style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Nobody is waiting.
          </p>
        ) : (
          <>
            <div style={{ overflowX: 'auto' }}>
              <table className="ss-table ss-table--zebra">
                <thead>
                  <tr>
                    {/* Empty header for the checkbox column. Named via
                        aria-label rather than hidden text — this repo has
                        no sr-only utility, so a <span> here would just
                        render the word "Select" in the header. */}
                    <th scope="col" aria-label="Select" />
                    <th scope="col">Email</th>
                    <th scope="col">Source</th>
                    <th scope="col">Joined</th>
                  </tr>
                </thead>
                <tbody>
                  {queued.map((e) => (
                    <tr key={e.id}>
                      <td>
                        <input
                          type="checkbox"
                          checked={selected.has(e.id)}
                          disabled={busy}
                          onChange={() => toggle(e.id)}
                          aria-label={`Select ${e.email}`}
                        />
                      </td>
                      <td>{e.email}</td>
                      <td style={{ color: 'var(--fg-muted)' }}>
                        {e.source ?? '—'}
                      </td>
                      <td style={{ color: 'var(--fg-muted)' }}>
                        {new Date(e.created_at).toLocaleDateString()}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <button
              type="button"
              className="ss-btn"
              onClick={admit}
              disabled={busy || selected.size === 0}
              style={{ marginTop: 'var(--s4)' }}
            >
              {busy
                ? 'Admitting…'
                : `Admit ${selected.size || ''}`.trim() + ' selected'}
            </button>
            <button
              type="button"
              className="ss-btn ss-btn--danger"
              onClick={() => remove(selected, () => setSelected(new Set()))}
              disabled={busy || selected.size === 0}
              style={{ marginTop: 'var(--s4)', marginLeft: 'var(--s3)' }}
            >
              {busy
                ? 'Deleting…'
                : `Delete ${selected.size || ''}`.trim() + ' from queue'}
            </button>
          </>
        )}
      </section>

      <section
        className="ss-card"
        style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s5)' }}
      >
        <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
          Admitted · {admitted.length}
        </div>
        <p style={{ marginTop: 0, color: 'var(--fg-muted)' }}>
          People who are in. Use <strong>Resend</strong> if someone never got
          their invite — a mail outage at admit time, say. It re-sends the{' '}
          <em>same</em> link, so anything already delivered stays valid.
        </p>

        {admitted.length === 0 ? (
          <p style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Nobody has been admitted yet.
          </p>
        ) : (
          <>
            <div style={{ overflowX: 'auto' }}>
              <table className="ss-table ss-table--zebra">
                <thead>
                  <tr>
                    <th scope="col" aria-label="Select" />
                    <th scope="col">Email</th>
                    <th scope="col">Source</th>
                    <th scope="col">Admitted</th>
                  </tr>
                </thead>
                <tbody>
                  {admitted.map((e) => {
                    // Redeemed = an account already exists for this
                    // invite. `delete_batch`'s SQL predicate refuses these
                    // rows regardless, but a checkbox that submits then
                    // silently does nothing reads as a dead button — so
                    // this row gets a badge and the checkbox is disabled
                    // as a UX hint. It stays a hint, not a guard: the
                    // field can be stale, and `remove()` above still
                    // trusts only the server's response.
                    //
                    // Disabling this same checkbox also blocks Resend,
                    // which is correct, not just harmless: a consumed
                    // token can never be redeemed again (`redeem_invite`
                    // requires `invite_consumed_at IS NULL`), so resending
                    // one would mail a link that can never work.
                    const redeemed = Boolean(e.invite_consumed_at);
                    return (
                      <tr key={e.id}>
                        <td>
                          <input
                            type="checkbox"
                            checked={resendSelected.has(e.id)}
                            disabled={busy || redeemed}
                            onChange={() => toggleResend(e.id)}
                            aria-label={
                              redeemed
                                ? `${e.email} — already redeemed, cannot be resent or deleted`
                                : `Select ${e.email} to resend`
                            }
                            title={
                              redeemed
                                ? 'Invite already redeemed — an account exists, so it cannot be resent or deleted'
                                : undefined
                            }
                          />
                        </td>
                        <td>
                          {e.email}
                          {redeemed ? (
                            <span
                              className="ss-badge ss-badge--ok"
                              style={{ marginLeft: 'var(--s2)' }}
                              title="An account already exists for this invite"
                            >
                              Redeemed
                            </span>
                          ) : null}
                        </td>
                        <td style={{ color: 'var(--fg-muted)' }}>
                          {e.source ?? '—'}
                        </td>
                        <td style={{ color: 'var(--fg-muted)' }}>
                          {e.admitted_at
                            ? new Date(e.admitted_at).toLocaleDateString()
                            : '—'}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            <button
              type="button"
              className="ss-btn"
              onClick={resend}
              disabled={busy || resendSelected.size === 0}
              style={{ marginTop: 'var(--s4)' }}
            >
              {busy
                ? 'Resending…'
                : `Resend ${resendSelected.size || ''}`.trim() + ' selected'}
            </button>
            <button
              type="button"
              className="ss-btn ss-btn--danger"
              onClick={() =>
                remove(resendSelected, () => setResendSelected(new Set()))
              }
              disabled={busy || resendSelected.size === 0}
              style={{ marginTop: 'var(--s4)', marginLeft: 'var(--s3)' }}
            >
              {busy
                ? 'Deleting…'
                : `Delete ${resendSelected.size || ''}`.trim() + ' admitted'}
            </button>
          </>
        )}
      </section>
    </>
  );
}
