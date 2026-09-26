#!/usr/bin/env node
// Enforce .github/commit-rules.json on commit subjects.
//
//   node scripts/check-commits.mjs --message-file .git/COMMIT_EDITMSG   (commit-msg hook)
//   node scripts/check-commits.mjs --range origin/next..HEAD            (CI, pull requests)
//
// Release notes are generated from these subjects, so a subject that
// breaks the rules is a release note that reads badly or not at all.
// The hook gives fast feedback but can be skipped with --no-verify; the
// CI job running the same check is the authority.
//
// Exit codes: 0 all passed, 1 at least one subject failed, 2 bad usage.

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { checkSubject, loadRules } from './lib/commit-rules.mjs';

const RULES = fileURLToPath(new URL('../.github/commit-rules.json', import.meta.url));

function usage(code) {
  console.error(
    'usage: check-commits.mjs --message-file <path> | --range <from>..<to>',
  );
  process.exit(code);
}

const args = process.argv.slice(2);
let messageFile = null;
let range = null;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--message-file') messageFile = args[++i];
  else if (args[i] === '--range') range = args[++i];
  else if (args[i] === '--help' || args[i] === '-h') usage(0);
  else usage(2);
}
if (!messageFile === !range) usage(2);

const rules = loadRules(RULES);

/** @type {{ref: string, subject: string, author: string}[]} */
let commits;
if (messageFile) {
  const raw = readFileSync(messageFile, 'utf8');
  // The first non-comment line is the subject git will record.
  const subject =
    raw
      .split(/\r?\n/)
      .find((l) => l.trim() !== '' && !l.startsWith('#')) ?? '';
  const author = process.env.GIT_AUTHOR_NAME ?? gitOr(['config', 'user.name'], '');
  commits = [{ ref: 'this commit', subject, author }];
} else {
  const out = gitOr(['log', '--no-merges', '--format=%h%x00%an%x00%s', range], null);
  if (out === null) {
    console.error(`[commit-rules] could not read the range ${range}`);
    process.exit(2);
  }
  commits = out
    .split('\n')
    .filter(Boolean)
    .map((l) => {
      const [ref, author, subject] = l.split('\0');
      return { ref, subject, author };
    });
}

function gitOr(argv, fallback) {
  try {
    return execFileSync('git', argv, { encoding: 'utf8' }).trim();
  } catch {
    return fallback;
  }
}

let failed = 0;
for (const c of commits) {
  const problems = checkSubject(c.subject, c.author, rules);
  if (problems.length === 0) continue;
  failed++;
  console.error(`\n✖ ${c.ref}: ${c.subject}`);
  for (const p of problems) console.error(`    ${p}`);
}

if (failed > 0) {
  console.error(
    `\n[commit-rules] ${failed} of ${commits.length} subject(s) broke the rules in .github/commit-rules.json.` +
      (messageFile
        ? ' Reword and commit again.'
        : ' Reword them (git rebase -i, or amend the latest) and push again.'),
  );
  process.exit(1);
}
console.log(`[commit-rules] ${commits.length} subject(s) checked, all pass.`);
