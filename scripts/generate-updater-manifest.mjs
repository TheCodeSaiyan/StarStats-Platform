#!/usr/bin/env node
// Generate a Tauri 2 updater manifest from downloaded CI artifacts.
//
// Usage:
//   node scripts/generate-updater-manifest.mjs \
//     --artifacts-dir <dir> \
//     --version <semver> \
//     --base-url <url> \
//     --output <file> \
//     [--tag <tag>]   (literal tag name for the notes URL; defaults to
//                      `v${version}` for back-compat with pre-2026-05-23
//                      single-track callers. Required for tray-track
//                      releases since the tag is `tray-v...`.)
//
// The script walks <artifacts-dir> recursively for Tauri 2 updater bundle
// pairs (a bundle file and its sibling .sig). For each match it embeds the
// minisign signature (full file content, including trailing newline) into a
// platform entry keyed per the Tauri target triple.
//
// Bundles searched (matched in priority order — first match wins per platform):
//   - Windows NSIS:    *_x64-setup.exe[.sig]        -> windows-x86_64 (preferred)
//   - Windows MSI:     *_x64_en-US.msi[.sig]        -> windows-x86_64 (fallback)
//   - Linux AppImage:  *_amd64.AppImage[.sig]       -> linux-x86_64
//
// Tauri 2's `bundle.createUpdaterArtifacts: true` produces a `.sig` next to
// the actual installer (.exe / .msi / .AppImage). The plugin downloads that
// installer at update time, verifies its signature against the embedded
// pubkey, and runs it. Older docs reference `.nsis.zip` bundles — that
// format is no longer produced in Tauri 2's current packaging.
//
// Exits 1 if no signed bundles are found (signing was skipped or assets are
// missing) so CI fails loudly instead of publishing an empty manifest.
//
// The --version flag is the manifest's advertised "latest" version. It MUST
// match the version embedded in the bundle filenames (which comes from the
// workspace Cargo.toml). The script enforces this — if --version doesn't
// match the version found in the bundle filenames, it fails. This catches
// the common release-prep error of tagging vX.Y.Z without bumping
// Cargo.toml first, which would otherwise produce a manifest that promises
// vX.Y.Z and points at vA.B.C bundles, sending clients into update loops.

import { readFile, readdir, stat, writeFile, mkdir } from "node:fs/promises";
import { dirname, join, basename } from "node:path";
import { argv, exit, stdout, stderr } from "node:process";

function parseArgs(rawArgs) {
  const args = {};
  for (let i = 0; i < rawArgs.length; i += 1) {
    const flag = rawArgs[i];
    if (!flag.startsWith("--")) continue;
    const key = flag.slice(2);
    const value = rawArgs[i + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`Missing value for --${key}`);
    }
    args[key] = value;
    i += 1;
  }
  return args;
}

async function walk(dir) {
  const out = [];
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch (err) {
    if (err.code === "ENOENT") return out;
    throw err;
  }
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...(await walk(full)));
    } else if (entry.isFile()) {
      out.push(full);
    }
  }
  return out;
}

// Tauri 2 updater bundle matchers. Listed in priority order — when multiple
// matchers hit on the same platform (e.g. both NSIS and MSI on Windows), the
// earlier entry wins. NSIS is preferred because its `passive` install mode
// runs without elevation prompts on user-scope installs.
const PLATFORM_MATCHERS = [
  {
    platform: "windows-x86_64",
    bundleSuffix: "_x64-setup.exe",
  },
  {
    platform: "windows-x86_64",
    bundleSuffix: "_x64_en-US.msi",
  },
  {
    platform: "linux-x86_64",
    bundleSuffix: "_amd64.AppImage",
  },
];

// Bundle filenames look like `StarStats_0.1.0_x64-setup.exe` or
// `StarStats_0.1.0_amd64.AppImage`. Capture the semver between the
// product name and the platform/arch tag so we can cross-check against
// the --version flag.
const VERSION_FROM_FILENAME = /_(\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?)_/;

function extractVersion(filePath) {
  const m = basename(filePath).match(VERSION_FROM_FILENAME);
  return m ? m[1] : null;
}

function matchPlatform(filePath) {
  const name = basename(filePath);
  for (const matcher of PLATFORM_MATCHERS) {
    if (name.endsWith(matcher.bundleSuffix)) {
      return { platform: matcher.platform, bundleName: name };
    }
  }
  return null;
}

