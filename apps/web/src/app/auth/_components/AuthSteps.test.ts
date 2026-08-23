import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import {
  AUTH_STEP_ROUTES,
  ENTRY_POINTS,
  authStepFor,
} from './AuthSteps';

const AUTH_DIR = path.join(process.cwd(), 'src/app/auth');

/** Every `page.tsx` under `/auth`, as the route it serves. */
function authRoutes(): string[] {
  const out: string[] = [];
  const walk = (dir: string, url: string) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) {
        if (e.name.startsWith('_')) continue;
        walk(path.join(dir, e.name), `${url}/${e.name}`);
      } else if (e.name === 'page.tsx') {
        out.push(url);
      }
    }
  };
  walk(AUTH_DIR, '/auth');
  return out.sort();
}

describe('auth step naming', () => {
  it('names every route in the segment', () => {
    // The whole point of the map is that no auth route falls back to the
    // generic "Access" — that was the state this replaced, where all nine
    // routes shared one header. A new page added without a title would
    // silently rejoin it, so the filesystem is the source of truth here.
    const missing = authRoutes().filter(
      (r) => !AUTH_STEP_ROUTES.includes(r),
    );
    expect(missing).toEqual([]);
  });

  it('does not name a route that no longer exists', () => {
    const routes = authRoutes();
    const dead = AUTH_STEP_ROUTES.filter((r) => !routes.includes(r));
    expect(dead).toEqual([]);
  });

  it('gives every step a distinct heading and a subtitle', () => {
    for (const r of AUTH_STEP_ROUTES) {
      const [title, ctx] = authStepFor(r);
      expect(title, r).not.toBe('Access');
      expect(ctx.length, r).toBeGreaterThan(8);
    }
  });

  it('only offers entry points that need no token', () => {
    // Listing `reset-password`, `verify`, `email-change`, `totp-verify` or the
    // magic-link redeem would send a reader to an error state and call it a
    // destination — they are all reached WITH a token.
    const TOKEN_GATED = [
      '/auth/reset-password',
      '/auth/verify',
      '/auth/email-change',
      '/auth/totp-verify',
      '/auth/magic-link/redeem',
      '/auth/logout',
    ];
    for (const [href] of ENTRY_POINTS) {
      expect(TOKEN_GATED, href).not.toContain(href);
      expect(authRoutes(), href).toContain(href);
    }
  });
});

describe('auth bodies are calibrated', () => {
  it('defines no inline style objects', () => {
    // Nine pages used to carry 50 `React.CSSProperties` blocks between them —
    // a 420px card, a 28px/600 title, a filled explainer box. Inline styles
    // are the one thing the projection's redraw cannot reach, so each of those
    // was a page rendering in the old voice inside the new frame. If one comes
    // back, it will look fine in review and wrong on screen.
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) walk(p);
        else if (e.name.endsWith('.tsx')) {
          const src = fs.readFileSync(p, 'utf8');
          if (src.includes('React.CSSProperties = {')) {
            offenders.push(path.relative(AUTH_DIR, p));
          }
        }
      }
    };
    walk(AUTH_DIR);
    expect(offenders).toEqual([]);
  });
});
