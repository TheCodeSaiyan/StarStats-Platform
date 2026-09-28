import { describe, expect, it } from 'vitest';
import { checkLink, linkify } from './links';

const verdict = (href: string) => checkLink(href)?.verdict;

describe('link warnings', () => {
  it('trusts the real sites and their subdomains', () => {
    expect(verdict('https://robertsspaceindustries.com/pledge')).toBe('trusted');
    expect(verdict('https://support.robertsspaceindustries.com/x')).toBe('trusted');
    expect(verdict('https://starstats.app/lfg')).toBe('trusted');
  });

  it('shows everything else as unknown, with its real domain', () => {
    const c = checkLink('https://www.youtube.com/watch?v=x');
    expect(c?.verdict).toBe('unknown');
    expect(c?.host).toBe('www.youtube.com');
  });

  it('warns about the lookalike login pages this community sees', () => {
    for (const href of [
      'https://robertspaceindustries.com/login', // one letter off
      'https://robertsspaceindustries.com.account-verify.xyz/login', // trusted name as a subdomain
      'https://rsi-robertsspaceindustries-giveaway.com',
      'https://xn--robertsspaceindustres-9zb.com',
    ]) {
      expect(verdict(href), href).toBe('warn');
    }
  });

  it('warns about shorteners, IPs, plain http, downloads and hidden logins', () => {
    expect(checkLink('https://bit.ly/abc')?.reasons.join()).toContain('shortened');
    expect(checkLink('https://203.0.113.5/x')?.reasons.join()).toContain('IP');
    expect(checkLink('http://example.com')?.reasons.join()).toContain('https');
    expect(checkLink('https://example.com/free-ship.exe')?.reasons.join()).toContain('downloads');
    expect(checkLink('https://robertsspaceindustries.com@evil.example/')?.verdict).toBe('warn');
  });

  it('ignores anything that is not an http(s) link', () => {
    expect(checkLink('javascript:alert(1)')).toBeNull();
    expect(checkLink('not a url')).toBeNull();
  });
});

describe('linkify', () => {
  it('splits text and links, leaving sentence punctuation outside', () => {
    const s = linkify('see https://starstats.app/lfg. then ok');
    expect(s.map((x) => x.kind)).toEqual(['text', 'link', 'text']);
    expect(s[1]).toMatchObject({ text: 'https://starstats.app/lfg', verdict: 'trusted' });
    expect(s[2]).toEqual({ kind: 'text', text: '. then ok' });
  });

  it('leaves text without links alone', () => {
    expect(linkify('o7 pilots')).toEqual([{ kind: 'text', text: 'o7 pilots' }]);
  });
});
