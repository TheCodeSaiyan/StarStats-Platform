'use client';

import React, { useEffect, useState } from 'react';
import { Plane, HoloTable, HoloKV, BeamChip, Flatline } from 'holo';
import {
  formatBytes,
  RELEASES_HTML_URL,
  type AssetOs,
  type PlatformAsset,
  type TrayRelease,
} from '@/lib/github-releases';

/**
 * The download half of the Emitter, redrawn.
 *
 * Every judgement is lifted from the flat `DownloadsView` unchanged — OS
 * detection from the user agent, detected platform sorted first, the
 * macOS-specific "on the roadmap" wording, the recommended asset getting the
 * primary affordance, release notes and the preview build behind disclosure.
 * Only the drawing is new.
 *
 * TWO DELIBERATE CHANGES, both required by the system:
 *
 *   - The platform glyphs are gone. `DownloadsView` used a 🐧 for Linux, and
 *     the system permits no emoji anywhere — geometric Unicode and 1.4–1.6
 *     stroke SVG only. The names carry it; there was never information in the
 *     glyph.
 *   - Assets are a table rather than a stack of cards. Four artifacts across
 *     three platforms is tabular data, and the kit's own Emitter screen reads
 *     it as one — platform, artifact, action.
 *
 * Detection runs in an effect, so the first paint has no platform highlighted
 * and the server render matches. That is intentional: guessing server-side
 * would need the UA header and would be wrong behind any cache.
 */
const OS_NAMES: Record<AssetOs, string> = {
  windows: 'Windows',
  macos: 'macOS',
  linux: 'Linux',
};

const OS_ORDER: AssetOs[] = ['windows', 'macos', 'linux'];

function detectOs(): AssetOs | null {
  if (typeof navigator === 'undefined') return null;
  const ua = navigator.userAgent;
  if (/Windows/i.test(ua)) return 'windows';
  if (/Mac OS X|Macintosh/i.test(ua)) return 'macos';
  if (/Linux|X11|CrOS/i.test(ua)) return 'linux';
  return null;
}

function groupByOs(assets: PlatformAsset[]): Map<AssetOs, PlatformAsset[]> {
  const map = new Map<AssetOs, PlatformAsset[]>();
  for (const a of assets) {
    const list = map.get(a.os) ?? [];
    list.push(a);
    map.set(a.os, list);
  }
  return map;
}

function fmtDate(iso: string | null): string {
  if (!iso) return '';
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    });
  } catch {
    return iso;
  }
}

/** Shipped copy, verbatim — a port redraws, it does not reword. */
function missingCopy(os: AssetOs): string {
  return os === 'macos'
    ? 'No macOS build yet — it’s on the roadmap.'
    : 'No installer in this release.';
}

