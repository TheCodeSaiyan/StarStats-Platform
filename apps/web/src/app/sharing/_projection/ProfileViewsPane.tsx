import React from 'react';
import { Plane, Flatline } from 'holo';
import type { ProfileViewStats } from '@/lib/api';
import { renderBreakdown } from './format';

/**
 * Profile-view counts for a public profile.
 *
 * Tracking only exists while the profile IS public — a private profile has
 * nothing to count — so the private state says that rather than showing a
 * zero. A zero would read as "nobody looked", which is a different and much
 * more discouraging claim.
 *
 * The `data-testid` hooks (`profile-views-card` / `-total` / `-breakdown` /
 * `-sparkline`) are the contract `e2e/profile-view-stats.spec.ts` asserts on
 * and are carried over from the flat card deliberately: the numbers did not
 * change, only their drawing, so the spec should keep passing unchanged.
 */
export function ProfileViewsPane({
  stats,
  isPublic,
}: {
  stats: ProfileViewStats | null;
  isPublic: boolean;
}) {
  if (!isPublic) {
    return (
      <div data-testid="profile-views-card">
        {/* Shipped copy, verbatim. A port redraws; it does not reword. */}
        <p className="hp-prose">
          Make your profile public to start tracking views.
        </p>
      </div>
    );
  }

  const totals = stats?.totals;
  const last30 = totals?.last_30d ?? 0;
  const days = stats?.days ?? [];
  // Oldest-first so the sparkline reads left to right.
  const sparkline = [...days].reverse();
  const maxDay = sparkline.reduce((acc, d) => Math.max(acc, d.total), 0);

  if (last30 === 0) {
    return (
      <div data-testid="profile-views-card">
        <Flatline
          title="No views yet."
          reason="no-data"
          hint="Your profile is public — views appear here once someone opens it."
        />
      </div>
    );
  }

  return (
    <div data-testid="profile-views-card">
      {/* The one figure this pane is about, so it glows — the only thing here
          that does. The breakdown beneath it is a caption and does not. */}
      <div className="hp-viewcount" data-testid="profile-views-total">
        {last30}
      </div>
      <p className="hp-prose" data-testid="profile-views-breakdown">
        {renderBreakdown(totals?.by_source_30d ?? {})}
      </p>
      {sparkline.length > 0 && maxDay > 0 ? (
        <Plane
          tilt="flat"
          cap="Views per day"
          hint={`peak ${maxDay}`}
          style={{ marginTop: 18 }}
        >
          {/* Height and brightness, never hue — the system has one colour per
              calibration, so a categorical scale would survive a recalibration
              as a foreign palette. A zero day draws as a stub rather than
              being dropped: the gaps are part of the shape. */}
          <div
            className="hp-spark"
            aria-label="Daily view counts for the last 30 days"
            data-testid="profile-views-sparkline"
          >
            {sparkline.map((d) => (
              <i
                key={d.day}
                title={`${d.day} · ${d.total} view${d.total === 1 ? '' : 's'}`}
                style={{
                  height: `${Math.max(2, (d.total / maxDay) * 100)}%`,
                }}
                data-peak={d.total === maxDay ? 'true' : undefined}
              />
            ))}
          </div>
        </Plane>
      ) : null}
    </div>
  );
}
