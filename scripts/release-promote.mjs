#!/usr/bin/env node
// scripts/release-promote.mjs
//
// Channel promotion for the StarStats two-branch + two-track release
// model. See:
//   - the release design notes
//   - the release design notes
//
// Two tracks ship independently:
//   - `tray`     — Tauri desktop client. Tags: tray-vX.Y.Z[-channel.N].
//                  Versions live in crates/starstats-client/Cargo.toml
//                  + crates/starstats-client/tauri.conf.json.
//   - `platform` — server + web container images. Tags: vX.Y.Z[-channel.N].
//                  Version lives in workspace Cargo.toml ([workspace.package]),
//                  inherited by starstats-core + starstats-server.
//
// Subcommands:
//   prerelease <tray|platform> <alpha|beta|rc> [--sha <sha>] [--n <num>] [--dry-run]
//     On `next`: bump versions to X.Y.Z-channel.N, commit, tag, push.
//   live <tray|platform> [--sha <sha>] [--dry-run]
//     Fast-forward `main` to a `next` SHA, bump versions to bare X.Y.Z,
//     commit, tag, push.
//   hotfix-finish [--dry-run]
//     Merge `main` into `next` to restore Invariant #1 after a hotfix.
//     Track-agnostic (main → next merge applies regardless of track).
//
// Pure helpers (computeNextVersion, detectChannel, bumpCargoToml,
// bumpClientCargo, bumpTauriConf, latestSemverFromTags, …) are
// exported for the unit test in release-promote.test.mjs.
//
// Zero npm deps. Uses node:fs, node:path, node:process. All shell
// invocations go through execFileSync (no shell, no injection vector;
// all arguments are validated literals or git-rev strings we computed
// ourselves).

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CHANNELS = ["alpha", "beta", "rc"];
const TRACKS = ["tray", "platform"];
const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.(\d+))?$/;

// ---------------------------------------------------------------------------
// Track-aware tag plumbing
// ---------------------------------------------------------------------------

/**
 * Tag prefix for a track. Tray tags are `tray-vX.Y.Z`; platform tags
 * are bare `vX.Y.Z` (unchanged from the single-track era for
 * compatibility with the deployed homelab Komodo sync + roadmap-emit
 * receiver wiring).
 */
export function tagPrefix(track) {
  if (track === "tray") return "tray-v";
  if (track === "platform") return "v";
  throw new Error(`unknown track: ${JSON.stringify(track)}`);
}

/**
 * Parse a tag string into `{track, version}` or null if the tag
 * doesn't belong to either track. Distinguishes `tray-vX.Y.Z` from
 * bare `vX.Y.Z`.
 */
