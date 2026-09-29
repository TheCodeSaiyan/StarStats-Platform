import Link from 'next/link';
import React from 'react';
import type { Friend } from '@/lib/api';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { openDmAction } from './actions';

/**
 * Start (or reopen) a DM from /chat itself. Friends only, as the API
 * requires; a friend who cannot be messaged gets the page's error line
 * back rather than a dead end, because `from=chat` routes it here.
 */
export function StartChat({ friends }: { friends: Friend[] }) {
  if (friends.length === 0) {
    return (
      <p className="ss-chat__muted" data-testid="start-chat">
        You can message friends. <Link href="/friends">Add friends</Link> to start a chat.
      </p>
    );
  }
  const sorted = [...friends].sort((a, b) => a.handle.localeCompare(b.handle));
  return (
    <form action={openDmAction} className="ss-chat__start" data-testid="start-chat">
      <input type="hidden" name="from" value="chat" />
      <label className="ss-label">
        <span className="ss-label-text">New message</span>
        <select className="ss-input" name="handle" required defaultValue="">
          <option value="" disabled>
            Choose a friend
          </option>
          {sorted.map((f) => (
            <option key={f.handle} value={f.handle}>
              @{f.handle}
            </option>
          ))}
        </select>
      </label>
      <ConfirmSubmitButton className="hp-btn">Message</ConfirmSubmitButton>
    </form>
  );
}