export function ReleasePlanes({
  stable,
  prerelease,
  error,
}: {
  stable: TrayRelease | null;
  prerelease: TrayRelease | null;
  error: boolean;
}) {
  const [os, setOs] = useState<AssetOs | null>(null);
  useEffect(() => setOs(detectOs()), []);

  if (!stable) {
    return (
      <Flatline
        title={
          error
            ? 'Couldn’t reach the release feed'
            : 'No release published yet'
        }
        // `FlatlineReason` is a closed set and neither member fits a GitHub
        // outage, so the hint below carries the meaning and the reason only
        // picks a default that is never used.
        reason="no-data"
        hint={
          error
            ? 'The live download list is temporarily unavailable. You can still grab the latest build straight from GitHub.'
            : 'Once the first tray build ships it will appear here automatically.'
        }
        action={
          <a
            href={RELEASES_HTML_URL}
            target="_blank"
            rel="noreferrer noopener"
            className="hp-btn hp-btn--primary"
          >
            Open releases on GitHub →
          </a>
        }
      />
    );
  }

  const byOs = groupByOs(stable.assets);
  // Detected OS first, then canonical order.
  const orderedOs = [...OS_ORDER].sort((a, b) => {
    if (a === os) return -1;
    if (b === os) return 1;
    return OS_ORDER.indexOf(a) - OS_ORDER.indexOf(b);
  });

  // Declared, not inferred. The two branches below produce structurally
  // different rows (a platform with no build has no download action), and
  // `flatMap` would otherwise infer a union that `HoloTable`'s single row type
  // cannot accept.
  interface AssetRow {
    key: string;
    platform: React.ReactNode;
    artifact: React.ReactNode;
    size: React.ReactNode;
    get: React.ReactNode;
  }

  const rows: AssetRow[] = orderedOs.flatMap<AssetRow>((o) => {
    const assets = byOs.get(o);
    if (!assets || assets.length === 0) {
      return [
        {
          key: `${o}-none`,
          platform: (
            <>
              {OS_NAMES[o]}
              {o === os ? (
                <BeamChip dot style={{ marginLeft: 8 }}>
                  Your system
                </BeamChip>
              ) : null}
            </>
          ),
          artifact: <span className="hp-dim">{missingCopy(o)}</span>,
          size: '—',
          get: null,
        },
      ];
    }
    return assets.map((a, i) => ({
      key: a.filename,
      platform:
        i === 0 ? (
          <>
            {OS_NAMES[o]}
            {o === os ? (
              <BeamChip dot style={{ marginLeft: 8 }}>
                Your system
              </BeamChip>
            ) : null}
          </>
        ) : (
          ''
        ),
      artifact: a.label,
      size: formatBytes(a.size),
      get: (
        <a
          href={a.url}
          className={
            a.recommended ? 'hp-btn hp-btn--primary' : 'hp-btn hp-btn--ghost'
          }
          download
        >
          Get
        </a>
      ),
    }));
  });

  return (
    <>
      <HoloKV
        items={[
          { k: 'Latest', v: `v${stable.version}` },
          {
            k: 'Released',
            // `toLocaleDateString(undefined, …)` deliberately formats in the
            // READER's locale, which is not the server's — so the SSR text and
            // the hydrated text legitimately differ and React 19 treats that
            // as a hydration failure, regenerating the tree and logging a page
            // error. `suppressHydrationWarning` is the documented mechanism
            // for exactly this case: text that is SUPPOSED to differ.
            //
            // The alternatives were both worse: pinning a locale would show a
            // reader outside en-US a date in a format they do not use, and
            // formatting server-side would pick the container's locale, which
            // is nobody's. Inherited from the flat `DownloadsView`, where the
            // same mismatch was live and unnoticed because `/downloads` had no
            // e2e coverage until this surface absorbed pairing.
            v: (
              <span suppressHydrationWarning>
                {fmtDate(stable.publishedAt) || '—'}
              </span>
            ),
          },
        ]}
      />

      <Plane tilt="flat" cap="Builds" hint="code-signed" style={{ marginTop: 18 }}>
        <HoloTable
          columns={[
            { key: 'platform', label: 'Platform' },
            { key: 'artifact', label: 'Artifact' },
            { key: 'size', label: 'Size', numeric: true },
            { key: 'get', label: '' },
          ]}
          rows={rows}
        />
      </Plane>

      <p className="hp-prose">
        {/* The leading ↻ is shipped copy and is geometric Unicode, which the
            system allows; only emoji are barred. Kept verbatim. */}
        ↻ The tray updates itself after the first install — you only need to
        download once.
      </p>

      {stable.notes ? (
        <details className="hp-disclose">
          <summary>Release notes — {stable.name}</summary>
          <div className="hp-prose hp-prose--pre">{stable.notes}</div>
        </details>
      ) : null}

      {prerelease ? (
        <details className="hp-disclose">
          <summary>Preview build · v{prerelease.version}</summary>
          <p className="hp-prose">
            Early access to the next release. Expect rough edges.
          </p>
          <div className="hp-formrow">
            {prerelease.assets.map((a) => (
              <a
                key={a.filename}
                href={a.url}
                className="hp-btn hp-btn--ghost"
                download
              >
                {a.label} · {formatBytes(a.size)}
              </a>
            ))}
          </div>
        </details>
      ) : null}

      <p className="hp-prose">
        <a href={RELEASES_HTML_URL} target="_blank" rel="noreferrer noopener">
          All releases on GitHub →
        </a>
      </p>
    </>
  );
}
