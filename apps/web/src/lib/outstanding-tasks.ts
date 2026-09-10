import type { DeviceDto } from '@/lib/api';

/**
 * What still needs doing before StarStats works, derived from live state.
 *
 * Replaces the first-run modal, which had two problems. It fired only on
 * `summary.total === 0`, so an account whose uplink later stopped syncing
 * was told nothing — the one case where silence looks exactly like working
 * normally. And it dismissed to `localStorage` forever, which is right for
 * a welcome and wrong for a fault: dismiss it once, and a problem that
 * comes back never speaks again.
 *
 * So there is no dismissal here and no stored state. A task is present
 * because the thing is true right now, and it disappears when it stops
 * being true. If that means someone sees a row twice, the row was right
 * twice.
 */
export interface OutstandingTask {
  id: string;
  /** The action, imperative. */
  title: string;
  /** What is happening while it goes undone — the consequence, not a restatement. */
  detail: string;
  href: string;
  cta: string;
}

/**
 * How long an uplink can go unheard-from before we say so.
 *
 * Seven days rather than something twitchy: people don't play every day,
 * and a banner that appears every time somebody takes a weekend off is one
 * people learn to ignore — at which point it is worse than nothing, because
 * it still looks like cover.
 */
export const UPLINK_STALE_DAYS = 7;

export interface TaskInput {
  devices: DeviceDto[];
  /** Events the account holds. `null` when the summary call failed. */
  eventTotal: number | null;
  rsiVerified: boolean;
  now: Date;
}

/**
 * The data path is a PIPELINE, so at most one of its stages is reported.
 *
 * Pair → sync on → reading the log → still reporting. Each stage only
 * matters once the one before it is satisfied, and listing "pair an uplink"
 * beside "turn sync on" would be incoherent — there is nothing to turn sync
 * on for. Showing the earliest unmet stage is both the true answer and the
 * only actionable one.
 */
function dataPathTask(input: TaskInput): OutstandingTask | null {
  const { devices, eventTotal, now } = input;

  if (devices.length === 0) {
    return {
      id: 'pair-uplink',
      title: 'Pair an uplink',
      detail:
        'Nothing is reading your Game.log yet, so nothing can arrive. The uplink is the desktop app that does the reading.',
      href: '/downloads',
      cta: 'Get the uplink',
    };
  }

  const syncing = devices.filter((d) => d.sync_enabled);
  if (syncing.length === 0) {
    return {
      id: 'enable-sync',
      title: 'Turn sync on',
      detail:
        devices.length === 1
          ? `${devices[0].label} is paired but not syncing — it is reading your log and keeping it to itself.`
          : 'Your uplinks are paired but none are syncing — they are reading your log and keeping it to themselves.',
      href: '/downloads',
      cta: 'Open Emitter',
    };
  }

  // Paired and syncing, and still nothing has landed. The uplink is running
  // but not pointed at a log it can read.
  if (eventTotal === 0) {
    return {
      id: 'point-at-log',
      title: 'Point the uplink at your Game.log',
      detail:
        'An uplink is paired and syncing, but no events have arrived. That usually means it has the wrong folder.',
      href: '/docs/troubleshooting',
      cta: 'Troubleshooting',
    };
  }

  // Everything is wired up — has it gone quiet? Only meaningful for the
  // uplinks actually meant to be sending.
  const cutoff = now.getTime() - UPLINK_STALE_DAYS * 24 * 60 * 60 * 1000;
  const heard = syncing
    .map((d) => (d.last_seen_at ? Date.parse(d.last_seen_at) : NaN))
    .filter((t) => Number.isFinite(t));

  if (heard.length > 0 && Math.max(...heard) < cutoff) {
    return {
      id: 'uplink-quiet',
      title: 'An uplink has gone quiet',
      detail: `Nothing has been received for over ${UPLINK_STALE_DAYS} days. If you have been playing, the uplink is not running.`,
      href: '/downloads',
      cta: 'Check uplinks',
    };
  }

  return null;
}

/**
 * Blockers that are not about data flowing, and so stack with it.
 *
 * RSI verification is here because failing it is SILENT: the sharing page
 * lets you set things up and the grants simply never work, which is a
 * worse experience than being told up front.
 */
function accountTasks(input: TaskInput): OutstandingTask[] {
  if (input.rsiVerified) return [];
  return [
    {
      id: 'verify-rsi',
      title: 'Verify your RSI handle',
      detail:
        'Until it is verified you cannot make your profile public or share with an org — the options are there, they just will not take.',
      href: '/settings',
      cta: 'Verify',
    },
  ];
}

export function outstandingTasks(input: TaskInput): OutstandingTask[] {
  const path = dataPathTask(input);
  return [...(path ? [path] : []), ...accountTasks(input)];
}
