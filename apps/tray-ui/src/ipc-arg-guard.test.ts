/**
 * Tripwire guard for the Tauri IPC arg-name invariant (C1 fix, 2026-07-09).
 *
 * Two failure modes this locks down:
 *  (a) A `#[tauri::command]` in the Rust client that forgets
 *      `rename_all = "snake_case"`. Under tauri-macros >=2.6 a bare
 *      command defaults its IPC keys to camelCase, so a snake_case TS
 *      payload binds silently to `None` (the exact bug C1 fixed).
 *  (b) A TS `invoke(...)` wrapper in api.ts that sends a camelCase key.
 *      With the Rust side on snake_case, a camelCase key never binds.
 *
 * Deliberately dumb + grep-based — it is a tripwire, not a parser. It
 * reads the source files off disk relative to this test's location so it
 * survives file moves within the workspace.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url)); // apps/tray-ui/src
const COMMANDS_RS = resolve(
  here,
  '../../../crates/starstats-client/src/commands.rs',
);
const API_TS = resolve(here, './api.ts');

describe('Tauri IPC arg-name invariant', () => {
  it('every #[tauri::command] in commands.rs declares rename_all = "snake_case"', () => {
    const rust = readFileSync(COMMANDS_RS, 'utf8');
    // Only column-0 attributes are real commands; `^` (multiline) skips
    // any `#[tauri::command]` that appears indented inside a doc comment.
    const attrs = rust.match(/^#\[tauri::command[^\]]*\]/gm) ?? [];
    expect(attrs.length).toBeGreaterThan(0);
    const offenders = attrs.filter(
      (a) => !a.includes('rename_all = "snake_case"'),
    );
    expect(
      offenders,
      `bare #[tauri::command] found (needs rename_all = "snake_case"): ${offenders.join(
        ', ',
      )}`,
    ).toEqual([]);
  });

  it('every invoke() arg-object key in api.ts is lowercase snake_case', () => {
    const ts = readFileSync(API_TS, 'utf8');
    // Capture the object literal passed as the 2nd arg to invoke(...).
    // The optional <...> generic (which may itself contain `{ }`) is
    // consumed first; the arg object is a single flat `{ ... }` (no
    // nested braces in any wrapper), so `[^}]*` stops at its close.
    const callRe = /invoke\s*(?:<[^>]*>)?\s*\(\s*[^,]*?,\s*(\{[^}]*\})/gs;
    const badKeys: string[] = [];
    let m: RegExpExecArray | null;
    while ((m = callRe.exec(ts)) !== null) {
      const body = m[1].slice(1, -1); // strip the braces
      for (const seg of body.split(',')) {
        const raw = seg.trim();
        if (!raw) continue; // trailing comma
        // `key: value` -> key ; shorthand `key` -> key
        const key = raw.split(':')[0].trim();
        if (!/^[a-z0-9_]+$/.test(key)) badKeys.push(key);
      }
    }
    expect(
      badKeys,
      `non-snake_case invoke() keys in api.ts: ${badKeys.join(', ')}`,
    ).toEqual([]);
  });
});