export function parseTrackTag(tagName) {
  const t = (tagName ?? "").trim();
  if (!t) return null;
  if (t.startsWith("tray-v")) {
    const v = t.slice("tray-v".length);
    if (SEMVER_RE.test(v)) return { track: "tray", version: v };
    return null;
  }
  if (t.startsWith("v")) {
    const v = t.slice(1);
    if (SEMVER_RE.test(v)) return { track: "platform", version: v };
    return null;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Pure helpers — exported for tests
// ---------------------------------------------------------------------------

/**
 * Parse `X.Y.Z` or `X.Y.Z-channel.N` into a structured object.
 */
export function parseVersion(version) {
  if (typeof version !== "string" || version.length === 0) {
    throw new Error(`invalid version (empty): ${JSON.stringify(version)}`);
  }
  const m = SEMVER_RE.exec(version);
  if (!m) {
    throw new Error(`invalid version string: ${version}`);
  }
  return {
    major: Number(m[1]),
    minor: Number(m[2]),
    patch: Number(m[3]),
    channel: m[4] ?? null,
    n: m[5] === undefined ? null : Number(m[5]),
  };
}

/**
 * Detect the release channel implied by a version string.
 * 1.2.3-alpha.4 → "alpha"; 1.2.3 → "live".
 */
export function detectChannel(version) {
  const parsed = parseVersion(version);
  if (parsed.channel === null) return "live";
  return parsed.channel;
}

/**
 * Strip any pre-release suffix to return bare X.Y.Z. This is the value
 * that must be written to Cargo.toml + tauri.conf.json — Tauri's WiX
 * MSI bundler rejects any non-numeric pre-release identifier ("optional
 * pre-release identifier in app version must be numeric-only and cannot
 * be greater than 65535 for msi target"), so the suffix lives only on
 * the git tag, not in the build configs. The Tauri updater manifest
 * still carries the full version because release.yml passes the full
 * tag name into generate-updater-manifest.mjs --version.
 */
export function bareSemver(version) {
  return version.replace(/-.*$/, "");
}

/**
 * Compare two parsed versions per semver precedence rules.
 * Returns negative if a < b, positive if a > b, 0 if equal.
 *
 * Critical: pre-release versions have LOWER precedence than the
 * matching release (semver §11: "When major, minor, and patch are
 * equal, a pre-release version has lower precedence than a normal
 * version"). git's `--sort=version:refname` does NOT honour this —
 * it compares strings, so `v1.8.2-alpha.7` sorts ABOVE `v1.8.2`
 * because `-alpha.7` is a longer suffix. We have to sort in JS.
 *
 * Numeric N comparison is also crucial — git's lex sort would put
 * `alpha.10 < alpha.2`. Here we parse N as a number.
 */
export function compareVersions(a, b) {
  if (a.major !== b.major) return a.major - b.major;
  if (a.minor !== b.minor) return a.minor - b.minor;
  if (a.patch !== b.patch) return a.patch - b.patch;
  // Release > pre-release at the same X.Y.Z.
  if (a.channel === null && b.channel !== null) return 1;
  if (a.channel !== null && b.channel === null) return -1;
  if (a.channel === null && b.channel === null) return 0;
  // Both pre-release: channel order alpha < beta < rc happens to be
  // alphabetical, which is what semver §11 specifies for identifier
  // comparison ("identifiers consisting of only digits are compared
  // numerically; identifiers with letters or hyphens are compared
  // lexically").
  if (a.channel !== b.channel) return a.channel < b.channel ? -1 : 1;
  return a.n - b.n;
}

/**
 * Given a list of git tag names and an optional `track`, return the
 * highest recognised semver string (without the leading `v` /
 * `tray-v` prefix), or null if none.
 *
 * When `track` is provided, tags belonging to the OTHER track are
 * silently skipped — critical because the script may be passed the
 * entire `git tag -l` output.
 *
 * Sort respects semver precedence: release > pre-release of same
 * X.Y.Z; rc > beta > alpha; higher N within same channel; numeric
 * (not lex) N comparison.
 */
/**
 * Given a list of git tag names, an optional `track`, a target
 * `X.Y.Z` (as a "major.minor.patch" string), and a target `channel`,
 * return the highest `N` seen for `vX.Y.Z-channel.N` tags in that
 * cycle. Returns 0 if no matching tag exists.
 *
 * Used by `computeNextVersion` to correctly advance N when switching
 * channels within a cycle that already has tags for the target
 * channel — without this, computing the "next alpha" after a beta
 * was cut on the same X.Y.Z naively returns `alpha.1` and collides
 * with the existing `alpha.1` from earlier in the cycle.
 *
 * Like `latestSemverFromTags`, tags belonging to the OTHER track are
 * silently skipped when `track` is supplied.
 */
export function highestNForCycleChannel(tagList, track, xyz, channel) {
  if (track !== undefined && !TRACKS.includes(track)) {
    throw new Error(
      `highestNForCycleChannel: unknown track ${JSON.stringify(track)}; expected one of ${TRACKS.join(", ")}`,
    );
  }
  if (!CHANNELS.includes(channel)) {
    throw new Error(
      `highestNForCycleChannel: unknown channel ${JSON.stringify(channel)}; expected one of ${CHANNELS.join(", ")}`,
    );
  }
  let max = 0;
  for (const raw of tagList) {
    const meta = parseTrackTag(raw);
    if (meta === null) continue;
    if (track !== undefined && meta.track !== track) continue;
    const v = parseVersion(meta.version);
    if (v.channel !== channel) continue;
    if (`${v.major}.${v.minor}.${v.patch}` !== xyz) continue;
    if (v.n > max) max = v.n;
  }
  return max;
}

export function latestSemverFromTags(tagList, track) {
  if (track !== undefined && !TRACKS.includes(track)) {
    throw new Error(
      `latestSemverFromTags: unknown track ${JSON.stringify(track)}; expected one of ${TRACKS.join(", ")}`,
    );
  }
  const parsed = [];
  for (const raw of tagList) {
    const meta = parseTrackTag(raw);
    if (meta === null) continue;
    if (track !== undefined && meta.track !== track) continue;
    parsed.push({ str: meta.version, parsed: parseVersion(meta.version) });
  }
  if (parsed.length === 0) return null;
  parsed.sort((a, b) => compareVersions(b.parsed, a.parsed));
  return parsed[0].str;
}

/**
 * Return the highest track tag whose version is STRICTLY BELOW
 * `currentVersion`. Used to define the SHA range to scan for merged
 * PR labels when auto-discovering the roadmap slug for a release.
 *
 * Pure — no side effects.
 *
 * The "previous" tag is always of the same track (so a tray release
 * doesn't accidentally scan PRs against a platform-only tag range),
 * but is channel-agnostic: a tray-v1.8.9-rc.2 release will find
 * tray-v1.8.9-rc.1 as its previous tag, and a tray-v1.8.10-alpha.1
 * release will find tray-v1.8.9 (the previous live).
 *
 * Returns the full tag string (with track prefix), or `null` if the
 * track has no prior tag at all (first-ever release on the track).
 */
export function previousTrackTagBelow(tagList, track, currentVersion) {
  if (!TRACKS.includes(track)) {
    throw new Error(
      `previousTrackTagBelow: unknown track ${JSON.stringify(track)}; expected one of ${TRACKS.join(", ")}`,
    );
  }
  const current = parseVersion(currentVersion);
  const candidates = [];
  for (const raw of tagList) {
    const meta = parseTrackTag(raw);
    if (meta === null) continue;
    if (meta.track !== track) continue;
    let parsed;
    try {
      parsed = parseVersion(meta.version);
    } catch {
      continue;
    }
    if (compareVersions(parsed, current) >= 0) continue;
    candidates.push({ raw, parsed });
  }
  if (candidates.length === 0) return null;
  candidates.sort((a, b) => compareVersions(b.parsed, a.parsed));
  return candidates[0].raw;
}

/**
 * Extract unique `roadmap/<slug>` label suffixes from a list of PR
 * objects (the shape returned by `gh pr list --json labels`). Tolerates
 * both the object form (`{name: "roadmap/foo"}`) and the string form
 * (`"roadmap/foo"`) because the CLI's JSON shape has varied across
 * gh versions.
 *
 * Pure — no IO.
 *
 * Used by the slug auto-discovery to turn a list of "PRs merged in
 * this release window" into "the unique set of roadmap items this
 * release ships work on."
 */
export function parseSlugsFromPrLabels(prs) {
  const slugs = new Set();
  for (const pr of prs || []) {
    const labels = pr?.labels;
    if (!labels) continue;
    for (const label of labels) {
      const name = typeof label === "string" ? label : label?.name;
      if (typeof name !== "string") continue;
      if (!name.startsWith("roadmap/")) continue;
      const slug = name.slice("roadmap/".length).trim();
      if (slug.length === 0) continue;
      slugs.add(slug);
    }
  }
  return [...slugs];
}

/**
 * Discover the roadmap slug(s) for this release by scanning PRs merged
 * between the previous track tag and `targetSha` for `roadmap/<slug>`
 * labels (planted by the `pr-roadmap-link` skill at PR-create time).
 *
 * Returns:
 *   - `null` if there's no previous tag (first-ever release on the track),
 *     or if `gh` is unavailable / failed.
 *   - `{ slugs: string[], prevTag: string, prCount: number }` otherwise.
 *
 * Uses `gh pr list` filtered by `merged:>{prevTagDate}` then narrows
 * by ancestry to PRs whose merge commit is reachable from targetSha
 * but NOT from prevTag. Network-failure tolerant: any error during
 * discovery returns `null`, and the caller treats `null` the same as
 * "no slug to annotate" — so a flaky gh request degrades the release
 * UX (no auto-annotation) but never blocks the release itself.
 */
function discoverSlugFromMergedPrs(runner, track, currentVersion, targetSha) {
  // 1. Find the previous track tag (below currentVersion, same track).
  let tagsRaw;
  try {
    tagsRaw = runner.gitRead(["tag", "-l", "--sort=-version:refname"]);
  } catch {
    return null;
  }
  const tags = tagsRaw.split(/\r?\n/).filter((t) => t.length > 0);
  const prevTag = previousTrackTagBelow(tags, track, currentVersion);
  if (!prevTag) return null;

  // 2. Use the prev tag's commit date as a coarse merged:>{date} filter
  //    for gh, then narrow precisely by git ancestry below.
  let prevDate;
  try {
    prevDate = runner.gitRead(["log", "-1", "--format=%cI", prevTag]);
  } catch {
    return null;
  }

  // 3. Fetch merged PRs since the prev tag's date.
  let prsJson;
  try {
    prsJson = execFileSync(
      "gh",
      [
        "pr",
        "list",
        "--state",
        "merged",
        "--base",
        "next",
        "--search",
        `merged:>${prevDate}`,
        "--limit",
        "100",
        "--json",
        "number,mergeCommit,labels,title",
      ],
      { cwd: repoRoot(), encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
  } catch (e) {
    console.warn(
      `[auto-slug] gh pr list failed (${e.message?.split("\n")[0]}); skipping discovery`,
    );
    return null;
  }

  let prs;
  try {
    prs = JSON.parse(prsJson);
  } catch {
    return null;
  }

  // 4. Narrow by ancestry: keep PRs whose merge commit is reachable
  //    from targetSha AND NOT reachable from prevTag. That's the
  //    exact "what merged in this release window" set.
  const inRange = prs.filter((pr) => {
    const sha = pr?.mergeCommit?.oid;
    if (!sha) return false;
    return isAncestor(runner, sha, targetSha) && !isAncestor(runner, sha, prevTag);
  });

  const slugs = parseSlugsFromPrLabels(inRange);
  return { slugs, prevTag, prCount: inRange.length };
}

/**
 * Resolve the roadmap slug for an outgoing tag annotation.
 *
 * Priority order:
 *   1. Explicit `--roadmap-item-slug <slug>` flag → wins, no discovery.
 *   2. `--no-auto-slug` flag → skip discovery; emit unsigned tag.
 *   3. Auto-discover from merged-PR `roadmap/*` labels:
 *      - 0 slugs found → null (no annotation, same as today's behaviour).
 *      - 1 slug → use it; log the source so the operator knows.
 *      - >1 slugs → fail loud, demand explicit `--roadmap-item-slug`.
 *
 * Returns the resolved slug string, or null if no annotation should
 * be added.
 */
function resolveSlugForTag({ runner, track, flags, nextVersion, targetSha }) {
  const explicit = flags["roadmap-item-slug"];
  if (explicit && String(explicit).trim() !== "") {
    const slug = String(explicit).trim();
    console.log(`[${track}] slug: ${slug} (explicit --roadmap-item-slug)`);
    return slug;
  }
  if (flags["no-auto-slug"]) {
    console.log(`[${track}] slug: (skipped via --no-auto-slug)`);
    return null;
  }
  const discovered = discoverSlugFromMergedPrs(runner, track, nextVersion, targetSha);
  if (!discovered) {
    console.log(`[${track}] slug: (none) — no prior tag or gh unavailable`);
    return null;
  }
  if (discovered.slugs.length === 0) {
    console.log(
      `[${track}] slug: (none) — ${discovered.prCount} PR(s) since ${discovered.prevTag}, none with roadmap/* labels`,
    );
    return null;
  }
  if (discovered.slugs.length === 1) {
    const slug = discovered.slugs[0];
    console.log(
      `[${track}] slug: ${slug} (auto-discovered from PRs since ${discovered.prevTag})`,
    );
    return slug;
  }
  console.error(
    `[${track}] error: multiple roadmap slugs in PRs since ${discovered.prevTag}: ${discovered.slugs.join(", ")}\n` +
      `        Re-run with --roadmap-item-slug <slug> to pick one, or --no-auto-slug to skip the annotation entirely.`,
  );
  process.exit(1);
}

/**
 * Compute the next version given the current version, the subcommand,
 * and options. Pure — no side effects.
 *
 * Track-agnostic: the arithmetic is the same for both tray and
 * platform. The track determines WHICH version string is fed in
 * (read from tray-prefixed or bare-prefixed tags upstream).
 */
export function computeNextVersion(current, command, opts = {}) {
  const cur = parseVersion(current);

  if (command === "prerelease") {
    const channel = opts.channel;
    if (!CHANNELS.includes(channel)) {
      throw new Error(
        `unknown channel ${JSON.stringify(channel)}; expected one of ${CHANNELS.join(", ")}`,
      );
    }
    let { major, minor, patch } = cur;
    if (cur.channel === null) {
      // Live → first prerelease bumps patch.
      patch += 1;
    }
    let nextN;
    if (cur.channel === channel) {
      nextN = cur.n + 1;
    } else if (opts.existingTags) {
      // Channel switch within the same X.Y.Z. Look up any existing
      // tags for the target channel in this cycle — without this,
      // computing "next alpha" after a beta was cut naively returns
      // alpha.1 and collides with the alpha.1 that was already shipped
      // earlier in the cycle. The 2026-05-24 platform-track regression
      // pinned by `regression: ...` test below.
      const xyz = `${major}.${minor}.${patch}`;
      const maxN = highestNForCycleChannel(
        opts.existingTags,
        opts.track,
        xyz,
        channel,
      );
      nextN = maxN + 1;
    } else {
      // Legacy callers without existingTags get the old behaviour —
      // safe when the caller has separately ensured the cycle is clean
      // for the target channel.
      nextN = 1;
    }
    if (opts.n !== undefined && opts.n !== null) {
      if (!Number.isInteger(opts.n) || opts.n < 1) {
        throw new Error(`--n must be a positive integer, got ${opts.n}`);
      }
      if (cur.channel === channel && opts.n <= cur.n) {
        throw new Error(
          `--n ${opts.n} would not advance (current is ${channel}.${cur.n}); refusing to go backward`,
        );
      }
      nextN = opts.n;
    }
    return `${major}.${minor}.${patch}-${channel}.${nextN}`;
  }

  if (command === "live") {
    if (cur.channel === null) {
      return `${cur.major}.${cur.minor}.${cur.patch + 1}`;
    }
    return `${cur.major}.${cur.minor}.${cur.patch}`;
  }

  throw new Error(`unknown command: ${command}`);
}

/**
 * Replace the `version = "..."` line inside `[workspace.package]` only.
 * Used by the `platform` track — this is the workspace version that
 * starstats-core + starstats-server inherit.
 */
export function bumpCargoToml(content, newVersion) {
  const headerRe = /^\[workspace\.package\]\s*$/m;
  const headerMatch = headerRe.exec(content);
  if (!headerMatch) {
    throw new Error("[workspace.package] section not found in Cargo.toml");
  }
  const sectionStart = headerMatch.index + headerMatch[0].length;
  const afterHeader = content.slice(sectionStart);
  const nextSectionRe = /\n\[/;
  const nextMatch = nextSectionRe.exec(afterHeader);
  const sectionEnd =
    nextMatch === null ? content.length : sectionStart + nextMatch.index;

  const section = content.slice(sectionStart, sectionEnd);
  const versionLineRe = /^(\s*version\s*=\s*")[^"]*(")/m;
  if (!versionLineRe.test(section)) {
    throw new Error(
      "version key not found inside [workspace.package] section",
    );
  }
  const newSection = section.replace(
    versionLineRe,
    (_, pre, post) => `${pre}${newVersion}${post}`,
  );
  return content.slice(0, sectionStart) + newSection + content.slice(sectionEnd);
}

/**
 * Replace the `version = "..."` line inside `[package]` only. Used by
 * the `tray` track — this is crates/starstats-client/Cargo.toml, which
 * since the track split holds a literal version field (no longer
 * `version.workspace = true`).
 *
 * Refuses if the file still has `version.workspace = true` — guards
 * against the file getting reverted to workspace inheritance, which
 * would silently shift the tray onto the platform's version cycle.
 */
export function bumpClientCargo(content, newVersion) {
  const headerRe = /^\[package\]\s*$/m;
  const headerMatch = headerRe.exec(content);
  if (!headerMatch) {
    throw new Error("[package] section not found in client Cargo.toml");
  }
  const sectionStart = headerMatch.index + headerMatch[0].length;
  const afterHeader = content.slice(sectionStart);
  const nextSectionRe = /\n\[/;
  const nextMatch = nextSectionRe.exec(afterHeader);
  const sectionEnd =
    nextMatch === null ? content.length : sectionStart + nextMatch.index;

  const section = content.slice(sectionStart, sectionEnd);
  if (/^\s*version\.workspace\s*=\s*true/m.test(section)) {
    throw new Error(
      "client Cargo.toml still inherits version from workspace; flip to a literal `version = \"x.y.z\"` line first",
    );
  }
  const versionLineRe = /^(\s*version\s*=\s*")[^"]*(")/m;
  if (!versionLineRe.test(section)) {
    throw new Error(
      "version key not found inside [package] section of client Cargo.toml",
    );
  }
  const newSection = section.replace(
    versionLineRe,
    (_, pre, post) => `${pre}${newVersion}${post}`,
  );
  return content.slice(0, sectionStart) + newSection + content.slice(sectionEnd);
}

/**
 * Update the top-level `version` field in tauri.conf.json, preserving
 * 2-space indent + trailing newline. Tray-only.
 */
export function bumpTauriConf(content, newVersion) {
  let parsed;
  try {
    parsed = JSON.parse(content);
  } catch (e) {
    throw new Error(`tauri.conf.json is not valid JSON: ${e.message}`);
  }
  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("tauri.conf.json must be a JSON object");
  }
  if (!Object.prototype.hasOwnProperty.call(parsed, "version")) {
    throw new Error("version key missing from tauri.conf.json");
  }
  parsed.version = newVersion;
  return JSON.stringify(parsed, null, 2) + "\n";
}

// ---------------------------------------------------------------------------
// CLI — only runs when invoked directly
// ---------------------------------------------------------------------------

function isMain() {
  if (typeof process.argv[1] !== "string") return false;
  try {
    return (
      path.resolve(process.argv[1]) ===
      path.resolve(fileURLToPath(import.meta.url))
    );
  } catch {
    return false;
  }
}

function repoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
}

function readWorkspaceCargo() {
  return readFileSync(path.join(repoRoot(), "Cargo.toml"), "utf8");
}

function readClientCargo() {
  return readFileSync(
    path.join(repoRoot(), "crates", "starstats-client", "Cargo.toml"),
    "utf8",
  );
}

function readTauriConf() {
  return readFileSync(
    path.join(repoRoot(), "crates", "starstats-client", "tauri.conf.json"),
    "utf8",
  );
}

function currentWorkspaceCargoVersion() {
  const content = readWorkspaceCargo();
  const headerRe = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s;
  const m = headerRe.exec(content);
  if (!m) {
    throw new Error("could not find workspace.package version in Cargo.toml");
  }
  return m[1];
}

function currentClientCargoVersion() {
  const content = readClientCargo();
  const headerRe = /\[package\][^[]*?version\s*=\s*"([^"]+)"/s;
  const m = headerRe.exec(content);
  if (!m) {
    throw new Error(
      "could not find [package].version in crates/starstats-client/Cargo.toml (is it still on version.workspace = true?)",
    );
  }
  return m[1];
}

function currentCargoVersionForTrack(track) {
  if (track === "platform") return currentWorkspaceCargoVersion();
  if (track === "tray") return currentClientCargoVersion();
  throw new Error(`unknown track: ${JSON.stringify(track)}`);
}

/**
 * The version we should treat as "where we are in the release cycle"
 * for the given track. Prefers the highest semver git tag for that
 * track; falls back to the relevant Cargo.toml if no tags exist yet.
 *
 * Lists ALL tags (no -l pattern) and filters in JS via parseTrackTag —
 * `git tag -l` doesn't accept OR'd patterns, and a single glob like
 * `*v*` would also match unrelated tags. Filtering in JS gives us
 * exact, track-aware membership.
 */
function currentVersionForPromote(runner, track) {
  const tagsRaw = runner.gitRead(["tag", "-l", "--sort=-version:refname"]);
  const fromTags = latestSemverFromTags(tagsRaw.split("\n"), track);
  return fromTags ?? currentCargoVersionForTrack(track);
}

function parseFlags(argv) {
  const positional = [];
  const flags = Object.create(null);
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dry-run") {
      flags["dry-run"] = true;
    } else if (a === "--help" || a === "-h") {
      flags.help = true;
    } else if (a.startsWith("--")) {
      const eq = a.indexOf("=");
      if (eq !== -1) {
        flags[a.slice(2, eq)] = a.slice(eq + 1);
      } else {
        const next = argv[i + 1];
        if (next === undefined || next.startsWith("--")) {
          flags[a.slice(2)] = true;
        } else {
          flags[a.slice(2)] = next;
          i++;
        }
      }
    } else {
      positional.push(a);
    }
  }
  return { positional, flags };
}

function usage() {
  return `Usage:
  release-promote.mjs prerelease <tray|platform> <alpha|beta|rc> [--sha <sha>] [--n <num>] [--roadmap-item-slug <slug>] [--no-auto-slug] [--dry-run]
  release-promote.mjs live       <tray|platform> [--sha <sha>] [--roadmap-item-slug <slug>] [--no-auto-slug] [--dry-run]

Roadmap slug resolution (annotated on the outgoing tag):
  1. --roadmap-item-slug <slug>    — explicit, wins outright.
  2. --no-auto-slug                — skip discovery; tag is unannotated.
  3. (default) auto-discover from "roadmap/<slug>" labels on PRs
     merged between the previous track tag and the target SHA. If
     exactly one slug → used. If multiple → script refuses; pick
     one via --roadmap-item-slug. If zero → no annotation.
  release-promote.mjs hotfix-finish [--dry-run]
  release-promote.mjs --help

See:
  the release design notes
  the release design notes
`;
}

// --- git helpers (execFileSync — no shell, no injection vector) ---

class Runner {
  constructor({ dryRun }) {
    this.dryRun = !!dryRun;
  }

  /** Read-only git query — always runs, even in dry-run. */
  gitRead(args) {
    return execFileSync("git", args, {
      cwd: repoRoot(),
      stdio: ["ignore", "pipe", "pipe"],
      encoding: "utf8",
    }).trim();
  }

  /** Destructive command — only echoes in dry-run. */
  run(cmd, args, { critical = true } = {}) {
    const line = `${cmd} ${args.join(" ")}`;
    if (this.dryRun) {
      console.log(`[dry-run] ${line}`);
      return "";
    }
    console.log(`$ ${line}`);
    try {
      return execFileSync(cmd, args, {
        cwd: repoRoot(),
        stdio: ["ignore", "inherit", "inherit"],
      });
    } catch (err) {
      if (critical) throw err;
      console.warn(`(warn) ${line} failed: ${err.message}`);
      return "";
    }
  }

  /**
   * Non-fatal command that reports whether it worked. Returns true on
   * success, false on failure (warning to stderr either way).
   *
   * `run(..., { critical: false })` cannot express this: it swallows the
   * error and returns "" — indistinguishable from a successful command that
   * printed nothing. Callers that need to branch on failure (e.g. walking a
   * fallback ladder) must use this instead. In dry-run nothing executes, so
   * this reports true.
   */
  tryRun(cmd, args) {
    try {
      this.run(cmd, args, { critical: true });
      return true;
    } catch (err) {
      console.warn(`(warn) ${cmd} ${args.join(" ")} failed: ${err.message}`);
      return false;
    }
  }

  writeFile(targetPath, content) {
    if (this.dryRun) {
      console.log(`[dry-run] write ${path.relative(repoRoot(), targetPath)}`);
      return;
    }
    writeFileSync(targetPath, content);
  }
}

function isAncestor(_runner, ancestor, descendant) {
  try {
    execFileSync(
      "git",
      ["merge-base", "--is-ancestor", ancestor, descendant],
      { cwd: repoRoot(), stdio: "ignore" },
    );
    return true;
  } catch {
    return false;
  }
}

function resolveSha(runner, ref) {
  return runner.gitRead(["rev-parse", "--verify", ref]);
}

/**
 * Cargo.lock refresh attempts for the platform track, tried in order until
 * one succeeds. Exported so the unit test can assert the ladder's shape
 * without shelling out to cargo.
 *
 * Ordering rationale:
 *   1. `--workspace --offline` — the common case; no network, fast.
 *   2. per-package `--offline`  — narrower resolution; survives some
 *      workspace-wide resolution failures.
 *   3. `--workspace` (ONLINE)   — last resort. Attempts 1+2 both fail on a
 *      cold registry cache, which is precisely the failure that stranded
 *      Cargo.lock at 1.8.67 while Cargo.toml said 1.8.68 (see the v1.8.68
 *      auto-alpha run: "(warn) cargo update --workspace --offline failed"
 *      immediately followed by the commit). An offline-only ladder cannot
 *      recover from that — every rung has the same dependency.
 */
export const CARGO_LOCK_REFRESH_ATTEMPTS = [
  ["update", "--workspace", "--offline"],
  ["update", "-p", "starstats-core", "-p", "starstats-server", "--offline"],
  ["update", "--workspace"],
];

export function bumpVersionFiles(runner, track, newVersion) {
  if (track === "platform") {
    const cargoPath = path.join(repoRoot(), "Cargo.toml");
    runner.writeFile(cargoPath, bumpCargoToml(readWorkspaceCargo(), newVersion));

    // Refresh Cargo.lock's record of the workspace member versions.
    //
    // This used to be a try/catch around a `critical: false` call, which is
    // dead code: run() only rethrows when `critical` is true, so the catch
    // could never fire and the fallback never ran. Walk the ladder
    // explicitly instead, treating each rung as non-fatal and moving on.
    let refreshed = false;
    for (const args of CARGO_LOCK_REFRESH_ATTEMPTS) {
      if (runner.tryRun("cargo", args)) {
        refreshed = true;
        break;
      }
    }

    // Never fail silently. A stale lock is not release-blocking (nothing
    // builds with --locked, so cargo re-resolves at build time and the drift
    // self-heals), but it lands a wrong Cargo.lock in git and dirties the
    // tree on the next local build — so it must be visible, not swallowed.
    if (!refreshed) {
      console.log(
        `::warning::Cargo.lock refresh failed for ${newVersion} — every cargo update attempt failed. ` +
          `Cargo.toml is bumped but Cargo.lock still records the previous version. ` +
          `Not release-blocking (no build uses --locked), but commit a re-synced lock.`,
      );
    }
    return;
  }
  if (track === "tray") {
    const clientCargoPath = path.join(
      repoRoot(),
      "crates",
      "starstats-client",
      "Cargo.toml",
    );
    const tauriPath = path.join(
      repoRoot(),
      "crates",
      "starstats-client",
      "tauri.conf.json",
    );
    runner.writeFile(clientCargoPath, bumpClientCargo(readClientCargo(), newVersion));
    runner.writeFile(tauriPath, bumpTauriConf(readTauriConf(), newVersion));
    runner.run(
      "cargo",
      ["update", "-p", "starstats-client", "--offline"],
      { critical: false },
    );
    return;
  }
  throw new Error(`unknown track: ${JSON.stringify(track)}`);
}

function bumpedPaths(track) {
  if (track === "platform") return ["Cargo.toml", "Cargo.lock"];
  if (track === "tray") {
    return [
      "crates/starstats-client/Cargo.toml",
      "crates/starstats-client/tauri.conf.json",
      "Cargo.lock",
    ];
  }
  throw new Error(`unknown track: ${JSON.stringify(track)}`);
}

function validateTrack(arg) {
  if (!TRACKS.includes(arg)) {
    console.error(
      `error: track must be one of ${TRACKS.join(", ")}, got ${JSON.stringify(arg)}`,
    );
    console.error(usage());
    process.exit(2);
  }
}

function cmdPrerelease(args) {
  const { positional, flags } = parseFlags(args);
  const track = positional[0];
  validateTrack(track);
  const channel = positional[1];
  if (!CHANNELS.includes(channel)) {
    console.error(`error: prerelease requires channel (one of ${CHANNELS.join(", ")})`);
    console.error(usage());
    process.exit(2);
  }
  const runner = new Runner({ dryRun: !!flags["dry-run"] });

  runner.run("git", ["fetch", "--tags", "origin", "next"]);

  const targetSha = flags.sha
    ? resolveSha(runner, String(flags.sha))
    : resolveSha(runner, "origin/next");

  if (!isAncestor(runner, targetSha, "origin/next")) {
    console.error(
      `error: target SHA ${targetSha.slice(0, 12)} is not reachable from origin/next`,
    );
    process.exit(1);
  }
  if (isAncestor(runner, targetSha, "origin/main")) {
    console.log(
      `[no-op] target SHA ${targetSha.slice(0, 12)} is already on origin/main; nothing to promote (next has not advanced beyond main)`,
    );
    process.exit(0);
  }

  const current = currentVersionForPromote(runner, track);
  // Pass the full tag list + track so computeNextVersion can advance
  // N past any existing tags for the target channel in this cycle
  // (avoids the `current: beta.2 → next: alpha.1 (already exists)` bug
  // when the cycle previously had alphas before the channel switch).
  const tagsRaw = runner.gitRead(["tag", "-l", "--sort=-version:refname"]);
  const existingTags = tagsRaw.split("\n");
  const next = computeNextVersion(current, "prerelease", {
    channel,
    n: flags.n !== undefined ? Number(flags.n) : undefined,
    existingTags,
    track,
  });
  const bareNext = bareSemver(next);
  console.log(`[${track}] current: ${current}  →  next: ${next} (cargo: ${bareNext})`);
  const tag = `${tagPrefix(track)}${next}`;

  runner.run("git", ["checkout", "next"]);
  runner.run("git", ["pull", "--ff-only"]);
  if (flags.sha) {
    const head = runner.gitRead(["rev-parse", "HEAD"]);
    if (head !== targetSha) {
      console.error(
        "error: promotion from a non-HEAD SHA on next requires manual branch positioning",
      );
      process.exit(1);
    }
  }

  const cargoNow = currentCargoVersionForTrack(track);
  const needsBump = cargoNow !== bareNext;
  if (needsBump) {
    bumpVersionFiles(runner, track, bareNext);
    runner.run("git", ["add", ...bumpedPaths(track)]);
    runner.run("git", ["commit", "-m", `chore: bump ${track} to ${tag}`]);
  } else {
    console.log(`[${track}] cargo already at bare ${bareNext}; tagging HEAD without commit`);
  }
  // Annotate the tag with `Roadmap-Item: <slug>` when one resolves;
  // the release.yml roadmap-emit-event job reads this annotation so
  // per-release item attribution travels with the tag itself (no
  // race-prone repo-variable mutation between dispatch and tag push).
  // Slug resolution: explicit --roadmap-item-slug → auto-discover
  // from merged-PR `roadmap/*` labels → none. See resolveSlugForTag.
  const prereleaseSlug = resolveSlugForTag({
    runner,
    track,
    flags,
    nextVersion: next,
    targetSha,
  });
  const prereleaseTagArgs = ["tag"];
  if (prereleaseSlug) {
    const msg = `Release ${tag}\n\nRoadmap-Item: ${prereleaseSlug}\n`;
    prereleaseTagArgs.push("-a", tag, "-m", msg);
  } else {
    prereleaseTagArgs.push(tag);
  }
  runner.run("git", prereleaseTagArgs);
  if (needsBump) {
    runner.run("git", ["push", "origin", "next"]);
  }
  runner.run("git", ["push", "origin", tag]);

  console.log(`done: ${tag} pushed on next`);
}

function cmdLive(args) {
  const { positional, flags } = parseFlags(args);
  const track = positional[0];
  validateTrack(track);
  const runner = new Runner({ dryRun: !!flags["dry-run"] });

  runner.run("git", ["fetch", "--tags", "origin", "main", "next"]);

  const targetSha = flags.sha
    ? resolveSha(runner, String(flags.sha))
    : resolveSha(runner, "origin/next");

  if (!isAncestor(runner, targetSha, "origin/next")) {
    console.error(
      `error: target SHA ${targetSha.slice(0, 12)} is not reachable from origin/next`,
    );
    process.exit(1);
  }
  if (!isAncestor(runner, "origin/main", targetSha)) {
    console.error(
      `error: origin/main is not an ancestor of ${targetSha.slice(0, 12)}; fast-forward not possible`,
    );
    process.exit(1);
  }

  const current = currentVersionForPromote(runner, track);
  const next = computeNextVersion(current, "live", {});
  const bareNext = bareSemver(next);
  console.log(`[${track}] current: ${current}  →  next: ${next} (cargo: ${bareNext})`);
  const tag = `${tagPrefix(track)}${next}`;

  runner.run("git", ["checkout", "main"]);
  runner.run("git", ["pull", "--ff-only"]);
  runner.run("git", ["merge", "--ff-only", targetSha]);

  const cargoNow = currentCargoVersionForTrack(track);
  const needsBump = cargoNow !== bareNext;
  if (needsBump) {
    bumpVersionFiles(runner, track, bareNext);
    runner.run("git", ["add", ...bumpedPaths(track)]);
    runner.run("git", ["commit", "-m", `chore: bump ${track} to ${tag}`]);
  } else {
    console.log(`[${track}] cargo already at bare ${bareNext}; tagging HEAD without commit`);
  }
  // Annotate the tag with `Roadmap-Item: <slug>` — same resolution
  // pattern as the prerelease path above (explicit flag wins,
  // otherwise auto-discover from merged-PR `roadmap/*` labels).
  const liveSlug = resolveSlugForTag({
    runner,
    track,
    flags,
    nextVersion: next,
    targetSha,
  });
  const liveTagArgs = ["tag"];
  if (liveSlug) {
    const msg = `Release ${tag}\n\nRoadmap-Item: ${liveSlug}\n`;
    liveTagArgs.push("-a", tag, "-m", msg);
  } else {
    liveTagArgs.push(tag);
  }
  runner.run("git", liveTagArgs);
  runner.run("git", ["push", "origin", "main"]);
  runner.run("git", ["push", "origin", tag]);

  console.log(`done: ${tag} pushed on main`);
}

function cmdHotfixFinish(args) {
  const { flags } = parseFlags(args);
  const runner = new Runner({ dryRun: !!flags["dry-run"] });

  runner.run("git", ["fetch", "origin", "main", "next"]);
  runner.run("git", ["checkout", "next"]);
  runner.run("git", ["pull", "--ff-only"]);
  runner.run("git", [
    "merge",
    "origin/main",
    "--no-ff",
    "-m",
    "merge main into next after hotfix",
  ]);
  runner.run("git", ["push", "origin", "next"]);

  console.log("done: main merged into next; invariant restored");
}

function main(argv) {
  const sub = argv[0];
  const rest = argv.slice(1);

  if (!sub || sub === "--help" || sub === "-h") {
    process.stdout.write(usage());
    process.exit(sub ? 0 : 2);
  }

  switch (sub) {
    case "prerelease":
      return cmdPrerelease(rest);
    case "live":
      return cmdLive(rest);
    case "hotfix-finish":
      return cmdHotfixFinish(rest);
    default:
      console.error(`unknown subcommand: ${sub}`);
      console.error(usage());
      process.exit(2);
  }
}

if (isMain()) {
  try {
    main(process.argv.slice(2));
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exit(1);
  }
}
