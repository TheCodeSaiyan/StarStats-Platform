/**
 * Tests for the Settings two-pane scroll-spy sidebar (M2).
 *
 * The nav is the one moving part of the restructure — the section
 * content is unchanged server markup. These tests pin (1) the landmark +
 * a11y contract, (2) that every category renders, (3) that each item
 * links to the exact anchor the server page/actions expect, and (4) a
 * representative section resolves through its nav link.
 */

import React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { SettingsNav } from './SettingsNav';
import { SETTINGS_NAV } from './settings-nav-config';

// jsdom has no IntersectionObserver — stub a no-op so the scroll-spy
// effect runs without throwing. We assert the rendered link contract,
// not the observer callback (that needs a real layout engine).
beforeEach(() => {
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
      takeRecords() {
        return [];
      }
    },
  );
});

describe('SettingsNav', () => {
  it('renders one Settings navigation landmark', () => {
    render(<SettingsNav />);
    const nav = screen.getByRole('navigation', { name: 'Settings' });
    expect(nav).toBeInTheDocument();
  });

  it('renders every category heading', () => {
    render(<SettingsNav />);
    for (const category of SETTINGS_NAV) {
      expect(screen.getByText(category.label)).toBeInTheDocument();
    }
  });

  // Representative expected anchors, including the behaviour-load-bearing
  // ones the server actions redirect to (#security, #danger, #rsi).
  it.each([
    ['Appearance', '#theme'],
    ['Account info', '#account-info'],
    ['Email verification', '#verification'],
    ['RSI handle', '#rsi'],
    ['Device sync', '#hangar'],
    ['Sign-in email', '#email'],
    ['Password', '#password'],
    ['Two-factor', '#security'],
    ['Delete account', '#danger'],
  ])('links %s to %s', (label, anchor) => {
    render(<SettingsNav />);
    const link = screen.getByRole('link', { name: label });
    expect(link).toHaveAttribute('href', anchor);
  });

  it('exposes exactly one link per configured nav item', () => {
    render(<SettingsNav />);
    const configuredCount = SETTINGS_NAV.reduce(
      (sum, category) => sum + category.items.length,
      0,
    );
    expect(screen.getAllByRole('link')).toHaveLength(configuredCount);
  });

  it('marks the first item aria-current by default', () => {
    render(<SettingsNav />);
    const first = screen.getByRole('link', { name: 'Appearance' });
    expect(first).toHaveAttribute('aria-current', 'true');
  });

  it('resolves a representative section through its nav link', () => {
    // Mount a stand-in section next to the nav — a representative slice
    // of the two-pane content — and confirm the link points at it.
    render(
      <div>
        <SettingsNav />
        <section id="theme">
          <h2>Appearance</h2>
        </section>
      </div>,
    );
    const nav = screen.getByRole('navigation', { name: 'Settings' });
    const link = within(nav).getByRole('link', { name: 'Appearance' });
    const targetId = link.getAttribute('href')?.slice(1);
    expect(targetId).toBe('theme');
    expect(document.getElementById(targetId ?? '')).not.toBeNull();
  });
});
