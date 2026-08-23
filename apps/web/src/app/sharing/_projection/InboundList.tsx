import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import {
  Plane,
  BeamChip,
  BeamButton,
  BeamSelect,
  BeamTextarea,
} from 'holo';
import type { SharedWithMeEntry } from '@/lib/api';
import { formatExpiry } from './format';

/**
 * Shares other people have granted you.
 *
 * Active and expired are separated because they mean different things: an
 * expired grant is history, not a broken live one, and mixing them makes a
 * reader think access is still open.
 */
export function InboundList({
  entries,
  reportShareAction,
}: {
  entries: readonly SharedWithMeEntry[];
  reportShareAction: (formData: FormData) => void | Promise<void>;
}) {
  if (entries.length === 0) {
    return (
      <p className="hp-prose">
        Nobody has shared their manifest with you yet.
      </p>
    );
  }

  const now = Date.now();
  const isExpired = (e: SharedWithMeEntry) =>
    e.expires_at != null && new Date(e.expires_at).getTime() <= now;
  const active = entries.filter((e) => !isExpired(e));
  const expired = entries.filter(isExpired);

  return (
    <>
      {active.length > 0 ? (
        <Plane tilt="flat" cap="Active" style={{ marginTop: 18 }}>
          {active.map((entry) => (
            <InboundRow
              key={entry.owner_handle}
              entry={entry}
              reportShareAction={reportShareAction}
            />
          ))}
        </Plane>
      ) : null}
      {expired.length > 0 ? (
        <Plane tilt="flat" cap="Expired" hint="history" style={{ marginTop: 18 }}>
          {expired.map((entry) => (
            <InboundRow
              key={entry.owner_handle}
              entry={entry}
              reportShareAction={reportShareAction}
              expired
            />
          ))}
        </Plane>
      ) : null}
    </>
  );
}

function InboundRow({
  entry,
  reportShareAction,
  expired = false,
}: {
  entry: SharedWithMeEntry;
  reportShareAction: (formData: FormData) => void | Promise<void>;
  expired?: boolean;
}) {
  const expiryLabel = formatExpiry(entry.expires_at);
  return (
    <div className="hp-grant" data-expired={expired ? 'true' : undefined}>
      <div className="hp-grant__who">
        <Link
          href={`/u/${encodeURIComponent(entry.owner_handle)}` as Route}
        >
          @{entry.owner_handle}
        </Link>
        {entry.note ? (
          <span className="hp-grant__note">{entry.note}</span>
        ) : null}
      </div>
      {expiryLabel ? (
        <BeamChip
          tone={expiryLabel === 'expired' ? 'bad' : undefined}
          title={entry.expires_at ?? undefined}
        >
          {expiryLabel === 'expired' ? 'expired' : `expires ${expiryLabel}`}
        </BeamChip>
      ) : null}
      <div className="hp-grant__act-btns">
        <Link href={`/u/${encodeURIComponent(entry.owner_handle)}` as Route}>
          View profile →
        </Link>
      </div>
      {/* Recipient-facing report affordance. Collapsed by default so the row
          stays compact — `<details>` because this is genuinely a disclosure
          and the system ships no accordion to reach for. */}
      <details className="hp-report">
        <summary>Report this share</summary>
        <form action={reportShareAction} className="hp-formcol">
          <input type="hidden" name="owner_handle" value={entry.owner_handle} />
          {/* `recipient_handle` (the REPORTER) is deliberately not sent: the
              action reads it from `session.claimedHandle`. Server-side handle
              truth — the client may only name the OTHER party. */}
          <BeamSelect
            id={`report-reason-${entry.owner_handle}`}
            name="reason"
            label="Reason"
            required
            defaultValue="abuse"
          >
            <option value="abuse">Abuse</option>
            <option value="spam">Spam</option>
            <option value="data_misuse">Data misuse</option>
            <option value="other">Other</option>
          </BeamSelect>
          {/* `details`, PLURAL — `reportShareAction` reads
              `formData.get('details')`. The port had this as `detail`, so
              every report submitted with the reporter's explanation silently
              dropped. A textarea, not an input: this is up to 500 characters
              of incident description. */}
          <BeamTextarea
            id={`report-details-${entry.owner_handle}`}
            label="Details"
            name="details"
            rows={3}
            maxLength={500}
            placeholder="Optional, max 500 chars"
          />
          <BeamButton
            type="submit"
            variant="danger"
            style={{ alignSelf: 'flex-start' }}
          >
            Submit report
          </BeamButton>
        </form>
      </details>
    </div>
  );
}
