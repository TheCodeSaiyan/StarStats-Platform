'use client';

import React from 'react';
import { linkify } from '@/lib/chat/links';

/**
 * A chat message's text with its links checked on this device
 * (`lib/chat/links`). Trusted sites get a badge; anything else shows its
 * real domain; a warned link says why and asks before it opens. Links open
 * in a new tab with no referrer and no access back to this page.
 */
export function MessageText({ text }: { text: string }) {
  return (
    <span style={{ whiteSpace: 'pre-wrap' }}>
      {linkify(text).map((seg, i) =>
        seg.kind === 'text' ? (
          <React.Fragment key={i}>{seg.text}</React.Fragment>
        ) : (
          <span key={i} className="ss-chat__link" data-verdict={seg.verdict}>
            <a
              href={seg.href}
              target="_blank"
              rel="noopener noreferrer nofollow ugc"
              onClick={(e) => {
                if (seg.verdict !== 'warn') return;
                const ok = window.confirm(
                  `Careful: this link ${seg.reasons.join('; ')}.\n\nIt goes to ${seg.host}. Open it anyway?`,
                );
                if (!ok) e.preventDefault();
              }}
            >
              {seg.text}
            </a>
            {seg.verdict === 'trusted' ? (
              <span className="ss-chat__linktag"> ✓ {seg.host}</span>
            ) : seg.verdict === 'warn' ? (
              <span className="ss-chat__linktag ss-chat__linktag--warn" role="note">
                {' '}
                ⚠ {seg.reasons[0]}
              </span>
            ) : (
              <span className="ss-chat__linktag"> ({seg.host})</span>
            )}
          </span>
        ),
      )}
    </span>
  );
}
