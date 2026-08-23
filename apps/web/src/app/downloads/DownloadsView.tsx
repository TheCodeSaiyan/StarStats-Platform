'use client';

import React, { useEffect, useState } from 'react';
import {
  formatBytes,
  RELEASES_HTML_URL,
  type AssetOs,
  type PlatformAsset,
  type TrayRelease,
} from '@/lib/github-releases';
import {
  CODE_SIGNING_PUBLISHER,
  CODE_SIGNING_SUBJECT_DN,
} from '@/lib/signing';

const OS_META: Record<AssetOs, { name: string; glyph: string }> = {
  windows: { name: 'Windows', glyph: '⊞' },
  macos: { name: 'macOS', glyph: '' },
  linux: { name: 'Linux', glyph: '🐧' },
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

function PlatformCard({
  os,
  assets,
  detected,
}: {
  os: AssetOs;
  assets: PlatformAsset[] | undefined;
  detected: boolean;
}) {
  const meta = OS_META[os];
  return (
    <div
      className="ss-card"
      aria-current={detected ? 'true' : undefined}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 12,
        padding: 20,
        borderColor: detected ? 'var(--accent)' : undefined,
        boxShadow: detected ? '0 0 0 1px var(--accent)' : undefined,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        {meta.glyph && <span aria-hidden style={{ fontSize: 18 }}>{meta.glyph}</span>}
        <h3 style={{ margin: 0, fontSize: 'var(--fs-md)', fontWeight: 600 }}>{meta.name}</h3>
        {detected && (
          <span
            style={{
              marginLeft: 'auto',
              fontSize: 11,
              color: 'var(--accent)',
              border: '1px solid var(--accent)',
              borderRadius: 0,
              padding: '1px 8px',
            }}
          >
            Your system
          </span>
        )}
      </div>

      {!assets || assets.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13, fontStyle: 'italic' }}>
          {os === 'macos'
            ? 'No macOS build yet — it’s on the roadmap.'
            : 'No installer in this release.'}
        </p>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {assets.map((a) => (
            <a
              key={a.filename}
              href={a.url}
              className={a.recommended ? 'ss-btn ss-btn--primary' : 'ss-btn ss-btn--ghost'}
              style={{ justifyContent: 'space-between', display: 'flex', gap: 8 }}
              download
            >
              <span>{a.label}</span>
              <span style={{ opacity: 0.7, fontSize: 12, fontWeight: 400 }}>
                {formatBytes(a.size)}
              </span>
            </a>
          ))}
        </div>
      )}
    </div>
  );
}

