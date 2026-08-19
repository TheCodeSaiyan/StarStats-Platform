'use client';

import { useState, useRef, useEffect } from 'react';
import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { SupporterChip } from '@/components/SupporterChip';
import type { SupporterStatusDto } from '@/lib/api';

interface MenuLink {
  label: string;
  href: Route;
  badge?: number;
}

/**
 * The @handle ▾ account menu in the TopBar. Holds the secondary
 * surfaces demoted from the 3-pillar rail (profile, devices, KB,
 * sharing, submissions, settings, support, admin) plus sign-out.
 * Client island — the surrounding TopBar stays a server component.
 */
export function AccountMenu({
  handle,
  staffRoles,
  inboundShareCount = 0,
  supporter = null,
}: {
  handle: string | null;
  staffRoles: string[];
  inboundShareCount?: number;
  supporter?: SupporterStatusDto | null;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      // Escape closes and restores focus to the trigger (M-W10).
      if (e.key === 'Escape') {
        setOpen(false);
        btnRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', onClick);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onClick);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const profileHref = (handle ? `/u/${encodeURIComponent(handle)}` : '/settings') as Route;
  const links: MenuLink[] = [
    { label: 'My public profile', href: profileHref },
    { label: 'Devices', href: '/devices' as Route },
    { label: 'Knowledge base', href: '/kb' as Route },
    {
      label: 'Shared with me',
      href: '/sharing' as Route,
      badge: inboundShareCount > 0 ? inboundShareCount : undefined,
    },
    { label: 'Submissions', href: '/submissions' as Route },
    { label: 'Settings', href: '/settings' as Route },
    { label: 'Support', href: '/support' as Route },
    ...(staffRoles.length > 0 ? [{ label: 'Admin', href: '/admin' as Route }] : []),
  ];

  const itemStyle: React.CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    gap: 8,
    padding: '8px 12px',
    fontSize: 13,
    color: 'var(--fg)',
    textDecoration: 'none',
  };

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <button
        ref={btnRef}
        type="button"
        // Disclosure, not a menu: this popover is a plain list of
        // navigation links with no arrow-key roving-focus model, so
        // `role="menu"`/`menuitem` (which promise that model) were
        // dropped in favour of aria-expanded + aria-controls (M-W10).
        aria-expanded={open}
        aria-controls="account-menu"
        onClick={() => setOpen((o) => !o)}
        className="mono"
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          background: 'transparent',
          border: '1px solid var(--border)',
          borderRadius: 'var(--r-sm)',
          padding: '5px 10px',
          color: 'var(--fg-muted)',
          fontSize: 13,
          cursor: 'pointer',
        }}
      >
        <span>{handle ? `@${handle}` : 'Account'}</span>
        <SupporterChip status={supporter} size="sm" />
        {inboundShareCount > 0 && (
          <span className="ss-rail-badge" aria-label={`${inboundShareCount} new`}>
            {inboundShareCount > 99 ? '99+' : inboundShareCount}
          </span>
        )}
        <span aria-hidden="true">▾</span>
      </button>
      {open && (
        <nav
          id="account-menu"
          aria-label="Account"
          style={{
            position: 'absolute',
            right: 0,
            top: 'calc(100% + 6px)',
            minWidth: 200,
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--r-sm)',
            boxShadow: '0 8px 24px rgba(0,0,0,0.35)',
            padding: '4px 0',
            zIndex: 50,
          }}
        >
          {links.map((l) => (
            <Link
              key={l.href + l.label}
              href={l.href}
              onClick={() => setOpen(false)}
              style={itemStyle}
            >
              <span style={{ flex: 1 }}>{l.label}</span>
              {l.badge !== undefined && (
                <span className="ss-rail-badge" aria-label={`${l.badge} new`}>
                  {l.badge > 99 ? '99+' : l.badge}
                </span>
              )}
            </Link>
          ))}
          <a href="/auth/logout" style={{ ...itemStyle, borderTop: '1px solid var(--border)' }}>
            Sign out
          </a>
        </nav>
      )}
    </div>
  );
}
