'use client';

import React, { useState, useEffect, useRef } from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { usePathname } from 'next/navigation';

import { CompassStar } from '@/components/CompassStar';

/**
 * Top chrome for every signed-out page. Full-bleed wrapper —
 * no max-width on the <header> — so the border-bottom spans the
 * viewport. Rendered from the signed-out branch of app/layout.tsx.
 */
export function MarketingNav() {
  // usePathname returns null during static rendering / before
  // hydration; the active-link styling kicks in once the client
  // boots, which is fine — server-rendered HTML is just inactive.
  const pathname = usePathname();

  // Mobile dropdown state. The link row collapses behind a hamburger
  // at ≤640px (see `.ss-mnav-toggle` / `.ss-mnav-links` in
  // starstats-tokens.css). Close on every soft-navigation so the panel
  // doesn't stay open over the next page.
  const [menuOpen, setMenuOpen] = useState(false);
  const toggleRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setMenuOpen(false);
  }, [pathname]);

  // Escape closes the mobile dropdown and returns focus to the
  // hamburger toggle (M-W10 — the drawer had no keyboard dismiss).
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setMenuOpen(false);
        toggleRef.current?.focus();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [menuOpen]);

  const linkStyle = (href: string): React.CSSProperties => {
    const isActive = pathname === href;
    return {
      color: isActive ? 'var(--fg)' : 'inherit',
      textDecoration: 'none',
    };
  };

  const activeProps = (href: string) =>
    pathname === href ? { 'aria-current': 'page' as const } : {};

  return (
    <header
      className="ss-marketing-nav"
      style={{
        display: 'flex',
        alignItems: 'center',
        padding: '20px 48px',
        borderBottom: '1px solid var(--border)',
        background: 'var(--bg)',
        position: 'relative',
        zIndex: 1,
      }}
    >
      <Link
        href="/"
        className="ss-mark"
        style={{ textDecoration: 'none', color: 'inherit' }}
        {...activeProps('/')}
      >
        <span className="ss-mark-glyph" style={{ color: 'var(--accent)' }}>
          <CompassStar size={14} />
        </span>
        <span>STARSTATS</span>
      </Link>
      <span
        className="ss-eyebrow"
        style={{ marginLeft: 12, color: 'var(--fg-dim)', fontWeight: 500 }}
      >
        Community telemetry · Unofficial
      </span>
      <div style={{ flex: 1 }} />
      <button
        ref={toggleRef}
        type="button"
        className="ss-mnav-toggle"
        aria-label={menuOpen ? 'Close menu' : 'Open menu'}
        aria-expanded={menuOpen}
        aria-controls="ss-marketing-menu"
        onClick={() => setMenuOpen((o) => !o)}
      >
        <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
          {menuOpen ? (
            <path
              d="M4 4l10 10M14 4L4 14"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          ) : (
            <path
              d="M3 5h12M3 9h12M3 13h12"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          )}
        </svg>
      </button>
      <nav
        id="ss-marketing-menu"
        className="ss-mnav-links"
        data-open={menuOpen ? 'true' : undefined}
        style={{
          display: 'flex',
          gap: 28,
          alignItems: 'center',
          color: 'var(--fg-muted)',
          fontSize: 13,
        }}
      >
        <Link
          href="/features"
          style={linkStyle('/features')}
          {...activeProps('/features')}
        >
          Features
        </Link>
        <Link
          href={'/star-platform' as Route}
          style={linkStyle('/star-platform')}
          {...activeProps('/star-platform')}
        >
          StarPlatform
        </Link>
        <Link
          href={'/docs' as Route}
          style={linkStyle('/docs')}
          {...activeProps('/docs')}
        >
          Docs
        </Link>
        <Link
          href={'/guides' as Route}
          style={linkStyle('/guides')}
          {...activeProps('/guides')}
        >
          Guides
        </Link>
        <Link
          href={'/trust' as Route}
          style={linkStyle('/trust')}
          {...activeProps('/trust')}
        >
          Trust
        </Link>
        <Link
          href="/privacy"
          style={linkStyle('/privacy')}
          {...activeProps('/privacy')}
        >
          Privacy
        </Link>
        <Link
          href={'/terms' as Route}
          style={linkStyle('/terms')}
          {...activeProps('/terms')}
        >
          Terms
        </Link>
        <Link
          href="/downloads"
          style={linkStyle('/downloads')}
          {...activeProps('/downloads')}
        >
          Download
        </Link>
        <a
          href="https://github.com/TheCodeSaiyan/StarStats-Platform"
          target="_blank"
          rel="noreferrer noopener"
          style={{ color: 'inherit', textDecoration: 'none' }}
          title="View source on GitHub"
        >
          GitHub
        </a>
        <Link href="/auth/login" className="ss-btn ss-btn--ghost">
          Sign in
        </Link>
        <Link href="/auth/signup" className="ss-btn ss-btn--primary">
          Get started →
        </Link>
      </nav>
    </header>
  );
}
