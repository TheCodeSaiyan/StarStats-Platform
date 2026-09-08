'use client';

import React, { useEffect, useRef, useState } from 'react';
import Link from 'next/link';
import type { Route } from 'next';

/**
 * First-run prompt: an account with nothing in it needs the tray before this
 * page can say anything at all.
 *
 * The signal is the LIFETIME event count, not the device list. A paired device
 * that has never uplinked leaves the projection just as empty, and the reader's
 * next action is the same either way — install the tray and point it at
 * `Game.log`. Zero events is also the one number that cannot be a slow window:
 * it means nothing has ever arrived.
 *
 * Shown once. The dismissal is remembered per browser in `localStorage`, which
 * can throw outright (private windows, blocked site data), so every access is
 * guarded and a failure means "not dismissed" — a prompt shown twice is a much
 * smaller fault than a first-run reader who never sees it.
 *
 * Deliberately NOT a hard gate: it can be dismissed, and the empty projection
 * behind it is still readable. The reader may be here to look around.
 */
const STORAGE_KEY = 'starstats.emitter-prompt.dismissed';

function readDismissed(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

function writeDismissed(): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, '1');
  } catch {
    // A reader who blocks site data sees this again next visit. Fine.
  }
}

export function EmitterPrompt() {
  // Starts closed and opens after mount: the dismissal lives in
  // `localStorage`, which the server cannot read, so rendering it open would
  // flash the prompt at every reader who had already dismissed it.
  const [open, setOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!readDismissed()) setOpen(true);
  }, []);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        writeDismissed();
        setOpen(false);
      }
    };
    document.addEventListener('keydown', onKey);
    panelRef.current?.focus();
    return () => document.removeEventListener('keydown', onKey);
  }, [open]);

  if (!open) return null;

  const dismiss = () => {
    writeDismissed();
    setOpen(false);
  };

  return (
    <div className="hp-firstrun">
      <div
        className="hp-firstrun__panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="ss-firstrun-title"
        ref={panelRef}
        tabIndex={-1}
      >
        <div className="hp-firstrun__eyebrow">First run</div>
        <h2 id="ss-firstrun-title">Your projection is empty until you add an Emitter.</h2>
        <p>
          StarStats reads your own <code>Game.log</code>. The Emitter is the
          small desktop app that does the reading — nothing reaches this page
          until it is running and pointed at your install.
        </p>
        <ol className="hp-firstrun__steps">
          <li>Download the Emitter for your platform.</li>
          <li>
            Point it at <code>Game.log</code> in your Star Citizen folder.
          </li>
          <li>Pair it to this account, and turn sync on.</li>
        </ol>
        <div className="hp-firstrun__actions">
          <Link
            className="hp-btn"
            href={'/downloads' as Route}
            onClick={writeDismissed}
          >
            Get the Emitter →
          </Link>
          <button type="button" className="hp-firstrun__later" onClick={dismiss}>
            Look around first
          </button>
        </div>
      </div>
    </div>
  );
}
