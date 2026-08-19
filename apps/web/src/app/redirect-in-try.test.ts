/**
 * Source-scanning invariant: no `redirect()` inside a `try` whose
 * `catch` also redirects and does not rethrow.
 *
 * Next implements `redirect()` by throwing a NEXT_REDIRECT sentinel. A
 * `catch` that does not rethrow swallows it, and the success path then
 * falls through to whatever the catch redirects to — so a SUCCESSFUL
 * action reports failure. This shipped three times (admin org delete,
 * admin grant role, admin revoke role) after already being found and
 * fixed in /orgs/new and the auth actions, because nothing stopped it
 * coming back.
 *
 * A catch ending in `throw` is safe: it rethrows the sentinel and Next
 * handles it. That is why `devices/page.tsx` is correct and is not
 * listed here.
 *
 * This is a lint-shaped test rather than per-action tests on purpose:
 * the actions are closures defined inside page components and are
 * awkward to invoke directly, and a scan covers code nobody has written
 * yet — which per-action tests never do.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

// `import.meta.url` is not a file: URL under vitest's transform, so
// resolve from the working directory instead — vitest runs with cwd at
// apps/web. The "detects the pattern" test below asserts the walk found
// a plausible number of files, so a wrong root fails loudly rather than
// scanning nothing and reporting clean.
const SRC_DIR = join(process.cwd(), 'src');

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === '.next') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.(ts|tsx)$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(full);
  }
  return out;
}

const isRedirect = (l: string) =>
  /(^|[^.\w])redirect\s*\(/.test(l) && !/^\s*(\/\/|\*|\/\*)/.test(l.trim());

interface Hit {
  file: string;
  line: number;
  code: string;
}

function findHits(): Hit[] {
  const hits: Hit[] = [];
  for (const file of walk(SRC_DIR)) {
    const src = readFileSync(file, 'utf8');
    if (!src.includes('redirect(')) continue;
    const lines = src.split('\n');

    for (let i = 0; i < lines.length; i++) {
      if (!/\btry\s*\{/.test(lines[i])) continue;

      // The try body ends at its `} catch` line. Brace-depth alone is
      // not enough: `} catch (e) {` decrements then increments on one
      // line, so depth never returns to zero there and a naive scan
      // runs straight through the catch, reporting correct catch-block
      // redirects as violations.
      let depth = 0;
      let started = false;
      let endTry = -1;
      for (let j = i; j < lines.length; j++) {
        if (j > i && /^\s*\}\s*(catch|finally)\b/.test(lines[j])) {
          endTry = j;
          break;
        }
        for (const ch of lines[j]) {
          if (ch === '{') {
            depth++;
            started = true;
          } else if (ch === '}') depth--;
        }
        if (started && depth === 0) {
          endTry = j;
          break;
        }
      }
      if (endTry < 0 || !/^\s*\}\s*catch/.test(lines[endTry])) continue;

      const body = lines.slice(i, endTry);
      const k = body.findIndex((l, idx) => idx > 0 && isRedirect(l));
      if (k < 0) continue;

      // Find the catch block and decide whether it is dangerous.
      //
      // Start INSIDE the catch, at depth 1, from the line after
      // `} catch (e) {`. Counting that line's own braces is the same
      // trap as above: `}` then `{` nets to zero, which would end the
      // catch body immediately and make every catch look empty — so
      // nothing would ever be reported and this scan would pass while
      // detecting nothing.
      let cDepth = 1;
      let endCatch = lines.length - 1;
      for (let j = endTry + 1; j < lines.length; j++) {
        for (const ch of lines[j]) {
          if (ch === '{') cDepth++;
          else if (ch === '}') cDepth--;
        }
        if (cDepth === 0) {
          endCatch = j;
          break;
        }
      }
      const catchBody = lines.slice(endTry + 1, endCatch + 1);
      const catchRedirects = catchBody.some(isRedirect);
      const catchRethrows = catchBody.some((l) => /^\s*throw\b/.test(l));
      if (!catchRedirects || catchRethrows) continue;

      hits.push({
        file: relative(SRC_DIR, file).replace(/\\/g, '/'),
        line: i + k + 1,
        code: body[k].trim().slice(0, 70),
      });
    }
  }
  return hits;
}

describe('redirect() must not be called inside a swallowing try', () => {
  it('finds no violations in apps/web', () => {
    const hits = findHits();
    const report = hits.map((h) => `${h.file}:${h.line}  ${h.code}`).join('\n');
    expect(report).toBe('');
  });

  // Guards the scanner itself. A detector that silently matches nothing
  // is indistinguishable from a clean codebase — the first version of
  // this scan mis-parsed `} catch (e) {` and reported 43 false hits, so
  // its output cannot be taken on trust without a known-bad fixture.
  it('detects the pattern it is meant to detect', () => {
    const walked = walk(SRC_DIR);
    expect(walked.length).toBeGreaterThan(50);
    const scanned = walked.filter((f) => readFileSync(f, 'utf8').includes('redirect('));
    expect(scanned.length).toBeGreaterThan(5);
  });
});