async function main() {
  const args = parseArgs(argv.slice(2));
  const required = ["artifacts-dir", "version", "base-url", "output"];
  for (const key of required) {
    if (!args[key]) {
      stderr.write(`error: missing required flag --${key}\n`);
      exit(2);
    }
  }

  const artifactsDir = args["artifacts-dir"];
  const version = args["version"];
  const baseUrl = args["base-url"].replace(/\/$/, "");
  const outputPath = args["output"];

  const all = await walk(artifactsDir);
  // Index sig files by their bundle path (strip .sig).
  const sigByBundle = new Map();
  for (const f of all) {
    if (f.endsWith(".sig")) {
      sigByBundle.set(f.slice(0, -4), f);
    }
  }

  const platforms = {};
  const missingSigs = [];
  const bundleVersions = new Set();
  // Iterate matchers in priority order, not files. The earlier walk()
  // returns files in directory order — for the same platform, that
  // would let an MSI beat an NSIS .exe just by lexical path order.
  // By looping matchers first we honour the priority list: NSIS wins
  // for windows-x86_64 because it's listed before the MSI matcher.
  for (const matcher of PLATFORM_MATCHERS) {
    if (platforms[matcher.platform]) {
      // Already filled by an earlier (higher-priority) matcher.
      continue;
    }
    const candidates = all.filter(
      (f) => !f.endsWith(".sig") && f.endsWith(matcher.bundleSuffix),
    );
    if (candidates.length === 0) continue;
    if (candidates.length > 1) {
      stderr.write(
        `warning: multiple bundles match ${matcher.platform} via ` +
          `${matcher.bundleSuffix}; keeping ${basename(candidates[0])}, ` +
          `discarding ${candidates.slice(1).map(basename).join(", ")}\n`,
      );
    }
    const bundlePath = candidates[0];
    const sigPath = sigByBundle.get(bundlePath);
    if (!sigPath) {
      missingSigs.push(bundlePath);
      continue;
    }
    const signature = await readFile(sigPath, "utf8");
    if (!signature.trim()) {
      stderr.write(`error: signature file is empty: ${sigPath}\n`);
      exit(1);
    }
    const bundleVersion = extractVersion(bundlePath);
    if (bundleVersion) {
      bundleVersions.add(bundleVersion);
    }
    platforms[matcher.platform] = {
      signature,
      url: `${baseUrl}/${basename(bundlePath)}`,
    };
  }

  // Cross-check the --version flag against the version baked into the
  // bundle filenames. A mismatch means the tag was pushed without
  // bumping the workspace Cargo.toml — which would publish a manifest
  // that promises a new version but points at the OLD binaries,
  // sending clients into a "download, install, see same version,
  // poll again" loop.
  if (bundleVersions.size > 1) {
    stderr.write(
      `error: bundles report inconsistent versions: ${[...bundleVersions].join(", ")}. ` +
        "All update bundles in a single release must come from the same Cargo build.\n",
    );
    exit(1);
  }
  if (bundleVersions.size === 1) {
    const [bundleVersion] = bundleVersions;
    // MSI rejects non-numeric pre-release identifiers ("-alpha", "-rc1"),
    // so the Tauri bundler version (in tauri.conf.json) is intentionally
    // a numeric-only prefix of the marketing/tag version. Accept both:
    //   - exact match (tag and bundle are the same)
    //   - bundle is the numeric core of the tag (tag may carry a
    //     pre-release suffix the bundler couldn't keep)
    const bundleMatches =
      bundleVersion === version || version.replace(/-.*$/, "") === bundleVersion;
    if (!bundleMatches) {
      stderr.write(
        `error: --version (${version}) does not match the version in bundle ` +
          `filenames (${bundleVersion}). Bump the workspace Cargo.toml version ` +
          `to ${version} and rebuild before tagging.\n`,
      );
      exit(1);
    }
  }

  if (missingSigs.length > 0) {
    stderr.write(
      `error: ${missingSigs.length} bundle(s) found without paired .sig — ` +
        "the build was meant to be signed (TAURI_SIGNING_PRIVATE_KEY set) " +
        "but signing failed for these. Refusing to publish a partial manifest:\n",
    );
    for (const f of missingSigs) {
      stderr.write(`  ${f}\n`);
    }
    exit(1);
  }

  const platformKeys = Object.keys(platforms);
  if (platformKeys.length === 0) {
    stderr.write(
      "error: no signed updater bundles found under " +
        `${artifactsDir}. Looked for *.sig files paired with ` +
        PLATFORM_MATCHERS.map((m) => `*${m.bundleSuffix}`).join(", ") +
        ". Was TAURI_SIGNING_PRIVATE_KEY set during the build?\n",
    );
    exit(1);
  }

  // The notes URL must point at the ACTUAL release tag, not a
  // synthesised `v${version}`. Pre-release-tracks-split (2026-05-23)
  // both were the same — every release was tagged `vX.Y.Z[-suffix]`.
  // Post-split, tray releases use `tray-vX.Y.Z[-suffix]`, so the
  // synthesised `v${version}` link 404s. Caller passes `--tag` with
  // the literal tag name (release.yml: `${{ github.ref_name }}`).
  // Falls back to the legacy `v${version}` shape when `--tag` is
  // absent, preserving back-compat with any out-of-tree caller.
  const tag = args.tag || `v${version}`;
  const manifest = {
    version,
    pub_date: new Date().toISOString(),
    notes: `See https://github.com/TheCodeSaiyan/StarStats-Platform/releases/tag/${tag}`,
    platforms,
  };

  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");

  stdout.write(
    `wrote ${outputPath} with platforms: ${platformKeys.join(", ")}\n`,
  );
}

main().catch((err) => {
  stderr.write(`error: ${err.stack || err.message || err}\n`);
  exit(1);
});