export function DownloadsView({
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
      <div className="ss-card" style={{ padding: 24, marginTop: 24 }}>
        <h2 style={{ margin: '0 0 8px', fontSize: 'var(--fs-md)' }}>
          {error ? 'Couldn’t reach the release feed' : 'No release published yet'}
        </h2>
        <p style={{ margin: '0 0 16px', color: 'var(--fg-dim)' }}>
          {error
            ? 'The live download list is temporarily unavailable. You can still grab the latest build straight from GitHub.'
            : 'Once the first tray build ships it will appear here automatically.'}
        </p>
        <a
          href={RELEASES_HTML_URL}
          target="_blank"
          rel="noreferrer noopener"
          className="ss-btn ss-btn--primary"
        >
          Open releases on GitHub →
        </a>
      </div>
    );
  }

  const byOs = groupByOs(stable.assets);
  // Detected OS first, then canonical order.
  const orderedOs = [...OS_ORDER].sort((a, b) => {
    if (a === os) return -1;
    if (b === os) return 1;
    return OS_ORDER.indexOf(a) - OS_ORDER.indexOf(b);
  });

  return (
    <div style={{ marginTop: 24, display: 'flex', flexDirection: 'column', gap: 24 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span
          style={{
            fontSize: 13,
            fontWeight: 600,
            color: 'var(--accent)',
            border: '1px solid var(--accent)',
            borderRadius: 0,
            padding: '2px 10px',
          }}
        >
          Latest · v{stable.version}
        </span>
        <span style={{ color: 'var(--fg-dim)', fontSize: 13 }}>
          Released {fmtDate(stable.publishedAt)}
        </span>
      </div>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
          gap: 16,
        }}
      >
        {orderedOs.map((o) => (
          <PlatformCard key={o} os={o} assets={byOs.get(o)} detected={o === os} />
        ))}
      </div>

      <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
        ↻ The tray updates itself after the first install — you only need to
        download once.
      </p>

      <details className="ss-card" style={{ padding: '14px 18px' }}>
        <summary style={{ cursor: 'pointer', fontWeight: 600 }}>
          How to check the installer really came from us
        </summary>
        <div
          style={{
            marginTop: 10,
            color: 'var(--fg-dim)',
            fontSize: 13,
            lineHeight: 1.6,
          }}
        >
          <p style={{ margin: '0 0 10px' }}>
            The Windows installers are signed with a certificate issued to us
            by a certificate authority, so Windows names the publisher instead
            of calling it unknown. Right-click the download →{' '}
            <strong>Properties</strong> → <strong>Digital Signatures</strong> →{' '}
            <strong>Details</strong>. It should read:
          </p>
          <p
            style={{
              margin: '0 0 10px',
              fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
              color: 'var(--fg-muted)',
              wordBreak: 'break-all',
            }}
          >
            {CODE_SIGNING_SUBJECT_DN}
          </p>
          <p style={{ margin: '0 0 10px' }}>
            Anything other than <strong>{CODE_SIGNING_PUBLISHER}</strong> and the
            file did not come from us — don’t run it.
          </p>
          <p style={{ margin: '0 0 10px' }}>
            We deliberately don’t print a certificate thumbprint to compare.
            Our certificates are short-lived and rotate constantly, so a
            thumbprint published here would be out of date within days — and a
            perfectly genuine download would then look like a forgery. The
            publisher name above stays the same across every rotation, so
            that’s the thing worth checking.
          </p>
          <p style={{ margin: 0 }}>
            Windows may still show a SmartScreen prompt for a while. That’s a
            separate reputation system that trusts an installer more as more
            people run it, and a newly-issued signing identity starts with
            none. Choose <strong>More info</strong> →{' '}
            <strong>Run anyway</strong> once you’ve confirmed the publisher.
          </p>
        </div>
      </details>

      {stable.notes && (
        <details className="ss-card" style={{ padding: '14px 18px' }}>
          <summary style={{ cursor: 'pointer', fontWeight: 600 }}>
            Release notes — {stable.name}
          </summary>
          <div
            style={{
              whiteSpace: 'pre-wrap',
              color: 'var(--fg)',
              fontSize: 'var(--fs-base)',
              lineHeight: 1.6,
              marginTop: 12,
            }}
          >
            {stable.notes}
          </div>
        </details>
      )}

      {prerelease && (
        <details className="ss-card" style={{ padding: '14px 18px' }}>
          <summary style={{ cursor: 'pointer', fontWeight: 600 }}>
            Preview build · v{prerelease.version}
          </summary>
          <p style={{ color: 'var(--fg-dim)', fontSize: 13, margin: '12px 0' }}>
            Early access to the next release. Expect rough edges.
          </p>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
            {prerelease.assets.map((a) => (
              <a
                key={a.filename}
                href={a.url}
                className="ss-btn ss-btn--ghost"
                download
              >
                {a.label} · {formatBytes(a.size)}
              </a>
            ))}
          </div>
        </details>
      )}

      <div style={{ fontSize: 13, display: 'flex', gap: 20, flexWrap: 'wrap' }}>
        <a
          href={RELEASES_HTML_URL}
          target="_blank"
          rel="noreferrer noopener"
          style={{ color: 'var(--accent)' }}
        >
          All releases on GitHub →
        </a>
      </div>
    </div>
  );
}
