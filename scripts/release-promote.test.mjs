// Unit tests for release-promote.mjs pure functions.
// Run: node --test scripts/release-promote.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  computeNextVersion,
  detectChannel,
  bumpCargoToml,
  bumpClientCargo,
  bumpTauriConf,
  bareSemver,
  latestSemverFromTags,
  highestNForCycleChannel,
  tagPrefix,
  parseTrackTag,
  parseSlugsFromPrLabels,
  previousTrackTagBelow,
  bumpVersionFiles,
  CARGO_LOCK_REFRESH_ATTEMPTS,
} from "./release-promote.mjs";

// ---------------------------------------------------------------------------
// computeNextVersion
// ---------------------------------------------------------------------------

test("prerelease alpha from 1.8.0 → 1.8.1-alpha.1", () => {
  assert.equal(
    computeNextVersion("1.8.0", "prerelease", { channel: "alpha" }),
    "1.8.1-alpha.1",
  );
});

test("prerelease alpha from 1.8.1-alpha.1 → 1.8.1-alpha.2", () => {
  assert.equal(
    computeNextVersion("1.8.1-alpha.1", "prerelease", { channel: "alpha" }),
    "1.8.1-alpha.2",
  );
});

test("prerelease beta from 1.8.1-alpha.2 → 1.8.1-beta.1", () => {
  assert.equal(
    computeNextVersion("1.8.1-alpha.2", "prerelease", { channel: "beta" }),
    "1.8.1-beta.1",
  );
});

test("prerelease rc from 1.8.1-beta.3 → 1.8.1-rc.1", () => {
  assert.equal(
    computeNextVersion("1.8.1-beta.3", "prerelease", { channel: "rc" }),
    "1.8.1-rc.1",
  );
});

test("live from 1.8.1-rc.2 → 1.8.1", () => {
  assert.equal(
    computeNextVersion("1.8.1-rc.2", "live", {}),
    "1.8.1",
  );
});

test("live from 1.8.0 (no prior prerelease) → 1.8.1", () => {
  assert.equal(
    computeNextVersion("1.8.0", "live", {}),
    "1.8.1",
  );
});

test("prerelease beta --n 3 from 1.8.0 → 1.8.1-beta.3", () => {
  assert.equal(
    computeNextVersion("1.8.0", "prerelease", { channel: "beta", n: 3 }),
    "1.8.1-beta.3",
  );
});

test("prerelease beta --n 2 from 1.8.1-beta.5 → rejected (backward)", () => {
  assert.throws(
    () =>
      computeNextVersion("1.8.1-beta.5", "prerelease", {
        channel: "beta",
        n: 2,
      }),
    /backward|decrease|already/i,
  );
});

test("computeNextVersion rejects unknown channel", () => {
  assert.throws(
    () =>
      computeNextVersion("1.8.0", "prerelease", { channel: "stable" }),
    /channel/i,
  );
});

test("computeNextVersion rejects malformed current version", () => {
  assert.throws(
    () => computeNextVersion("not-a-version", "prerelease", { channel: "beta" }),
    /version/i,
  );
});

// Cross-channel sequencing on same X.Y.Z
test("prerelease beta from 1.8.1-beta.1 → 1.8.1-beta.2", () => {
  assert.equal(
    computeNextVersion("1.8.1-beta.1", "prerelease", { channel: "beta" }),
    "1.8.1-beta.2",
  );
});

test("prerelease rc from 1.8.1-rc.7 → 1.8.1-rc.8", () => {
  assert.equal(
    computeNextVersion("1.8.1-rc.7", "prerelease", { channel: "rc" }),
    "1.8.1-rc.8",
  );
});

// ---------------------------------------------------------------------------
// detectChannel
// ---------------------------------------------------------------------------

test("detectChannel: 1.8.1-alpha.2 → alpha", () => {
  assert.equal(detectChannel("1.8.1-alpha.2"), "alpha");
});

test("detectChannel: 1.8.1-beta.1 → beta", () => {
  assert.equal(detectChannel("1.8.1-beta.1"), "beta");
});

test("detectChannel: 1.8.1-rc.1 → rc", () => {
  assert.equal(detectChannel("1.8.1-rc.1"), "rc");
});

test("detectChannel: 1.8.1 → live", () => {
  assert.equal(detectChannel("1.8.1"), "live");
});

test("detectChannel: 1.8.0 → live", () => {
  assert.equal(detectChannel("1.8.0"), "live");
});

