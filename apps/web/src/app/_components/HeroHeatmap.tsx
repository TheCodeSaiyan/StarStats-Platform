/**
 * Server-rendered heatmap used by the marketing landing's hero mock.
 * Deterministic LCG seeded from the prototype so the preview stays
 * stable between renders. Pure presentation — no client behaviour.
 */

const WEEKS = 22;
const SEED = 11;

function buildCells(weeks: number, seed: number): number[] {
  const cells: number[] = [];
  let s = seed;
  const rng = () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 0xffffffff;
  };
  for (let w = 0; w < weeks; w += 1) {
    for (let d = 0; d < 7; d += 1) {
      const recencyBoost = w / weeks;
      const r = rng();
      const r2 = rng();
      let lvl = 0;
      const t = r * (0.3 + recencyBoost * 0.7);
      if (t > 0.18) lvl = 1;
      if (t > 0.32) lvl = 2;
      if (t > 0.55) lvl = 3;
      if (t > 0.78) lvl = 4;
      if (r2 < 0.12) lvl = 0;
      cells.push(lvl);
    }
  }
  return cells;
}

const DAY_LABELS = ['S', 'M', 'T', 'W', 'T', 'F', 'S'] as const;

export function HeroHeatmap({ label = '2 412 events tracked' }: { label?: string }) {
  const cells = buildCells(WEEKS, SEED);
  return (
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
        {DAY_LABELS.map((d, i) => (
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
      <div>
        <div
          className="ss-heatmap"
          style={{ gridTemplateColumns: `repeat(${WEEKS}, 14px)` }}
        >
          {cells.map((lvl, i) => (
            <span
              key={i}
              className="ss-heatcell"
              data-l={lvl || undefined}
              title={`${label} · level ${lvl}`}
            />
          ))}
        </div>
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginTop: 10,
            fontSize: 11,
            color: 'var(--fg-dim)',
          }}
        >
          <span>{label}</span>
          <span style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
            <span>Less</span>
            {[0, 1, 2, 3, 4].map((l) => (
              <span
                key={l}
                className="ss-heatcell"
                data-l={l || undefined}
                style={{ width: 10, height: 10 }}
              />
            ))}
            <span>More</span>
          </span>
        </div>
      </div>
    </div>
  );
}
