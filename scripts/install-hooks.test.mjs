// node --test scripts/install-hooks.test.mjs
//
// Real git repositories in a temp directory: the point of this installer is
// what it does NOT touch (core.hooksPath, other hooks), and only a real
// repository shows that.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { install, MARKER } = require('./install-hooks.cjs');

function repo() {
  const dir = mkdtempSync(path.join(tmpdir(), 'hooks-'));
  execFileSync('git', ['init', '-q', dir]);
  return dir;
}
const hook = (dir, name) => path.join(dir, '.git', 'hooks', name);
const hooksPath = (dir) => {
  try {
    return execFileSync('git', ['config', 'core.hooksPath'], { cwd: dir, encoding: 'utf8' }).trim();
  } catch {
    return null;
  }
};

test('installs commit-msg and leaves every other hook and core.hooksPath alone', () => {
  const dir = repo();
  try {
    const protect = '#!/bin/sh\n# loadout-managed-hook\nexit 0\n';
    writeFileSync(hook(dir, 'pre-commit'), protect);
    assert.equal(install(dir), 'installed');
    assert.ok(readFileSync(hook(dir, 'commit-msg'), 'utf8').includes(MARKER));
    assert.equal(readFileSync(hook(dir, 'pre-commit'), 'utf8'), protect, 'pre-commit untouched');
    assert.equal(hooksPath(dir), null, 'core.hooksPath never set');
    assert.equal(install(dir), 'updated', 're-running refreshes our own hook');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a commit-msg hook that is not ours is kept', () => {
  const dir = repo();
  try {
    const theirs = '#!/bin/sh\necho theirs\n';
    writeFileSync(hook(dir, 'commit-msg'), theirs);
    assert.equal(install(dir), 'kept-foreign');
    assert.equal(readFileSync(hook(dir, 'commit-msg'), 'utf8'), theirs);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('outside a repository it does nothing', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'nogit-'));
  try {
    assert.equal(install(dir), 'not-a-repo');
    assert.equal(existsSync(path.join(dir, '.git')), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
