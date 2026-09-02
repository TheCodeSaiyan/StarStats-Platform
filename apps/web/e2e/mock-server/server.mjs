// Mock StarStats API server. Listens on MOCK_PORT (default 3199) and
// answers JSON requests issued by the Next dev server when the user
// drives the app via Playwright.
//
// Why a real HTTP server (not page.route): the web app is RSC + Server
// Actions, so every API call goes from the Next.js Node process to
// `STARSTATS_API_URL`. Browser-side `page.route()` cannot intercept
// fetches issued from Node — they never traverse the browser context.
//
// Scenario model
// --------------
// Tests POST a scenario document to /__mock/scenario at the start of
// each test. The scenario maps "METHOD path" keys to a fixture
// (status + body). Path matching is exact — query strings are
// stripped before lookup.
//
// A scenario also has a `__id` string that's echoed back so a test
// can confirm the scenario landed before navigation. The server
// keeps only the most recent scenario in memory; tests must avoid
// `--workers=>1` against this single-process mock unless they all
// share a fixture.
//
// /__mock/reset clears the active scenario.
// /__mock/calls returns the per-key call log for assertions.

import http from 'node:http';
import { URL } from 'node:url';

const PORT = Number(process.env.MOCK_PORT ?? 3199);

let scenario = { __id: 'default', routes: {} };
const calls = [];

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8');
      if (raw.length === 0) {
        resolve(null);
        return;
      }
      try {
        resolve(JSON.parse(raw));
      } catch (e) {
        reject(e);
      }
    });
    req.on('error', reject);
  });
}

/** Ceiling for a fixture's `delayMs`. See the clamp at the dispatch site. */
const MAX_STUB_DELAY_MS = 30_000;

function send(res, status, body) {
  const json = JSON.stringify(body ?? null);
  res.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(json),
  });
  res.end(json);
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  const method = req.method ?? 'GET';
  const pathOnly = url.pathname;

  // Control plane.
  if (pathOnly.startsWith('/__mock/')) {
    if (method === 'POST' && pathOnly === '/__mock/scenario') {
      try {
        const body = await readBody(req);
        if (!body || typeof body !== 'object') {
          send(res, 400, { error: 'invalid_scenario' });
          return;
        }
        scenario = {
          __id: body.__id ?? 'unnamed',
          routes: body.routes ?? {},
        };
        calls.length = 0;
        send(res, 200, { ok: true, id: scenario.__id });
      } catch (e) {
        send(res, 400, { error: 'bad_json', detail: String(e) });
      }
      return;
    }
    if (method === 'POST' && pathOnly === '/__mock/reset') {
      scenario = { __id: 'default', routes: {} };
      calls.length = 0;
      send(res, 200, { ok: true });
      return;
    }
    if (method === 'GET' && pathOnly === '/__mock/calls') {
      send(res, 200, { calls });
      return;
    }
    if (method === 'GET' && pathOnly === '/__mock/health') {
      send(res, 200, { ok: true, id: scenario.__id });
      return;
    }
    send(res, 404, { error: 'unknown_control_endpoint' });
    return;
  }

  // Data plane.
  let parsedBody = null;
  try {
    if (
      method === 'POST' ||
      method === 'DELETE' ||
      method === 'PUT' ||
      method === 'PATCH'
    ) {
      parsedBody = await readBody(req);
    }
  } catch {
    // Non-JSON DELETE bodies (rare) — ignore and continue with null.
    parsedBody = null;
  }

  const key = `${method} ${pathOnly}`;
  calls.push({
    method,
    path: pathOnly,
    query: url.search,
    body: parsedBody,
    ts: new Date().toISOString(),
  });

  // Look up exact key first; fall back to method+wildcard prefix
  // (e.g. "GET /v1/orgs/*" matches /v1/orgs/anything).
  const stub = scenario.routes[key] ?? findWildcard(scenario.routes, method, pathOnly);

  if (!stub) {
    send(res, 599, {
      error: 'no_mock_fixture',
      detail: `No fixture for ${key}; scenario=${scenario.__id}`,
    });
    return;
  }

  // `delayMs` holds the response open. The only way to observe a
  // server-rendered loading fallback: Next streams the shell + the
  // segment's loading.tsx while this call is outstanding.
  //
  // CLAMPED, and not only to satisfy the js/resource-exhaustion scan that
  // flagged the unbounded version. A scenario is fixture data, so a typo of
  // 60000 for 6000 would park a worker for a minute and read as a hung
  // suite rather than a bad fixture. Playwright's own timeouts are shorter
  // than this ceiling, so a delay that reaches it fails the test honestly.
  const delayMs = Number(stub.delayMs);
  if (Number.isFinite(delayMs) && delayMs > 0) {
    // REJECT rather than clamp. An inline `Math.min` is not a bound the
    // js/resource-exhaustion query recognises, and rejecting is the better
    // harness behaviour regardless: a fixture asking to be held longer than
    // any Playwright timeout is a typo, and it should fail the way an
    // unknown route does instead of quietly becoming a different delay.
    if (delayMs > MAX_STUB_DELAY_MS) {
      send(res, 599, {
        error: 'delay_too_long',
        detail: `delayMs=${delayMs} exceeds the ${MAX_STUB_DELAY_MS}ms ceiling`,
      });
      return;
    }
    setTimeout(() => send(res, stub.status ?? 200, stub.body), delayMs);
    return;
  }

  send(res, stub.status ?? 200, stub.body);
});

function findWildcard(routes, method, pathOnly) {
  for (const k of Object.keys(routes)) {
    if (!k.startsWith(`${method} `)) continue;
    const pat = k.slice(method.length + 1);
    if (!pat.endsWith('*')) continue;
    const prefix = pat.slice(0, -1);
    if (pathOnly.startsWith(prefix)) return routes[k];
  }
  return null;
}

server.listen(PORT, '127.0.0.1', () => {
  // eslint-disable-next-line no-console
  console.log(`[mock-api] listening on http://127.0.0.1:${PORT}`);
});

const stop = () => {
  server.close(() => process.exit(0));
};
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
