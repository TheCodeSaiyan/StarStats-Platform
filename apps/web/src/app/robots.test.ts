import { afterEach, describe, expect, it } from 'vitest';

import robots from './robots';

const ORIGINAL = { ...process.env };

afterEach(() => {
  process.env = { ...ORIGINAL };
});

describe('robots.txt', () => {
  it('disallows everything on a noindex deployment', () => {
    process.env.STARSTATS_NOINDEX = '1';
    const result = robots();
    expect(result.rules).toEqual([{ userAgent: '*', disallow: '/' }]);
    // A blanket Disallow must not be paired with an Allow — a crawler
    // resolves the most specific match, so a stray `allow: '/'`
    // alongside it re-opens the whole site.
    const rules = Array.isArray(result.rules) ? result.rules : [result.rules];
    expect(rules.every((r) => r.allow === undefined)).toBe(true);
  });

  it('allows crawling on production', () => {
    delete process.env.STARSTATS_NOINDEX;
    process.env.STARSTATS_SITE_URL = 'https://starstats.app';
    const result = robots();
    const rules = Array.isArray(result.rules) ? result.rules : [result.rules];
    expect(rules[0]?.allow).toBe('/');
    expect(rules[0]?.disallow).toContain('/admin/');
    expect(result.host).toBe('https://starstats.app');
  });

  it('treats any value other than "1" as production', () => {
    // Guards the "STARSTATS_NOINDEX=false" foot-gun — a non-empty
    // string is truthy, so a `Boolean(env)` check would silently
    // noindex production.
    process.env.STARSTATS_NOINDEX = 'false';
    const rules = robots().rules;
    expect(Array.isArray(rules) ? rules[0]?.allow : rules.allow).toBe('/');
  });
});
