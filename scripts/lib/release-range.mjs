// Which commits a release's notes cover.
//
// Tags: tray `tray-vX.Y.Z[-alpha|beta|rc.N]`, platform `vX.Y.Z[...]`.
// A LIVE release is measured from the previous LIVE tag on its track, so a
// player on live reads everything since their last update, not just what
// changed since the last release candidate. A pre-release is measured from
// the previous tag of any kind on its track, so alpha testers see only what
// is new to them.

const TAG = /^(?<prefix>tray-)?v(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)(?:-(?<channel>alpha|beta|rc)\.(?<n>\d+))?$/;

export function parseTag(tag) {
  const m = TAG.exec(tag);
  if (!m) return null;
  const g = m.groups;
  return {
    tag,
    track: g.prefix ? 'tray' : 'platform',
    version: `${g.major}.${g.minor}.${g.patch}`,
    channel: g.channel ?? 'live',
    nums: [Number(g.major), Number(g.minor), Number(g.patch)],
    pre: g.channel ? { rank: { alpha: 0, beta: 1, rc: 2 }[g.channel], n: Number(g.n) } : null,
  };
}

/** Semver order; a pre-release sorts before its release. */
export function compareTags(a, b) {
  for (let i = 0; i < 3; i++) if (a.nums[i] !== b.nums[i]) return a.nums[i] - b.nums[i];
  if (!a.pre && !b.pre) return 0;
  if (!a.pre) return 1;
  if (!b.pre) return -1;
  return a.pre.rank - b.pre.rank || a.pre.n - b.pre.n;
}

/**
 * The tag a release's notes start from, or null for the first release on
 * the track.
 * @param {string[]} allTags every tag in the repository
 * @param {string} tag the release being described
 */
export function previousTag(allTags, tag) {
  const cur = parseTag(tag);
  if (!cur) throw new Error(`not a release tag: ${tag}`);
  const candidates = allTags
    .map(parseTag)
    .filter((t) => t && t.track === cur.track && compareTags(t, cur) < 0)
    .filter((t) => (cur.channel === 'live' ? t.channel === 'live' : true))
    .sort(compareTags);
  return candidates.length ? candidates[candidates.length - 1].tag : null;
}

/** The newest LIVE tag on a track, or null when it has none yet. */
export function latestLiveTag(allTags, track) {
  const live = allTags
    .map(parseTag)
    .filter((t) => t && t.track === track && t.channel === 'live')
    .sort(compareTags);
  return live.length ? live[live.length - 1].tag : null;
}
