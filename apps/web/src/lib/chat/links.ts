/**
 * Link warnings for chat (social design §4.6.5), run on the recipient's
 * device only: the server cannot see links in an encrypted message and
 * never learns what anyone linked or clicked.
 *
 * The scams this community sees are mostly RSI lookalike login pages, so
 * the heuristics and the trusted list carry most of the value. A hashed
 * blocklist can come later, pulled like the parser definitions.
 */

/** Domains that get a trusted badge; subdomains included. */
export const TRUSTED_DOMAINS = [
  'robertsspaceindustries.com',
  'cloudimperiumgames.com',
  'starcitizen.tools',
  'starstats.app',
] as const;

const SHORTENERS = new Set([
  'bit.ly',
  'tinyurl.com',
  't.co',
  'goo.gl',
  'is.gd',
  'ow.ly',
  'cutt.ly',
  'rb.gy',
  'shorturl.at',
  'tiny.cc',
  'buff.ly',
]);

const DOWNLOAD_EXT = /\.(exe|msi|scr|bat|cmd|ps1|vbs|jar|zip|rar|7z|apk|dmg|iso)$/i;

export type LinkVerdict = 'trusted' | 'unknown' | 'warn';

export interface LinkCheck {
  href: string;
  host: string;
  verdict: LinkVerdict;
  /** Why a link is warned about, in plain words. */
  reasons: string[];
}

export type Segment = { kind: 'text'; text: string } | ({ kind: 'link'; text: string } & LinkCheck);

/** Levenshtein distance, small and only for short names. */
function distance(a: string, b: string): number {
  const d = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) d[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      d[i][j] = Math.min(
        d[i - 1][j] + 1,
        d[i][j - 1] + 1,
        d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
    }
  }
  return d[a.length][b.length];
}

function isTrusted(host: string): string | null {
  return TRUSTED_DOMAINS.find((d) => host === d || host.endsWith(`.${d}`)) ?? null;
}

/** The registrable part, roughly: the last two labels. */
function registrable(host: string): string {
  return host.split('.').slice(-2).join('.');
}

export function checkLink(href: string): LinkCheck | null {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') return null;
  const host = url.hostname.toLowerCase().replace(/\.$/, '');
  const reasons: string[] = [];
  const trusted = isTrusted(host);

  if (url.protocol === 'http:') reasons.push('not a secure (https) link');
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.startsWith('[')) {
    reasons.push('goes to a bare IP address, not a named site');
  }
  if (host.split('.').some((l) => l.startsWith('xn--'))) {
    reasons.push('the address uses look-alike characters');
  }
  if (SHORTENERS.has(host)) reasons.push('a shortened link hides where it really goes');
  if (DOWNLOAD_EXT.test(url.pathname)) reasons.push('downloads a file');
  if (url.username || url.password) reasons.push('hides the real address behind a login part');
  if (!trusted) {
    for (const d of TRUSTED_DOMAINS) {
      const name = d.split('.')[0];
      const hostName = registrable(host).split('.')[0];
      if (host.includes(`${d}.`) || host.includes(`${name}-`) || host.includes(`-${name}`)) {
        reasons.push(`uses the name ${d} but is not that site`);
        break;
      }
      if (hostName !== name && hostName.length > 4 && distance(hostName, name) <= 2) {
        reasons.push(`looks like ${d} but is not that site`);
        break;
      }
    }
  }
  const verdict: LinkVerdict = reasons.length > 0 ? 'warn' : trusted ? 'trusted' : 'unknown';
  return { href: url.toString(), host, verdict, reasons };
}

const URL_RE = /\bhttps?:\/\/[^\s<>"'`]+/gi;

/** Split a message into text and checked links. Trailing punctuation that
 * ends a sentence is left out of the link. */
export function linkify(text: string): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    let raw = m[0];
    const trail = /[.,;:!?)\]]+$/.exec(raw)?.[0] ?? '';
    if (trail) raw = raw.slice(0, -trail.length);
    const start = m.index ?? 0;
    const check = checkLink(raw);
    if (!check) continue;
    if (start > last) out.push({ kind: 'text', text: text.slice(last, start) });
    out.push({ kind: 'link', text: raw, ...check });
    last = start + raw.length;
  }
  if (last < text.length) out.push({ kind: 'text', text: text.slice(last) });
  return out;
}
