/**
 * Prometheus metrics registry for the web app.
 *
 * Exposed at `/api/metrics`. The voyager Prometheus scrapes this
 * endpoint every 30s alongside the API's own `/metrics`.
 *
 * Cardinality discipline:
 *  - never label by user ID, email, session ID — those are unbounded.
 *  - always label by route, status, outcome — those are small bounded sets.
 *
 * Counter naming: `starstats_web_*`. The `prom-client` library doesn't
 * auto-suffix with `_total` like Rust's metrics-exporter-prometheus does,
 * so we include the suffix explicitly.
 *
 * Server-only — `prom-client` uses Node `perf_hooks` and the registry is
 * a process-global; both are wrong for the browser bundle.
 */

import 'server-only';
import {
  collectDefaultMetrics,
  Counter,
  Histogram,
  Registry,
} from 'prom-client';

// One registry per process. Next.js dev mode hot-reloads modules and
// would re-register the default metrics each time, throwing
// "Metric with name X has already been registered". The global cache
// keyed on a Symbol survives module reload.
const REGISTRY_KEY = Symbol.for('starstats.web.registry');

interface RegistryHolder {
  [REGISTRY_KEY]?: Registry;
}

const globalCache = globalThis as unknown as RegistryHolder;

function buildRegistry(): Registry {
  const r = new Registry();
  collectDefaultMetrics({ register: r, prefix: 'starstats_web_' });
  return r;
}

export const registry: Registry =
  globalCache[REGISTRY_KEY] ?? (globalCache[REGISTRY_KEY] = buildRegistry());

// --- Custom metrics ----------------------------------------------

export const authAttemptsTotal = getOrCreateCounter({
  name: 'starstats_web_auth_attempts_total',
  help: 'Auth attempts handled by the web app, by outcome.',
  labelNames: ['action', 'outcome'] as const,
});

export const apiCallDurationSeconds = getOrCreateHistogram({
  name: 'starstats_web_api_call_duration_seconds',
  help: 'Time spent in server-side fetch() to the StarStats API.',
  labelNames: ['endpoint', 'status'] as const,
  buckets: [0.01, 0.05, 0.1, 0.25, 0.5, 1, 2, 5],
});

// Re-running module init under dev hot-reload would call `new Counter`
// twice with the same name; prom-client throws on duplicate. These
// helpers fetch the existing metric from the registry if present.
function getOrCreateCounter<T extends string>(opts: {
  name: string;
  help: string;
  labelNames: readonly T[];
}): Counter<T> {
  const existing = registry.getSingleMetric(opts.name);
  if (existing) return existing as Counter<T>;
  return new Counter({ ...opts, registers: [registry] });
}

function getOrCreateHistogram<T extends string>(opts: {
  name: string;
  help: string;
  labelNames: readonly T[];
  buckets: number[];
}): Histogram<T> {
  const existing = registry.getSingleMetric(opts.name);
  if (existing) return existing as Histogram<T>;
  return new Histogram({ ...opts, registers: [registry] });
}
