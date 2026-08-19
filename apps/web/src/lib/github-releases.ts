/**
 * Server-side fetcher for the StarStats tray desktop releases.
 *
 * Replaces the old hardcoded links to the GitHub *releases page* with a live
 * read of the GitHub Releases API, so the `/downloads` page always surfaces the
 * current stable version, its per-platform installers, and the release notes.
 *
 * Tray releases are tagged `tray-vX.Y.Z[-{alpha,beta,rc}.N]` (the platform
 * track uses bare `vX.Y.Z` tags on the SAME repo — those are filtered out).
 *
 * This module is intentionally isomorphic (no `server-only` guard) so the pure
 * parsing/formatting helpers can be shared by the client `DownloadsView`. The
 * network read lives in `github-releases.server.ts`.
 */

export const RELEASES_REPO = 'TheCodeSaiyan/StarStats-Platform';
export const RELEASES_HTML_URL = `https://github.com/${RELEASES_REPO}/releases`;
export const RELEASES_API = `https://api.github.com/repos/${RELEASES_REPO}/releases`;
const TRAY_TAG_PREFIX = 'tray-v';

export type AssetOs = 'windows' | 'macos' | 'linux';

export interface PlatformAsset {
  os: AssetOs;
  /** Human label, e.g. "Windows Installer (.exe)". */
  label: string;
  /** Short kind discriminator: exe | msi | dmg | app | AppImage | deb. */
  kind: string;
  filename: string;
  url: string;
  size: number;
  /** Preferred asset for its OS (drives the primary CTA per platform). */
  recommended: boolean;
}

export interface TrayRelease {
  tag: string;
  version: string;
  name: string;
  prerelease: boolean;
  publishedAt: string | null;
  htmlUrl: string;
  notes: string;
  assets: PlatformAsset[];
}

export interface TrayReleaseSet {
  stable: TrayRelease | null;
  /** Only populated when a prerelease is *newer* than the latest stable. */
  prerelease: TrayRelease | null;
}

export interface RawAsset {
  name: string;
  browser_download_url: string;
  size: number;
}

export interface RawRelease {
  tag_name: string;
  name: string | null;
  body: string | null;
  draft: boolean;
  prerelease: boolean;
  published_at: string | null;
  html_url: string;
  assets: RawAsset[];
}

/**
 * Ordered classification rules. `$`-anchored so a signature file
 * (`*.exe.sig`, `*.deb.sig`) never matches the installer rule.
 * OS display order also comes from this array's ordering.
 */
const ASSET_RULES: ReadonlyArray<{
  test: RegExp;
  os: AssetOs;
  kind: string;
  label: string;
  recommended: boolean;
}> = [
  { test: /-setup\.exe$/i, os: 'windows', kind: 'exe', label: 'Windows Installer (.exe)', recommended: true },
  { test: /\.msi$/i, os: 'windows', kind: 'msi', label: 'Windows Installer (.msi)', recommended: false },
  { test: /\.dmg$/i, os: 'macos', kind: 'dmg', label: 'macOS (.dmg)', recommended: true },
  { test: /\.app\.tar\.gz$/i, os: 'macos', kind: 'app', label: 'macOS (.app)', recommended: false },
  { test: /\.AppImage$/i, os: 'linux', kind: 'AppImage', label: 'Linux (AppImage)', recommended: true },
  { test: /\.deb$/i, os: 'linux', kind: 'deb', label: 'Linux (.deb)', recommended: false },
];

const OS_ORDER: AssetOs[] = ['windows', 'macos', 'linux'];

export function isTrayTag(tag: string): boolean {
  return tag.startsWith(TRAY_TAG_PREFIX);
}

export function parseTrayVersion(tag: string): string {
  if (tag.startsWith(TRAY_TAG_PREFIX)) return tag.slice(TRAY_TAG_PREFIX.length);
  return tag.replace(/^tray-/, '').replace(/^v/, '');
}

export function classifyAsset(raw: RawAsset): PlatformAsset | null {
  for (const rule of ASSET_RULES) {
    if (rule.test.test(raw.name)) {
      return {
        os: rule.os,
        kind: rule.kind,
        label: rule.label,
        filename: raw.name,
        url: raw.browser_download_url,
        size: raw.size,
        recommended: rule.recommended,
      };
    }
  }
  return null;
}

function mapRelease(raw: RawRelease): TrayRelease {
  const assets = raw.assets
    .map(classifyAsset)
    .filter((a): a is PlatformAsset => a !== null)
    .sort((a, b) => {
      const osDelta = OS_ORDER.indexOf(a.os) - OS_ORDER.indexOf(b.os);
      if (osDelta !== 0) return osDelta;
      // Recommended asset first within an OS.
      return Number(b.recommended) - Number(a.recommended);
    });
  return {
    tag: raw.tag_name,
    version: parseTrayVersion(raw.tag_name),
    name: raw.name?.trim() || raw.tag_name,
    prerelease: raw.prerelease,
    publishedAt: raw.published_at,
    htmlUrl: raw.html_url,
    notes: (raw.body ?? '').trim(),
    assets,
  };
}

/** Drop drafts + non-tray (platform) tags, then map to the domain type. */
export function mapReleases(raw: RawRelease[]): TrayRelease[] {
  return raw
    .filter((r) => !r.draft && isTrayTag(r.tag_name))
    .map(mapRelease);
}

function publishedMs(r: TrayRelease): number {
  return r.publishedAt ? Date.parse(r.publishedAt) : 0;
}

export function selectReleases(releases: TrayRelease[]): TrayReleaseSet {
  const sorted = [...releases].sort((a, b) => publishedMs(b) - publishedMs(a));
  const stable = sorted.find((r) => !r.prerelease) ?? null;
  const latestPre = sorted.find((r) => r.prerelease) ?? null;

  // Only surface a prerelease when it is genuinely newer than the stable line —
  // otherwise the latest stable has already superseded it.
  const prerelease =
    latestPre && (!stable || publishedMs(latestPre) > publishedMs(stable))
      ? latestPre
      : null;

  return { stable, prerelease };
}

const KB = 1024;
const MB = KB * 1024;

export function formatBytes(bytes: number): string {
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  if (bytes >= KB) return `${Math.round(bytes / KB)} KB`;
  return `${bytes} B`;
}