test("detectChannel rejects garbage", () => {
  assert.throws(() => detectChannel("not-a-version"), /version/i);
  assert.throws(() => detectChannel(""), /version/i);
  assert.throws(() => detectChannel("1.8"), /version/i);
  assert.throws(() => detectChannel("1.8.0-foo.1"), /channel|version/i);
});

// ---------------------------------------------------------------------------
// bumpCargoToml
// ---------------------------------------------------------------------------

test("bumpCargoToml replaces only the workspace.package version", () => {
  const input = `[workspace]
resolver = "2"
members = ["crates/starstats-core"]

[workspace.package]
version = "1.8.0"
edition = "2021"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.40" }
`;
  const out = bumpCargoToml(input, "1.8.1-beta.1");
  assert.match(out, /\[workspace\.package\]\nversion = "1\.8\.1-beta\.1"/);
  // serde / tokio version unchanged
  assert.match(out, /serde = \{ version = "1\.0"/);
  assert.match(out, /tokio = \{ version = "1\.40" \}/);
  // Old version no longer appears in workspace.package block
  assert.doesNotMatch(
    out,
    /\[workspace\.package\][\s\S]*version = "1\.8\.0"/,
  );
});

test("bumpCargoToml preserves surrounding whitespace and ordering", () => {
  const input = `[workspace.package]
version = "1.8.0"
edition = "2021"
`;
  const out = bumpCargoToml(input, "1.9.0");
  assert.equal(
    out,
    `[workspace.package]
version = "1.9.0"
edition = "2021"
`,
  );
});

test("bumpCargoToml is idempotent when version already matches", () => {
  const input = `[workspace.package]
version = "1.8.0"
edition = "2021"
`;
  assert.equal(bumpCargoToml(input, "1.8.0"), input);
});

test("bumpCargoToml throws if [workspace.package] section is missing", () => {
  const input = `[workspace]\nmembers = []\n`;
  assert.throws(() => bumpCargoToml(input, "1.9.0"), /workspace\.package/);
});

// ---------------------------------------------------------------------------
// bumpTauriConf
// ---------------------------------------------------------------------------

test("bumpTauriConf updates only version and preserves formatting", () => {
  const input = `{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "StarStats",
  "version": "1.8.0",
  "identifier": "app.starstats.tray"
}
`;
  const out = bumpTauriConf(input, "1.8.1-beta.1");
  // Round-trips as JSON
  const parsed = JSON.parse(out);
  assert.equal(parsed.version, "1.8.1-beta.1");
  assert.equal(parsed.productName, "StarStats");
  assert.equal(parsed.identifier, "app.starstats.tray");
  assert.equal(parsed["$schema"], "https://schema.tauri.app/config/2");
  // 2-space indent
  assert.match(out, /^\{\n {2}"\$schema"/);
  // Trailing newline preserved
  assert.ok(out.endsWith("\n"));
});

test("bumpTauriConf is idempotent when version already matches", () => {
  const input = `{
  "version": "1.8.0",
  "productName": "StarStats"
}
`;
  const out = bumpTauriConf(input, "1.8.0");
  assert.equal(JSON.parse(out).version, "1.8.0");
});

test("bumpTauriConf throws if version key is missing", () => {
  const input = `{ "productName": "StarStats" }`;
  assert.throws(() => bumpTauriConf(input, "1.9.0"), /version/);
});

// ---------------------------------------------------------------------------
// bareSemver — Tauri MSI bundler can't handle pre-release suffixes
// ---------------------------------------------------------------------------

test("bareSemver: 1.8.0 stays bare", () => {
  assert.equal(bareSemver("1.8.0"), "1.8.0");
});

test("bareSemver: 1.8.2-alpha.1 → 1.8.2", () => {
  assert.equal(bareSemver("1.8.2-alpha.1"), "1.8.2");
});

test("bareSemver: 1.8.2-beta.5 → 1.8.2", () => {
  assert.equal(bareSemver("1.8.2-beta.5"), "1.8.2");
});

test("bareSemver: 1.8.2-rc.1 → 1.8.2", () => {
  assert.equal(bareSemver("1.8.2-rc.1"), "1.8.2");
});

// ---------------------------------------------------------------------------
// latestSemverFromTags — read "where in cycle" from git tags, not Cargo
// ---------------------------------------------------------------------------

test("latestSemverFromTags returns highest version (input order irrelevant)", () => {
  // Input is shuffled; latestSemverFromTags sorts internally.
  const tags = ["v1.8.0", "v1.8.2-alpha.1", "v1.8.1"];
  assert.equal(latestSemverFromTags(tags), "1.8.2-alpha.1");
});

test("latestSemverFromTags strips leading v", () => {
  assert.equal(latestSemverFromTags(["v1.5.0"]), "1.5.0");
});

test("latestSemverFromTags: release > pre-release of same X.Y.Z (semver precedence)", () => {
  // The exact bug today: git's --sort=version:refname puts v1.8.2-alpha.7
  // ABOVE v1.8.2 (longer string), so the old script tried to retag v1.8.2.
  // Semver §11: a pre-release version has lower precedence than the
  // matching release.
  const tags = ["v1.8.2-alpha.7", "v1.8.2", "v1.8.2-alpha.5", "v1.8.1"];
  assert.equal(latestSemverFromTags(tags), "1.8.2");
});

test("latestSemverFromTags: numeric N comparison (not lexical)", () => {
  // Git's lex sort would say alpha.2 > alpha.10 (because '2' > '1').
  // Semver: numeric identifiers compared as numbers.
  const tags = ["v1.8.2-alpha.10", "v1.8.2-alpha.2", "v1.8.2-alpha.9"];
  assert.equal(latestSemverFromTags(tags), "1.8.2-alpha.10");
});

test("latestSemverFromTags: channel ordering rc > beta > alpha", () => {
  const tags = ["v1.8.2-beta.5", "v1.8.2-alpha.7", "v1.8.2-rc.1"];
  assert.equal(latestSemverFromTags(tags), "1.8.2-rc.1");
});

test("latestSemverFromTags: across patch versions, highest patch wins", () => {
  const tags = ["v1.8.3-alpha.1", "v1.8.2", "v1.8.2-rc.5"];
  assert.equal(latestSemverFromTags(tags), "1.8.3-alpha.1");
});

test("latestSemverFromTags skips malformed entries", () => {
  const tags = ["not-a-tag", "v1.8.0-bogus-suffix-here", "v1.8.0"];
  assert.equal(latestSemverFromTags(tags), "1.8.0");
});

test("latestSemverFromTags returns null for empty list", () => {
  assert.equal(latestSemverFromTags([]), null);
});

test("latestSemverFromTags returns null when no recognised semvers", () => {
  assert.equal(latestSemverFromTags(["v0.0.0-dev", "junk"]), null);
});

test("latestSemverFromTags ignores blank lines (gh tag output)", () => {
  const tags = ["", "  ", "v1.8.0", ""];
  assert.equal(latestSemverFromTags(tags), "1.8.0");
});

// ---------------------------------------------------------------------------
// Integration: simulate what cmdPrerelease sees with latest-tag-from-state
// ---------------------------------------------------------------------------

test("after live 1.8.1, prerelease alpha computes 1.8.2-alpha.1 with cargo bare 1.8.2", () => {
  const current = latestSemverFromTags(["v1.8.1", "v1.8.0"]);
  const next = computeNextVersion(current, "prerelease", { channel: "alpha" });
  assert.equal(next, "1.8.2-alpha.1");
  assert.equal(bareSemver(next), "1.8.2");
});

test("after alpha.1, prerelease alpha computes 1.8.2-alpha.2 (cycle counting via tags)", () => {
  const current = latestSemverFromTags(["v1.8.2-alpha.1", "v1.8.1"]);
  const next = computeNextVersion(current, "prerelease", { channel: "alpha" });
  assert.equal(next, "1.8.2-alpha.2");
  assert.equal(bareSemver(next), "1.8.2");
});

test("after alpha.5, prerelease beta switches channel, resets N, keeps bare", () => {
  const current = latestSemverFromTags(["v1.8.2-alpha.5", "v1.8.1"]);
  const next = computeNextVersion(current, "prerelease", { channel: "beta" });
  assert.equal(next, "1.8.2-beta.1");
  assert.equal(bareSemver(next), "1.8.2");
});

test("after rc.2, live promotion keeps same X.Y.Z (no patch bump)", () => {
  const current = latestSemverFromTags(["v1.8.2-rc.2", "v1.8.2-beta.1"]);
  const next = computeNextVersion(current, "live", {});
  assert.equal(next, "1.8.2");
  assert.equal(bareSemver(next), "1.8.2");
});

// ---------------------------------------------------------------------------
// Track-aware tag plumbing (added with the release-tracks split,
// the release design notes)
// ---------------------------------------------------------------------------

test("tagPrefix: tray → 'tray-v'", () => {
  assert.equal(tagPrefix("tray"), "tray-v");
});

test("tagPrefix: platform → 'v'", () => {
  assert.equal(tagPrefix("platform"), "v");
});

test("tagPrefix rejects unknown track", () => {
  assert.throws(() => tagPrefix("server"), /track/i);
  assert.throws(() => tagPrefix(""), /track/i);
  assert.throws(() => tagPrefix(undefined), /track/i);
});

test("parseTrackTag: tray-v1.8.4 → tray track", () => {
  assert.deepEqual(parseTrackTag("tray-v1.8.4"), {
    track: "tray",
    version: "1.8.4",
  });
});

test("parseTrackTag: tray-v1.8.4-alpha.2 → tray track with prerelease", () => {
  assert.deepEqual(parseTrackTag("tray-v1.8.4-alpha.2"), {
    track: "tray",
    version: "1.8.4-alpha.2",
  });
});

test("parseTrackTag: v1.8.4 → platform track", () => {
  assert.deepEqual(parseTrackTag("v1.8.4"), {
    track: "platform",
    version: "1.8.4",
  });
});

test("parseTrackTag: v1.8.4-beta.1 → platform track with prerelease", () => {
  assert.deepEqual(parseTrackTag("v1.8.4-beta.1"), {
    track: "platform",
    version: "1.8.4-beta.1",
  });
});

test("parseTrackTag returns null for unknown shapes", () => {
  assert.equal(parseTrackTag("v1.8"), null);                // not full semver
  assert.equal(parseTrackTag("vfoo"), null);                // not semver
  assert.equal(parseTrackTag("tray-v"), null);              // empty version
  assert.equal(parseTrackTag("tray-vfoo"), null);           // not semver
  assert.equal(parseTrackTag("tray-1.8.4"), null);          // missing v
  assert.equal(parseTrackTag("1.8.4"), null);               // no prefix at all
  assert.equal(parseTrackTag("server-v1.8.4"), null);       // unknown track prefix
  assert.equal(parseTrackTag("v1.8.4-canary"), null);       // unknown channel
  assert.equal(parseTrackTag(""), null);
  assert.equal(parseTrackTag(null), null);
  assert.equal(parseTrackTag(undefined), null);
});

test("parseTrackTag strips surrounding whitespace (gh tag output)", () => {
  assert.deepEqual(parseTrackTag("  v1.8.4  "), {
    track: "platform",
    version: "1.8.4",
  });
  assert.deepEqual(parseTrackTag("\ttray-v1.8.4\n"), {
    track: "tray",
    version: "1.8.4",
  });
});

// latestSemverFromTags with track filter — the critical case the
// promote script depends on (next-version detection must read ONLY
// the right track's tags).

test("latestSemverFromTags filters to tray track only", () => {
  const mixed = [
    "v1.9.0",                  // platform, higher
    "tray-v1.8.5",             // tray, highest of its kind
    "tray-v1.8.4-rc.1",        // tray prerelease
    "v1.9.0-beta.2",           // platform prerelease
  ];
  assert.equal(latestSemverFromTags(mixed, "tray"), "1.8.5");
});

test("latestSemverFromTags filters to platform track only", () => {
  const mixed = [
    "v1.9.0",
    "tray-v1.8.5",
    "tray-v2.0.0",             // tray, would beat 1.9.0 if not filtered
    "v1.9.0-beta.2",
  ];
  assert.equal(latestSemverFromTags(mixed, "platform"), "1.9.0");
});

test("latestSemverFromTags without track honours both prefixes", () => {
  // Back-compat: when no track arg is given, both prefixes count.
  // Useful for diagnostics; the promote script always passes a track.
  const mixed = ["v1.9.0", "tray-v2.0.0"];
  assert.equal(latestSemverFromTags(mixed), "2.0.0");
});

test("latestSemverFromTags with track returns null when track has no tags", () => {
  const onlyPlatform = ["v1.9.0", "v1.8.0"];
  assert.equal(latestSemverFromTags(onlyPlatform, "tray"), null);
});

test("latestSemverFromTags with track rejects unknown track arg", () => {
  assert.throws(
    () => latestSemverFromTags(["v1.8.0"], "server"),
    /track/i,
  );
});

test("latestSemverFromTags with track: semver precedence still applies inside track", () => {
  // Critical: prereleases within a track must still sort below the
  // matching release (semver §11), and N must compare numerically.
  const tags = [
    "tray-v1.8.5-alpha.10",
    "tray-v1.8.5-alpha.2",
    "tray-v1.8.5",            // wins — release > prerelease at same X.Y.Z
    "tray-v1.8.5-rc.1",
    "v1.9.0",                  // ignored: wrong track
  ];
  assert.equal(latestSemverFromTags(tags, "tray"), "1.8.5");
});

// ---------------------------------------------------------------------------
// bumpClientCargo — track-aware bump for crates/starstats-client
// ---------------------------------------------------------------------------

test("bumpClientCargo replaces version inside [package] block", () => {
  const input = `[package]
name = "starstats-client"
version = "1.8.4"
edition.workspace = true
license.workspace = true

[dependencies]
serde = "1.0"
`;
  const out = bumpClientCargo(input, "1.8.5-alpha.1");
  assert.match(out, /\[package\]\nname = "starstats-client"\nversion = "1\.8\.5-alpha\.1"/);
  // [dependencies] block untouched
  assert.match(out, /\[dependencies\]\nserde = "1\.0"/);
});

test("bumpClientCargo preserves surrounding whitespace + comments", () => {
  const input = `[package]
name = "starstats-client"
# Decoupled from workspace.package.version after the release-track split.
version = "1.8.4"
edition.workspace = true
`;
  const out = bumpClientCargo(input, "1.9.0");
  assert.match(out, /# Decoupled from workspace\.package\.version/);
  assert.match(out, /version = "1\.9\.0"/);
});

test("bumpClientCargo is idempotent when version already matches", () => {
  const input = `[package]
name = "starstats-client"
version = "1.8.4"
edition.workspace = true
`;
  assert.equal(bumpClientCargo(input, "1.8.4"), input);
});

test("bumpClientCargo refuses files still on version.workspace = true", () => {
  // Guard against the file getting reverted to workspace inheritance,
  // which would silently merge the tray onto the platform's cycle.
  const input = `[package]
name = "starstats-client"
version.workspace = true
edition.workspace = true
`;
  assert.throws(
    () => bumpClientCargo(input, "1.9.0"),
    /workspace/i,
  );
});

test("bumpClientCargo throws if [package] section missing", () => {
  const input = `[dependencies]\nserde = "1.0"\n`;
  assert.throws(() => bumpClientCargo(input, "1.9.0"), /\[package\]/);
});

test("bumpClientCargo throws if version line missing", () => {
  const input = `[package]
name = "starstats-client"
edition.workspace = true
`;
  assert.throws(() => bumpClientCargo(input, "1.9.0"), /version/);
});

// ---------------------------------------------------------------------------
// Track-aware integration scenarios — simulate what cmdPrerelease /
// cmdLive see when tags from BOTH tracks coexist in the repo
// ---------------------------------------------------------------------------

test("integration: tray-v1.8.4 alpha promotion ignores platform tags", () => {
  const tags = [
    "v1.9.0",                  // platform live — higher number but wrong track
    "tray-v1.8.4",             // tray live — actual current
    "tray-v1.8.3",
  ];
  const current = latestSemverFromTags(tags, "tray");
  assert.equal(current, "1.8.4");
  const next = computeNextVersion(current, "prerelease", { channel: "alpha" });
  assert.equal(next, "1.8.5-alpha.1");
  assert.equal(`tray-v${next}`, "tray-v1.8.5-alpha.1");
});

test("integration: platform-v1.9.0 alpha promotion ignores tray tags", () => {
  const tags = [
    "tray-v2.0.0",             // tray live — higher number but wrong track
    "v1.9.0",                  // platform live — actual current
    "v1.8.5",
  ];
  const current = latestSemverFromTags(tags, "platform");
  assert.equal(current, "1.9.0");
  const next = computeNextVersion(current, "prerelease", { channel: "alpha" });
  assert.equal(next, "1.9.1-alpha.1");
  assert.equal(`v${next}`, "v1.9.1-alpha.1");
});

test("integration: tray and platform independent cycle counters", () => {
  // Both tracks happen to be at 1.8.4-alpha.N simultaneously but
  // with different N. Each track's promotion sees only its own N.
  const tags = [
    "tray-v1.8.4-alpha.3",
    "tray-v1.8.4-alpha.2",
    "v1.8.4-alpha.7",
    "v1.8.4-alpha.6",
  ];
  const trayCur = latestSemverFromTags(tags, "tray");
  const platformCur = latestSemverFromTags(tags, "platform");
  assert.equal(trayCur, "1.8.4-alpha.3");
  assert.equal(platformCur, "1.8.4-alpha.7");
  const trayNext = computeNextVersion(trayCur, "prerelease", { channel: "alpha" });
  const platformNext = computeNextVersion(platformCur, "prerelease", { channel: "alpha" });
  assert.equal(trayNext, "1.8.4-alpha.4");
  assert.equal(platformNext, "1.8.4-alpha.8");
});

// ---------------------------------------------------------------------------
// highestNForCycleChannel + computeNextVersion with existingTags
// ---------------------------------------------------------------------------
// 2026-05-24 regression: platform-track promote after PR #91's split kept
// hitting `current: beta.2 → next: alpha.1 (already exists)` on every push
// that didn't bump bare semver. The fix routes the full tag list through
// computeNextVersion so the channel-switch path advances N past whatever
// already exists for the target channel in the cycle.

test("highestNForCycleChannel returns 0 when no matching tag", () => {
  assert.equal(
    highestNForCycleChannel(["v1.8.4-beta.2", "v1.8.4-rc.1"], "platform", "1.8.4", "alpha"),
    0,
  );
});

test("highestNForCycleChannel returns max N for matching channel + cycle", () => {
  const tags = [
    "v1.8.4-alpha.1", "v1.8.4-alpha.2", "v1.8.4-alpha.7", "v1.8.4-alpha.8",
    "v1.8.4-beta.1", "v1.8.4-beta.2",
  ];
  assert.equal(highestNForCycleChannel(tags, "platform", "1.8.4", "alpha"), 8);
  assert.equal(highestNForCycleChannel(tags, "platform", "1.8.4", "beta"), 2);
  assert.equal(highestNForCycleChannel(tags, "platform", "1.8.4", "rc"), 0);
});

test("highestNForCycleChannel ignores tags from other track", () => {
  const tags = [
    "v1.8.4-alpha.5",         // platform
    "tray-v1.8.4-alpha.9",    // tray — must not count when track=platform
  ];
  assert.equal(highestNForCycleChannel(tags, "platform", "1.8.4", "alpha"), 5);
  assert.equal(highestNForCycleChannel(tags, "tray", "1.8.4", "alpha"), 9);
});

test("highestNForCycleChannel ignores other cycles", () => {
  const tags = [
    "v1.8.3-alpha.4",  // different X.Y.Z
    "v1.8.4-alpha.2",
    "v1.8.5-alpha.7",  // different X.Y.Z
  ];
  assert.equal(highestNForCycleChannel(tags, "platform", "1.8.4", "alpha"), 2);
});

test("highestNForCycleChannel rejects unknown channel", () => {
  assert.throws(() => highestNForCycleChannel([], "platform", "1.8.4", "stable"));
});

test("highestNForCycleChannel rejects unknown track", () => {
  assert.throws(() => highestNForCycleChannel([], "wrong", "1.8.4", "alpha"));
});

test("regression: computeNextVersion with existingTags advances past in-cycle alphas after a beta", () => {
  // The exact scenario that failed Promote release on 2026-05-24 commit
  // 9b8f048 — cycle had alphas.1-8, then beta.1+beta.2, and a push that
  // resolved to alpha (no bare-semver bump) tried to mint alpha.1 again.
  // With existingTags + track, computeNextVersion should return alpha.9.
  const existingTags = [
    "v1.8.4-alpha.1", "v1.8.4-alpha.2", "v1.8.4-alpha.3", "v1.8.4-alpha.4",
    "v1.8.4-alpha.5", "v1.8.4-alpha.6", "v1.8.4-alpha.7", "v1.8.4-alpha.8",
    "v1.8.4-beta.1", "v1.8.4-beta.2",
  ];
  const next = computeNextVersion("1.8.4-beta.2", "prerelease", {
    channel: "alpha",
    existingTags,
    track: "platform",
  });
  assert.equal(next, "1.8.4-alpha.9");
});

test("regression: computeNextVersion with existingTags handles fresh channel cleanly", () => {
  // After alpha.2, switching to beta the FIRST time — no existing betas
  // in the cycle, so beta.1 is correct.
  const existingTags = ["v1.8.4-alpha.1", "v1.8.4-alpha.2"];
  const next = computeNextVersion("1.8.4-alpha.2", "prerelease", {
    channel: "beta",
    existingTags,
    track: "platform",
  });
  assert.equal(next, "1.8.4-beta.1");
});

test("computeNextVersion without existingTags preserves legacy reset-to-1 behaviour", () => {
  // Existing callers that don't pass existingTags get the old behaviour.
  // Important: there are 65 prior tests that rely on this and would break
  // if the fix changed the default path.
  const next = computeNextVersion("1.8.4-beta.2", "prerelease", {
    channel: "alpha",
  });
  assert.equal(next, "1.8.4-alpha.1");
});

test("computeNextVersion existingTags respects track filter", () => {
  // A tray alpha.9 must not bump platform's alpha counter.
  const next = computeNextVersion("1.8.4-beta.1", "prerelease", {
    channel: "alpha",
    existingTags: ["tray-v1.8.4-alpha.9", "v1.8.4-alpha.3", "v1.8.4-beta.1"],
    track: "platform",
  });
  assert.equal(next, "1.8.4-alpha.4");
});

test("computeNextVersion existingTags + explicit --n still validated", () => {
  // --n still trumps automatic computation, BUT must not collide with
  // existing tags (the script's --n check is "must advance forward of
  // CURRENT channel" — collisions with the target channel after a switch
  // remain operator-error scope).
  const next = computeNextVersion("1.8.4-beta.2", "prerelease", {
    channel: "alpha",
    n: 15,
    existingTags: ["v1.8.4-alpha.1", "v1.8.4-alpha.8"],
    track: "platform",
  });
  assert.equal(next, "1.8.4-alpha.15");
});

// ---------------------------------------------------------------------------
// parseSlugsFromPrLabels — auto-discover the roadmap slug from merged
// PR labels in the range about to be released. Pure parsing only;
// the gh + git IO that produces the PR list is tested manually.
// ---------------------------------------------------------------------------

test("parseSlugsFromPrLabels: extracts unique slugs from gh JSON shape", () => {
  const prs = [
    { number: 1, labels: [{ name: "roadmap/foo" }, { name: "kind/feature" }] },
    { number: 2, labels: [{ name: "roadmap/bar" }] },
    { number: 3, labels: [{ name: "roadmap/foo" }] }, // duplicate slug
  ];
  assert.deepEqual(parseSlugsFromPrLabels(prs).sort(), ["bar", "foo"]);
});

test("parseSlugsFromPrLabels: tolerates string labels too (gh CLI shape varies)", () => {
  const prs = [
    { number: 1, labels: ["roadmap/foo", "ci"] },
    { number: 2, labels: ["roadmap/bar"] },
  ];
  assert.deepEqual(parseSlugsFromPrLabels(prs).sort(), ["bar", "foo"]);
});

test("parseSlugsFromPrLabels: empty list → empty array", () => {
  assert.deepEqual(parseSlugsFromPrLabels([]), []);
});

test("parseSlugsFromPrLabels: PRs without roadmap/ labels → empty", () => {
  const prs = [
    { number: 1, labels: [{ name: "kind/feature" }, { name: "needs-review" }] },
    { number: 2, labels: [] },
    { number: 3, labels: null },
  ];
  assert.deepEqual(parseSlugsFromPrLabels(prs), []);
});

test("parseSlugsFromPrLabels: ignores empty slug suffix", () => {
  // `roadmap/` with nothing after the slash is malformed; don't return ""
  const prs = [{ number: 1, labels: [{ name: "roadmap/" }, { name: "roadmap/ok" }] }];
  assert.deepEqual(parseSlugsFromPrLabels(prs), ["ok"]);
});

// ---------------------------------------------------------------------------
// previousTrackTagBelow — given a tag list + track + current version,
// return the highest track tag whose version is STRICTLY below the
// current. Used by the slug auto-discovery to find the SHA range to
// scan for merged PRs.
// ---------------------------------------------------------------------------

test("previousTrackTagBelow: rc.2 → finds beta.2 of same minor", () => {
  const tags = [
    "tray-v1.8.9-alpha.1",
    "tray-v1.8.9-alpha.2",
    "tray-v1.8.9-beta.1",
    "tray-v1.8.9-beta.2",
    "tray-v1.8.9-rc.1",
    "tray-v1.8.9-rc.2",
  ];
  assert.equal(
    previousTrackTagBelow(tags, "tray", "1.8.9-rc.2"),
    "tray-v1.8.9-rc.1",
  );
});

test("previousTrackTagBelow: live → finds rc of same minor", () => {
  const tags = [
    "tray-v1.8.8",
    "tray-v1.8.9-rc.1",
    "tray-v1.8.9-rc.2",
    "tray-v1.8.9",
  ];
  assert.equal(
    previousTrackTagBelow(tags, "tray", "1.8.9"),
    "tray-v1.8.9-rc.2",
  );
});

test("previousTrackTagBelow: alpha.1 of new minor → finds previous live", () => {
  const tags = [
    "tray-v1.8.8",
    "tray-v1.8.9",
    "tray-v1.8.10-alpha.1",
  ];
  assert.equal(
    previousTrackTagBelow(tags, "tray", "1.8.10-alpha.1"),
    "tray-v1.8.9",
  );
});

test("previousTrackTagBelow: respects track prefix (tray vs platform)", () => {
  const tags = [
    "v1.8.10",        // platform live
    "v1.8.11-alpha.1",
    "tray-v1.8.8",    // unrelated track
    "tray-v1.8.9",
  ];
  // For platform track, only the bare-semver tags count
  assert.equal(
    previousTrackTagBelow(tags, "platform", "1.8.11-alpha.1"),
    "v1.8.10",
  );
});

test("previousTrackTagBelow: first ever tag on a track → null", () => {
  const tags = ["tray-v1.0.0-alpha.1"];
  assert.equal(previousTrackTagBelow(tags, "tray", "1.0.0-alpha.1"), null);
});

test("previousTrackTagBelow: ignores non-track tags + malformed entries", () => {
  const tags = [
    "tray-v1.8.8",
    "tray-v1.8.9",
    "release-2024",        // garbage
    "",                    // empty line
    "v1.8.10",             // wrong track
  ];
  assert.equal(
    previousTrackTagBelow(tags, "tray", "1.8.10-alpha.1"),
    "tray-v1.8.9",
  );
});


// ---------------------------------------------------------------------------
// bumpVersionFiles — Cargo.lock refresh ladder
//
// Regression cover for the v1.8.68 drift: Cargo.toml went to 1.8.68 while
// Cargo.lock stayed at 1.8.67. Two stacked defects caused it —
//   (1) the fallback sat in a `catch` after a `run(..., {critical:false})`
//       call, and run() only rethrows when critical is true, so the catch was
//       unreachable and the fallback never ran; and
//   (2) every attempt passed `--offline`, so the fallback shared the exact
//       failure mode (cold registry cache) it was meant to rescue.
// These tests pin both properties.
// ---------------------------------------------------------------------------

/** Records tryRun calls; fails the first `failFirstN` of them. */
function fakeRunner(failFirstN = 0) {
  return {
    calls: [],
    warnings: [],
    writeFile() {},
    tryRun(cmd, args) {
      this.calls.push([cmd, ...args].join(" "));
      return this.calls.length > failFirstN;
    },
  };
}

test("cargo lock refresh: first attempt succeeds → no fallback work", () => {
  const r = fakeRunner(0);
  bumpVersionFiles(r, "platform", "9.9.9");
  assert.equal(r.calls.length, 1);
  assert.equal(r.calls[0], "cargo update --workspace --offline");
});

test("cargo lock refresh: first attempt fails → fallback IS reached", () => {
  // The original bug: this assertion failed because the fallback lived in an
  // unreachable catch, so only one call was ever made.
  const r = fakeRunner(1);
  bumpVersionFiles(r, "platform", "9.9.9");
  assert.equal(r.calls.length, 2, "second attempt must run when the first fails");
  assert.match(r.calls[1], /-p starstats-core/);
});

test("cargo lock refresh: offline attempts exhausted → falls back to ONLINE", () => {
  // A cold registry cache fails every --offline rung, so the ladder must end
  // with an attempt that may reach the network, or it cannot recover at all.
  const r = fakeRunner(2);
  bumpVersionFiles(r, "platform", "9.9.9");
  assert.equal(r.calls.length, 3);
  assert.equal(r.calls[2], "cargo update --workspace");
  assert.ok(!r.calls[2].includes("--offline"), "last resort must not be offline-only");
});

test("cargo lock refresh: all attempts fail → stops, does not loop forever", () => {
  const r = fakeRunner(99);
  bumpVersionFiles(r, "platform", "9.9.9");
  assert.equal(r.calls.length, CARGO_LOCK_REFRESH_ATTEMPTS.length);
});

test("cargo lock refresh ladder: at least one attempt is not --offline", () => {
  const hasOnline = CARGO_LOCK_REFRESH_ATTEMPTS.some(
    (a) => !a.includes("--offline"),
  );
  assert.ok(hasOnline, "an all-offline ladder cannot survive a cold registry cache");
});
