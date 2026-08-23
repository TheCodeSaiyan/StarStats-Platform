'use client';

import React from 'react';

/**
 * Console shell — the index rail plus work area used by operator surfaces.
 * Pair with `<Projection surface="console">`, which strips the volume's
 * ambience. Groups are rendered verbatim in the order given, so the rail can
 * mirror the app's real route tree rather than an invented one.
 *
 * `renderLink` is the same escape hatch `ChromeBar` carries: a plain `<a>` is a
 * full document load in a Next app, and the console is the surface an operator
 * clicks through most.
 */
export interface ConsoleItem {
  id: string;
  label: string;
  href?: string;
  /** Badge — e.g. a pending count. Omit rather than passing 0. */
  count?: number | null;
}

export interface ConsoleGroup {
  title: string;
  items: ConsoleItem[];
}

export function Console({
  groups = [],
  active,
  onSelect,
  renderLink,
  children,
}: {
  groups?: ConsoleGroup[];
  active?: string;
  onSelect?: (id: string) => void;
  /** Same escape hatch as `ChromeBar` — a plain `<a>` is a full page load. */
  renderLink?: (props: {
    href: string;
    children: React.ReactNode;
    'aria-current'?: 'page';
  }) => React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="hp-console">
      <nav className="hp-cnav" aria-label="Console sections">
        {groups.map((g) => (
          <React.Fragment key={g.title}>
            <div className="grp">{g.title}</div>
            {g.items.map((it) => {
              const body = (
                <>
                  <span>{it.label}</span>
                  {it.count != null ? <b>{it.count}</b> : null}
                </>
              );
              const current = it.id === active ? ('page' as const) : undefined;
              if (it.href && renderLink) {
                return (
                  <React.Fragment key={it.id}>
                    {renderLink({
                      href: it.href,
                      'aria-current': current,
                      children: body,
                    })}
                  </React.Fragment>
                );
              }
              return (
                <a
                  key={it.id}
                  href={it.href || `#${it.id}`}
                  aria-current={current}
                  onClick={(e) => {
                    if (onSelect) {
                      e.preventDefault();
                      onSelect(it.id);
                    }
                  }}
                >
                  {body}
                </a>
              );
            })}
          </React.Fragment>
        ))}
      </nav>
      <div className="hp-cwork">{children}</div>
    </div>
  );
}
