// Install the commit-msg hook that checks subjects against
// .github/commit-rules.json. Run by the root `prepare` script at
// `pnpm install`, which wraps the require in try/catch so an image build
// (no .git, no scripts/) is never broken by it.
//
// It writes ONE file, <git common dir>/hooks/commit-msg, and nothing else.
// It deliberately does NOT set core.hooksPath: that would make git ignore
// .git/hooks entirely, and this repository relies on a hook there (the
// Loadout pre-commit hook that keeps agent tooling out of commits in a
// public repo). An earlier version set core.hooksPath and silently switched
// that protection off.
//
// A commit-msg hook that is not ours is left alone: we only write when the
// file is missing or carries our marker.

'use strict';

const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const MARKER = '# starstats-commit-rules';

const SHIM = `#!/bin/sh
${MARKER}
# Installed by scripts/install-hooks.cjs (pnpm install). Checks the subject
# against .github/commit-rules.json; CI runs the same check, so skipping this
# with --no-verify only moves the failure to the pull request.
root="$(git rev-parse --show-toplevel)"
if [ -f "$root/scripts/check-commits.mjs" ]; then
  exec node "$root/scripts/check-commits.mjs" --message-file "$1"
fi
`;

/**
 * @param {string} cwd a directory inside the work tree
 * @returns {'installed' | 'updated' | 'kept-foreign' | 'not-a-repo'}
 */
function install(cwd) {
  let common;
  try {
    common = execFileSync('git', ['rev-parse', '--git-common-dir'], {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  } catch {
    return 'not-a-repo';
  }
  const hooks = path.resolve(cwd, common, 'hooks');
  const file = path.join(hooks, 'commit-msg');
  let existing = null;
  try {
    existing = fs.readFileSync(file, 'utf8');
  } catch {
    // missing: install
  }
  if (existing !== null && !existing.includes(MARKER)) return 'kept-foreign';
  fs.mkdirSync(hooks, { recursive: true });
  fs.writeFileSync(file, SHIM, { mode: 0o755 });
  fs.chmodSync(file, 0o755);
  return existing === null ? 'installed' : 'updated';
}

/** Entry point for `prepare`. Never throws. */
function main() {
  try {
    const result = install(process.cwd());
    if (result === 'kept-foreign') {
      console.warn('[hooks] a commit-msg hook that is not ours exists; left it alone');
    } else if (result !== 'not-a-repo') {
      console.log(`[hooks] commit-msg rules hook ${result}`);
    }
  } catch (e) {
    // Never fail an install over this; CI enforces the same rules.
    console.warn(`[hooks] not installed: ${e.message}`);
  }
}

module.exports = { install, main, MARKER };

if (require.main === module) main();
