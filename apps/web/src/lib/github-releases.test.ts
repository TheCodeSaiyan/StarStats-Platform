import { describe, it, expect } from 'vitest';
import {
  classifyAsset,
  parseTrayVersion,
  isTrayTag,
  mapReleases,
  selectReleases,
  type RawRelease,
} from './github-releases';

// Real asset names captured verbatim from
// GET repos/TheCodeSaiyan/StarStats-Platform/releases (tray-v1.8.51).
const REAL_ASSETS = [
  'StarStats_1.8.51_x64_en-US.msi.sig',
  'StarStats_1.8.51_amd64.deb.sig',
  'StarStats_1.8.51_x64-setup.exe',
  'StarStats_1.8.51_x64-setup.exe.sig',
  'StarStats.desktop',
  'StarStats_1.8.51_amd64.AppImage',
  'StarStats_1.8.51_amd64.AppImage.sig',
  'updater-manifest.json',
  'StarStats_1.8.51_x64_en-US.msi',
  'StarStats.png',
  'StarStats_1.8.51_amd64.deb',
];

function asset(name: string) {
  return { name, browser_download_url: `https://example/${name}`, size: 100 };
}

describe('classifyAsset', () => {
  it('classifies the NSIS .exe as a recommended Windows installer', () => {
    const a = classifyAsset(asset('StarStats_1.8.51_x64-setup.exe'));
    expect(a).not.toBeNull();
    expect(a!.os).toBe('windows');
    expect(a!.kind).toBe('exe');
    expect(a!.recommended).toBe(true);
  });

  it('classifies the .msi as a (non-recommended) Windows installer', () => {
    const a = classifyAsset(asset('StarStats_1.8.51_x64_en-US.msi'));
    expect(a!.os).toBe('windows');
    expect(a!.kind).toBe('msi');
    expect(a!.recommended).toBe(false);
  });

  it('classifies AppImage and .deb as Linux', () => {
    expect(classifyAsset(asset('StarStats_1.8.51_amd64.AppImage'))!.os).toBe('linux');
    expect(classifyAsset(asset('StarStats_1.8.51_amd64.deb'))!.os).toBe('linux');
  });

  it('excludes signatures, manifests, icons and desktop entries', () => {
    for (const n of [
      'StarStats_1.8.51_x64-setup.exe.sig',
      'StarStats_1.8.51_x64_en-US.msi.sig',
      'StarStats_1.8.51_amd64.AppImage.sig',
      'StarStats_1.8.51_amd64.deb.sig',
      'updater-manifest.json',
      'StarStats.png',
      'StarStats.desktop',
    ]) {
      expect(classifyAsset(asset(n)), n).toBeNull();
    }
  });

  it('keeps exactly the 4 real installer assets from a real release', () => {
    const kept = REAL_ASSETS.map((n) => classifyAsset(asset(n))).filter(Boolean);
    expect(kept).toHaveLength(4);
    expect(kept.map((a) => a!.kind).sort()).toEqual(['AppImage', 'deb', 'exe', 'msi']);
  });
});

describe('parseTrayVersion / isTrayTag', () => {
  it('strips the tray-v prefix', () => {
    expect(parseTrayVersion('tray-v1.8.51')).toBe('1.8.51');
    expect(parseTrayVersion('tray-v1.8.51-alpha.8')).toBe('1.8.51-alpha.8');
  });

  it('identifies tray tags and rejects platform tags', () => {
    expect(isTrayTag('tray-v1.8.51')).toBe(true);
    expect(isTrayTag('tray-v1.8.51-alpha.8')).toBe(true);
    expect(isTrayTag('v1.8.9')).toBe(false); // platform release
  });
});

function rel(over: Partial<RawRelease>): RawRelease {
  return {
    tag_name: 'tray-v1.0.0',
    name: 'tray-v1.0.0',
    body: '',
    draft: false,
    prerelease: false,
    published_at: '2026-07-20T12:00:00Z',
    html_url: 'https://example/r',
    assets: [asset('StarStats_1.0.0_x64-setup.exe')],
    ...over,
  };
}

describe('mapReleases', () => {
  it('drops drafts and non-tray (platform) releases', () => {
    const out = mapReleases([
      rel({ tag_name: 'tray-v1.8.51' }),
      rel({ tag_name: 'tray-v1.8.51-alpha.1', draft: true }), // draft
      rel({ tag_name: 'v1.8.9' }), // platform tag
    ]);
    expect(out.map((r) => r.tag)).toEqual(['tray-v1.8.51']);
  });
});

describe('selectReleases', () => {
  it('picks the newest stable and hides a superseded older prerelease', () => {
    const set = selectReleases(
      mapReleases([
        rel({ tag_name: 'tray-v1.8.51', prerelease: false, published_at: '2026-07-20T12:31:00Z' }),
        rel({ tag_name: 'tray-v1.8.51-alpha.8', prerelease: true, published_at: '2026-07-20T12:03:00Z' }),
      ]),
    );
    expect(set.stable?.version).toBe('1.8.51');
    expect(set.prerelease).toBeNull(); // alpha is older than the stable → not surfaced
  });

  it('surfaces a prerelease that is newer than the latest stable', () => {
    const set = selectReleases(
      mapReleases([
        rel({ tag_name: 'tray-v1.9.0-alpha.1', prerelease: true, published_at: '2026-07-21T09:00:00Z' }),
        rel({ tag_name: 'tray-v1.8.51', prerelease: false, published_at: '2026-07-20T12:31:00Z' }),
      ]),
    );
    expect(set.stable?.version).toBe('1.8.51');
    expect(set.prerelease?.version).toBe('1.9.0-alpha.1');
  });

  it('returns nulls when there are no tray releases', () => {
    expect(selectReleases([])).toEqual({ stable: null, prerelease: null });
  });
});
