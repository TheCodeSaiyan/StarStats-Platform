/**
 * Per-channel status strip rendered on each roadmap card / detail
 * page. Shows up to N chips (default 4); on a card it stays compact,
 * on the detail page it expands with build_health + commit sha.
 */

import type { ChannelStatusPublic } from '@/lib/roadmap';
import { StatusBadge } from './StatusBadge';

const CHANNEL_LABEL: Record<string, string> = {
  live: 'Live',
  beta: 'Beta',
  alpha: 'Alpha',
  'tech-preview': 'Tech preview',
};

function channelLabel(c: string): string {
  return CHANNEL_LABEL[c] ?? c;
}

export function ChannelChipStrip({
  channels,
  detailed = false,
}: {
  channels: ChannelStatusPublic[];
  detailed?: boolean;
}) {
  if (!channels.length) {
    return (
      <span
        className="ss-eyebrow"
        style={{ color: 'var(--fg-dim)', fontSize: 12 }}
      >
        Not yet targeted
      </span>
    );
  }
  return (
    <div
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 8,
        alignItems: 'center',
      }}
    >
      {channels.map((c) => (
        <div
          key={c.channel}
          title={
            detailed
              ? `${channelLabel(c.channel)}: ${c.status} (${c.build_health})`
              : undefined
          }
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            padding: '4px 10px',
            borderRadius: 8,
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            fontSize: 12,
          }}
        >
          <span style={{ color: 'var(--fg-dim)', fontWeight: 500 }}>
            {channelLabel(c.channel)}
          </span>
          <StatusBadge status={c.status} />
          {detailed && c.build_health !== 'unknown' && (
            <span
              style={{
                fontSize: 11,
                color:
                  c.build_health === 'failing'
                    ? 'var(--warn, var(--fg-dim))'
                    : 'var(--fg-dim)',
              }}
            >
              · {c.build_health}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}
