import React from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

let pathname: string | null = '/';
vi.mock('next/navigation', () => ({
  usePathname: () => pathname,
}));

import { MarketingNav } from './MarketingNav';

afterEach(() => {
  cleanup();
  pathname = '/';
});

describe('MarketingNav', () => {
  it('renders as a full-bleed <header> with no inline maxWidth constraint', () => {
    const { container } = render(<MarketingNav />);
    const header = container.querySelector('header');
    expect(header).not.toBeNull();
    // The outer element must carry no max-width — the full-bleed
    // wrapper / inner container pattern is what keeps the
    // border-bottom from collapsing into a centered stripe.
    expect(header?.style.maxWidth).toBe('');
  });

  it('links the brand mark to the landing page', () => {
    render(<MarketingNav />);
    const brand = screen.getByRole('link', { name: /starstats/i });
    expect(brand).toHaveAttribute('href', '/');
  });

  it('exposes the primary signup CTA', () => {
    render(<MarketingNav />);
    const cta = screen.getByRole('link', { name: /get started/i });
    expect(cta).toHaveAttribute('href', '/auth/signup');
  });

  it('marks the active page with aria-current and fg colour', () => {
    pathname = '/features';
    render(<MarketingNav />);
    const features = screen.getByRole('link', { name: /^features$/i });
    expect(features).toHaveAttribute('aria-current', 'page');
    expect(features.getAttribute('style')).toContain('color: var(--fg)');

    const privacy = screen.getByRole('link', { name: /^privacy$/i });
    expect(privacy).not.toHaveAttribute('aria-current');
  });

  it('leaves non-pathname links inactive when on /privacy', () => {
    pathname = '/privacy';
    render(<MarketingNav />);
    const privacy = screen.getByRole('link', { name: /^privacy$/i });
    expect(privacy).toHaveAttribute('aria-current', 'page');

    const features = screen.getByRole('link', { name: /^features$/i });
    expect(features).not.toHaveAttribute('aria-current');
  });

  it('toggles the mobile menu via the hamburger button', () => {
    const { container } = render(<MarketingNav />);
    const toggle = screen.getByRole('button', { name: /open menu/i });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');

    // Collapsed: the link panel carries no data-open (CSS hides it ≤640px).
    const nav = container.querySelector('#ss-marketing-menu');
    expect(nav).not.toBeNull();
    expect(nav?.getAttribute('data-open')).toBeNull();

    fireEvent.click(toggle);

    // Open: button flips to a labelled close affordance and the panel
    // exposes data-open="true" for the dropdown CSS.
    const closeToggle = screen.getByRole('button', { name: /close menu/i });
    expect(closeToggle).toHaveAttribute('aria-expanded', 'true');
    expect(nav?.getAttribute('data-open')).toBe('true');
  });

  it('exposes the StarPlatform link', () => {
    render(<MarketingNav />);
    const starPlatform = screen.getByRole('link', { name: /^starplatform$/i });
    expect(starPlatform).toHaveAttribute('href', '/star-platform');
  });

  it('exposes the Terms link alongside Privacy', () => {
    render(<MarketingNav />);
    const terms = screen.getByRole('link', { name: /^terms$/i });
    expect(terms).toHaveAttribute('href', '/terms');
    // Both legal pages stay reachable from the marketing nav — signup
    // fine-print points at /terms, so a nav that drops it strands the
    // one document users are told they agreed to.
    expect(screen.getByRole('link', { name: /^privacy$/i })).toHaveAttribute(
      'href',
      '/privacy',
    );
  });

  it('exposes the Trust link', () => {
    render(<MarketingNav />);
    // /trust is the page a suspicious visitor is sent to. If it is not in
    // the nav it may as well not exist — nobody guesses the URL.
    expect(screen.getByRole('link', { name: /^trust$/i })).toHaveAttribute(
      'href',
      '/trust',
    );
  });

  it('marks Trust active when on /trust', () => {
    pathname = '/trust';
    render(<MarketingNav />);
    expect(screen.getByRole('link', { name: /^trust$/i })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(screen.getByRole('link', { name: /^privacy$/i })).not.toHaveAttribute(
      'aria-current',
    );
  });

  it('exposes the Docs link', () => {
    render(<MarketingNav />);
    // Onboarding has four steps a stranger will not guess. /docs is where
    // they are written down; a nav that drops it recreates the silent
    // drop-off it exists to fix.
    expect(screen.getByRole('link', { name: /^docs$/i })).toHaveAttribute(
      'href',
      '/docs',
    );
  });

  it('marks Docs active when on /docs', () => {
    pathname = '/docs';
    render(<MarketingNav />);
    expect(screen.getByRole('link', { name: /^docs$/i })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(screen.getByRole('link', { name: /^trust$/i })).not.toHaveAttribute(
      'aria-current',
    );
  });

  it('exposes the Guides link alongside Docs', () => {
    render(<MarketingNav />);
    // /docs is "make it work"; /guides is "now use it". Both sit in the
    // nav because a visitor with nothing installed and a visitor whose
    // tiles are empty need different pages, and neither guesses a URL.
    expect(screen.getByRole('link', { name: /^guides$/i })).toHaveAttribute(
      'href',
      '/guides',
    );
    expect(screen.getByRole('link', { name: /^docs$/i })).toHaveAttribute(
      'href',
      '/docs',
    );
  });

  it('marks Guides active on /guides without also marking Docs', () => {
    pathname = '/guides';
    render(<MarketingNav />);
    expect(screen.getByRole('link', { name: /^guides$/i })).toHaveAttribute(
      'aria-current',
      'page',
    );
    // The sections share no path prefix — /guides is top-level precisely
    // so it is not a child of /docs. If someone re-nests it under /docs,
    // this is the assertion that should fail.
    expect(screen.getByRole('link', { name: /^docs$/i })).not.toHaveAttribute(
      'aria-current',
    );
  });

  it('marks Terms active when on /terms', () => {
    pathname = '/terms';
    render(<MarketingNav />);
    expect(screen.getByRole('link', { name: /^terms$/i })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(
      screen.getByRole('link', { name: /^privacy$/i }),
    ).not.toHaveAttribute('aria-current');
  });
});
