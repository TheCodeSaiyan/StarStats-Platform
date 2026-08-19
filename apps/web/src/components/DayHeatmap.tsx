/**
 * Per-day event-count heatmap. Originally inlined in the dashboard;
 * extracted so the public profile page (`/u/[handle]`) can reuse it
 * for the public/friend timeline. The two server-side response types
 * (`TimelineResponse` for the owner, `PublicTimelineResponse` for the
 * shared/public path) are structurally identical — both deliver
 * `{ buckets: [{ count, date }], days }` — so this component takes
 * the structural shape rather than coupling to either generated type.
 *
 * Render decisions:
 *   - 7-row grid (Sunday → Saturday) with leading-offset padding.
 *   - Quartile-ish heat levels relative to the max count in range.
 *     Empty days get level=undefined so the cell renders as bg-muted.
 *   - Footer caption shows total events + day range and a Less→More
 *     legend that mirrors the cell levels.
 */

export interface DayHeatmapBucket {
  count: number;
  /** YYYY-MM-DD in UTC. */
  date: string;
}

export interface DayHeatmapData {
  buckets: DayHeatmapBucket[];
  /** The window the buckets span — surfaced in the caption. */
  days: number;
}

function parseIsoDate(raw: string): Date | null {
  const m = raw.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return null;
  // Use local-noon to avoid TZ flips at day boundaries; we only care
  // about the day-of-week, and midnight + DST can land on the wrong
  // side. Same anchor the dashboard's original inline copy used.
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 12, 0, 0);
}

export function DayHeatmap({ timeline }: { timeline: DayHeatmapData }) {
  const buckets = timeline.buckets;
  if (buckets.length === 0) {
    return (
      <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
        Scope is clear.
      </p>
    );
  }

  const max = buckets.reduce((m, b) => (b.count > m ? b.count : m), 0);
  // Quartile-ish thresholds. For tiny ranges (e.g. max=2) everything
  // collapses sensibly into the lower levels.
  const levelFor = (count: number): 1 | 2 | 3 | 4 | undefined => {
    if (count <= 0) return undefined;
    if (max <= 0) return undefined;
    const ratio = count / max;
    if (ratio > 0.75) return 4;
    if (ratio > 0.5) return 3;
    if (ratio > 0.25) return 2;
    return 1;
  };

  // Parse bucket dates as local date components (the API returns
  // YYYY-MM-DD UTC). Compute leading offset so column 0 starts on
  // Sunday — matches the design's S/M/T/W/T/F/S row legend.
  const first = parseIsoDate(buckets[0].date);
  const leadingOffset = first ? first.getDay() : 0;

  type Cell = { date: string; count: number; level: 1 | 2 | 3 | 4 | undefined };
  const cells: Array<Cell | null> = [];
  for (let i = 0; i < leadingOffset; i++) cells.push(null);
  for (const b of buckets) {
    cells.push({ date: b.date, count: b.count, level: levelFor(b.count) });
  }
  // Pad trailing so the last column is complete.
  while (cells.length % 7 !== 0) cells.push(null);

  const weekCount = cells.length / 7;
  const totalForLabel = buckets.reduce((acc, b) => acc + b.count, 0);

  return (
    <div className="ss-heatmap-wrap" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
        <div
          style={{
            display: 'grid',
            gridTemplateRows: 'repeat(7, 14px)',
            gap: 3,
            fontSize: 10,
            color: 'var(--fg-dim)',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
          }}
        >
          {['S', 'M', 'T', 'W', 'T', 'F', 'S'].map((d, i) => (
            <span
              key={i}
              style={{
                lineHeight: '14px',
                textAlign: 'right',
                opacity: i % 2 ? 1 : 0,
              }}
            >
              {d}
            </span>
          ))}
        </div>
        <div
          className="ss-heatmap"
          style={{ gridTemplateColumns: `repeat(${weekCount}, 14px)` }}
          role="img"
          aria-label={`Per-day event counts for the last ${timeline.days} days`}
        >
          {cells.map((cell, i) => (
            <span
              key={i}
              className="ss-heatcell"
              data-l={cell?.level}
              title={
                cell
                  ? `${cell.date}: ${cell.count.toLocaleString()} ${cell.count === 1 ? 'event' : 'events'}`
                  : undefined
              }
            />
          ))}
        </div>
      </div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          fontSize: 11,
          color: 'var(--fg-dim)',
        }}
      >
        <span>
          <span className="mono">{totalForLabel.toLocaleString()}</span>{' '}
          events · last {timeline.days} days
        </span>
        <span style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
          <span>Less</span>
          {[undefined, 1, 2, 3, 4].map((l) => (
            <span
              key={String(l)}
              className="ss-heatcell"
              data-l={l}
              style={{ width: 10, height: 10 }}
            />
          ))}
          <span>More</span>
        </span>
      </div>
    </div>
  );
}
