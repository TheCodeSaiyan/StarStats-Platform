// node --test scripts/publish-release-notes.test.mjs
//
// Runs the real script against a local HTTP server that checks the
// signature the way the Rust server does (roadmap::events::
// verify_event_signature): HMAC-SHA256 over "v1.<ts>." + body, sent as
// "v1=<hex>". A mismatch here would mean every release's notes are
// refused in production while CI reports a warning nobody reads.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('./publish-release-notes.mjs', import.meta.url));
const KEY = 'test-release-key';

function serve(status) {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const ts = req.headers['x-starstats-timestamp'];
      const sig = req.headers['x-starstats-signature'];
      const expected = `v1=${crypto.createHmac('sha256', KEY).update(`v1.${ts}.`).update(body).digest('hex')}`;
      seen.push({ path: req.url, validSignature: sig === expected, body: JSON.parse(body) });
      res.writeHead(status);
      res.end(status === 200 ? '{}' : 'no');
    });
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () => resolve({ server, seen, port: server.address().port })),
  );
}

function run(env, tag = 'v0.1.61') {
  return new Promise((resolve) => {
    const p = spawn(process.execPath, [SCRIPT, '--tag', tag, '--offline'], {
      env: { ...process.env, ...env },
    });
    let out = '';
    p.stdout.on('data', (d) => (out += d));
    p.stderr.on('data', (d) => (out += d));
    p.on('close', (code) => resolve({ code, out }));
  });
}

test('sends a signed release the server-side check accepts', async () => {
  const { server, seen, port } = await serve(200);
  try {
    const { code, out } = await run({
      ROADMAP_CI_EVENT_HMAC_KEY: KEY,
      RELEASE_NOTES_URL: `http://127.0.0.1:${port}/v1/internal/releases`,
    });
    assert.equal(code, 0, out);
    assert.equal(seen.length, 1);
    assert.equal(seen[0].path, '/v1/internal/releases');
    assert.equal(seen[0].validSignature, true);
    assert.equal(seen[0].body.schema_version, 1);
    assert.equal(seen[0].body.tag, 'v0.1.61');
    assert.equal(seen[0].body.track, 'platform');
    assert.equal(seen[0].body.channel, 'live');
    assert.match(seen[0].body.date, /^\d{4}-\d{2}-\d{2}$/);
    assert.ok(Array.isArray(seen[0].body.groups));
  } finally {
    server.close();
  }
});

test('the endpoint is derived from the roadmap events URL', async () => {
  const { server, seen, port } = await serve(200);
  try {
    await run({
      ROADMAP_CI_EVENT_HMAC_KEY: KEY,
      RELEASE_NOTES_URL: '',
      ROADMAP_EVENTS_URL: `http://127.0.0.1:${port}/v1/internal/roadmap/events`,
    });
    assert.equal(seen[0]?.path, '/v1/internal/releases');
  } finally {
    server.close();
  }
});

test('a refusal is a warning, never a failed release', async () => {
  const { server, port } = await serve(401);
  try {
    const { code, out } = await run({
      ROADMAP_CI_EVENT_HMAC_KEY: KEY,
      RELEASE_NOTES_URL: `http://127.0.0.1:${port}/v1/internal/releases`,
    });
    assert.equal(code, 0);
    assert.match(out, /::warning::.*refused/);
  } finally {
    server.close();
  }
});
