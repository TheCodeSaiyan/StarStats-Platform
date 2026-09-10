import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Plane } from 'holo';
import type { OutstandingTask } from '@/lib/outstanding-tasks';

/**
 * What still needs doing, stated once at the top of the projection.
 *
 * Replaces the first-run modal. A modal is the right shape for a welcome
 * and the wrong one for a fault: it interrupts, it is dismissed, and the
 * dismissal outlives the problem. This renders only while something is
 * actually outstanding and stops rendering when it is fixed, so there is
 * nothing to dismiss and nothing to remember.
 *
 * Every row states the CONSEQUENCE rather than restating the title. "Turn
 * sync on" tells a reader what to click; "it is reading your log and
 * keeping it to itself" tells them why they have no data, which is the
 * thing they actually came to find out.
 *
 * Renders nothing for an empty list — the caller does not need to guard.
 */
export function OutstandingTasks({ tasks }: { tasks: OutstandingTask[] }) {
  if (tasks.length === 0) return null;

  return (
    <Plane
      tilt="flat"
      cap={tasks.length === 1 ? 'Needs doing' : `Needs doing · ${tasks.length}`}
      hint="these clear themselves once sorted"
      style={{ marginBottom: 'var(--s5)' }}
    >
      <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
        {tasks.map((t) => (
          <li
            key={t.id}
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              alignItems: 'baseline',
              gap: 'var(--s3)',
              padding: 'var(--s3) 0',
            }}
          >
            <div style={{ flex: '1 1 22ch', minWidth: 0 }}>
              <p className="hp-prose" style={{ margin: 0 }}>
                <strong>{t.title}</strong>
              </p>
              <p className="hp-note" style={{ margin: '4px 0 0' }}>
                {t.detail}
              </p>
            </div>
            <Link href={t.href as Route} className="hp-btn hp-btn--ghost">
              {t.cta} →
            </Link>
          </li>
        ))}
      </ul>
    </Plane>
  );
}
