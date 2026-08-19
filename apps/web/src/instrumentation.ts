/**
 * OpenTelemetry tracing + GlitchTip error reporting bootstrap for the
 * Next.js server runtime.
 *
 * Next.js 15+ auto-loads this file once per server process (no
 * config opt-in needed). The exported `register()` is invoked
 * before any route handlers run, which is exactly when OTel needs
 * to install its async-hooks-based context manager so spans
 * propagate correctly through `fetch()` and HTTP handlers.
 *
 * Design mirrors the Rust API (`crates/starstats-server/src/telemetry.rs`):
 *  - Read `OTEL_EXPORTER_OTLP_ENDPOINT`. Empty/unset -> silent no-op
 *    so local `pnpm dev` works without a collector.
 *  - Read `OTEL_SERVICE_NAME`, default to `starstats-web`.
 *  - Boot is best-effort; any error is logged to stderr and the
 *    server continues without tracing rather than failing to start.
 *  - SIGTERM hook flushes queued spans on container shutdown.
 *
 * GlitchTip uses the Sentry protocol; `@sentry/node` ships errors as
 * long as `GLITCHTIP_DSN` is set. Same degraded-boot semantics as OTel.
 *
 * The Edge runtime is skipped — `@opentelemetry/sdk-node` requires
 * Node built-ins (`async_hooks`, `perf_hooks`) that the Edge runtime
 * does not expose.
 */

async function initSentry(): Promise<void> {
  const dsn = process.env.GLITCHTIP_DSN;
  if (!dsn || dsn.length === 0) {
    // No DSN -> error reports are silently dropped. `error.tsx` still
    // renders the user-facing page; we just don't aggregate centrally.
    return;
  }

  try {
    const Sentry = await import('@sentry/node');
    Sentry.init({
      dsn,
      // Tracing is owned by OpenTelemetry; turn off Sentry's own
      // performance tracing to avoid double-instrumentation overhead.
      tracesSampleRate: 0,
      // GlitchTip respects `environment` and `release` for grouping.
      environment: process.env.NODE_ENV ?? 'development',
      release: process.env.GLITCHTIP_RELEASE,
      // Don't ship PII by default; the `onRequestError` hook below
      // strips the Cookie / Authorization headers explicitly.
      sendDefaultPii: false,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    process.stderr.write(`Sentry/GlitchTip init failed: ${msg}\n`);
  }
}

export async function register(): Promise<void> {
  // Next runs this file in both `nodejs` and `edge` runtimes.
  // sdk-node only works under Node.
  if (process.env.NEXT_RUNTIME !== 'nodejs') {
    return;
  }

  // Init Sentry first so errors during OTel bootstrap reach the
  // collector. Both calls early-return when their env-gates are unset.
  await initSentry();

  const endpoint = process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
  if (!endpoint || endpoint.length === 0) {
    // Local dev with no collector configured — silent degrade.
    return;
  }

  const serviceName = process.env.OTEL_SERVICE_NAME ?? 'starstats-web';

  try {
    // Lazy-load heavy SDK only after the env-gate, so dev without
    // a collector pays zero cost on boot.
    const { NodeSDK } = await import('@opentelemetry/sdk-node');
    const { OTLPTraceExporter } = await import(
      '@opentelemetry/exporter-trace-otlp-grpc'
    );
    const { getNodeAutoInstrumentations } = await import(
      '@opentelemetry/auto-instrumentations-node'
    );
    const { resourceFromAttributes } = await import('@opentelemetry/resources');
    const { ATTR_SERVICE_NAME } = await import(
      '@opentelemetry/semantic-conventions'
    );

    const sdk = new NodeSDK({
      resource: resourceFromAttributes({
        [ATTR_SERVICE_NAME]: serviceName,
      }),
      traceExporter: new OTLPTraceExporter({ url: endpoint }),
      instrumentations: [
        getNodeAutoInstrumentations({
          // fs instrumentation is extremely chatty inside Next's
          // server bundle (every static asset read). Disable it
          // unless we hit a specific need.
          '@opentelemetry/instrumentation-fs': { enabled: false },
        }),
      ],
    });

    sdk.start();

    // Flush spans synchronously on container shutdown so the last
    // request's traces actually reach Tempo.
    const shutdown = (): void => {
      sdk
        .shutdown()
        .catch((err: unknown) => {
          process.stderr.write(
            `OTel SDK shutdown error: ${err instanceof Error ? err.message : String(err)}\n`,
          );
        })
        .finally(() => {
          process.exit(0);
        });
    };
    process.on('SIGTERM', shutdown);
    process.on('SIGINT', shutdown);
  } catch (err: unknown) {
    // Never let telemetry init crash the server. Match the Rust
    // side's behavior: log the reason, degrade to logs+metrics only.
    const msg = err instanceof Error ? err.message : String(err);
    process.stderr.write(`OTel tracing disabled: ${msg}\n`);
  }
}

/**
 * Next 15+ calls this for every server-side error: server actions,
 * route handlers, RSC render failures, middleware throws. We forward
 * to GlitchTip via the already-initialized Sentry SDK. The DSN-not-set
 * case is a no-op because `Sentry.init` was never called, so
 * `captureException` becomes a noop client.
 */
export async function onRequestError(
  err: unknown,
  request: Readonly<{
    path: string;
    method: string;
    headers: Record<string, string | string[] | undefined>;
  }>,
  context: Readonly<{
    routerKind: 'Pages Router' | 'App Router';
    routePath: string;
    routeType: 'render' | 'route' | 'action' | 'middleware';
    revalidateReason: 'on-demand' | 'stale' | undefined;
    renderSource:
      | 'react-server-components'
      | 'react-server-components-payload'
      | 'server-rendering';
  }>,
): Promise<void> {
  if (!process.env.GLITCHTIP_DSN) return;

  try {
    const Sentry = await import('@sentry/node');
    Sentry.captureException(err, {
      tags: {
        route: context.routePath,
        route_type: context.routeType,
        method: request.method,
      },
      extra: {
        path: request.path,
        router_kind: context.routerKind,
        render_source: context.renderSource,
      },
    });
  } catch (reportErr: unknown) {
    const msg =
      reportErr instanceof Error ? reportErr.message : String(reportErr);
    process.stderr.write(`onRequestError: failed to ship to GlitchTip: ${msg}\n`);
  }
}
