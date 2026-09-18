// Contract tests for the roadmap CI event emitter's SILENT paths.
//
// The emitter is deliberately never release-blocking: a missing slug, an
// unconfigured pipeline or a receiver that doesn't know the slug all exit 0
// so a release still ships. That posture is right. What was wrong is that
// some of them exited 0 saying nothing a reader of the Actions run would
// notice — so a release could intend to record something, drop it, and stay
// green.
//
// The line these tests draw is INTENT, not absence:
//
//   No slug at all — most releases. A chore, a dependency bump, a docs
//   catch-up or any fix that ships nothing tracked has no roadmap item and
//   never should. Warning on those would fire on the majority of releases
//   and train every reader to scroll past the annotation, which is how the
//   next real silence gets missed. Stays quiet.
//
//   A slug WAS supplied and the event still did not land — unconfigured
//   secrets, a slug the receiver has never been seeded with, a channel the
//   server doesn't accept. Somebody said "this release ships that item" and
//   the pipeline dropped it on the floor. Warns, still exit 0.
//
// Deciding whether a release SHOULD have carried a slug is a different job
// and a different place: `release-promote.mjs` already discovers `roadmap/*`
// labels on the PRs merged since the previous tag, and that is where a human
// is present to answer. The emitter only ever sees the answer, never the
// question.
//
// Run: node --test scripts/roadmap-emit-event.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const execFileAsync = promisify(execFile);

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, "roadmap-emit-event.mjs");

/** Run the emitter with a controlled env. Never throws on a non-zero exit —
 *  the exit code is part of what these tests assert.
 *
 *  ASYNC on purpose. Two of these tests stand up an HTTP server in THIS
 *  process for the child to call; a synchronous spawn blocks the event loop,
 *  so the server never answers and the child's fetch (which has no timeout)
 *  hangs forever. Cost an interrupted run before it was obvious. */
async function runEmitter(env) {
  const dir = mkdtempSync(join(tmpdir(), "roadmap-emit-"));
  const summary = join(dir, "step-summary.md");
  let stdout = "";
  let status = 0;
  try {
    const r = await execFileAsync(process.execPath, [SCRIPT], {
      encoding: "utf8",
      env: {
        PATH: process.env.PATH,
        SystemRoot: process.env.SystemRoot,
        // Present so the script takes its CI-annotation path; the real job
        // runs with both of these set by the runner.
        GITHUB_ACTIONS: "true",
        GITHUB_STEP_SUMMARY: summary,
        ...env,
      },
    });
    stdout = r.stdout + r.stderr;
  } catch (e) {
    status = e.code ?? 1;
    stdout = (e.stdout ?? "") + (e.stderr ?? "");
  }
  const summaryText = existsSync(summary) ? readFileSync(summary, "utf8") : "";
  rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  return { status, stdout, summary: summaryText };
}

const CONFIGURED = {
  ROADMAP_CI_EVENT_HMAC_KEY: "test-key-not-a-real-secret",
  CHANNEL: "live",
  COMMIT_SHA: "0".repeat(40),
  BUILD_ID: "1234",
  CI_RUN_URL: "https://example.invalid/run/1234",
  TAG: "v0.1.52",
};

test("a release that ships no roadmap item stays quiet", async () => {
  // The common case by a wide margin. Chores, dependency bumps and docs
  // catch-ups ship nothing tracked, and an annotation on each of them would
  // be pure noise.
  const r = await runEmitter({
    ...CONFIGURED,
    ROADMAP_EVENTS_URL: "https://example.invalid/v1/internal/roadmap/events",
    // No ROADMAP_ITEM_SLUG — nothing to record, nothing to complain about.
  });

  assert.equal(r.status, 0);
  assert.doesNotMatch(
    r.stdout,
    /^::warning/m,
    "an untracked release is normal, not a fault",
  );
  assert.match(r.stdout, /no-op/, "it should still say what it did, in the log");
});

test("a slug that cannot be emitted because the pipeline is unconfigured warns", async () => {
  // Intent existed — this release names an item — and the event went
  // nowhere. That is worth an annotation.
  const r = await runEmitter({
    ...CONFIGURED,
    ROADMAP_ITEM_SLUG: "faster-backlog-upload",
    ROADMAP_CI_EVENT_HMAC_KEY: "",
    ROADMAP_EVENTS_URL: "",
  });

  assert.equal(r.status, 0, "a telemetry gap must never fail the release");
  assert.match(r.stdout, /^::warning/m);
  assert.match(r.stdout, /faster-backlog-upload/);
  assert.match(
    r.summary,
    /faster-backlog-upload/,
    "the job summary must record it too — annotations are easy to scroll past",
  );
});

test("a receiver that does not know the slug warns", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(404, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: "unknown_roadmap_slug" }));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address();
  try {
    const r = await runEmitter({
      ...CONFIGURED,
      ROADMAP_EVENTS_URL: `http://127.0.0.1:${port}/v1/internal/roadmap/events`,
      ROADMAP_ITEM_SLUG: "never-seeded",
    });
    assert.equal(r.status, 0, "a soft 404 must not block the release");
    assert.match(r.stdout, /^::warning/m);
    assert.match(r.stdout, /never-seeded/);
  } finally {
    await new Promise((r) => server.close(r));
  }
});

test("a successful emit stays quiet — a warning on every release is noise", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(202, { "content-type": "application/json" });
    res.end(JSON.stringify({ accepted: true }));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address();
  try {
    const r = await runEmitter({
      ...CONFIGURED,
      ROADMAP_EVENTS_URL: `http://127.0.0.1:${port}/v1/internal/roadmap/events`,
      ROADMAP_ITEM_SLUG: "faster-backlog-upload",
    });
    assert.equal(r.status, 0);
    assert.doesNotMatch(
      r.stdout,
      /^::warning/m,
      "the happy path must not annotate, or the annotations stop meaning anything",
    );
    assert.match(r.stdout, /ok \(attempt 1\)/);
  } finally {
    await new Promise((r) => server.close(r));
  }
});
