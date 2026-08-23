import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

// Resolve the platform version from the workspace Cargo.toml at build
// time, then expose it via `NEXT_PUBLIC_PLATFORM_VERSION` so the
// footer sub-line can render it (see apps/web/src/app/layout.tsx).
// Per the release design notes the
// `web` image rides the `platform` version line (server + web ship
// together); the tray has its own independent version stream.
//
// Reads `[workspace.package].version` from `<repo>/Cargo.toml`.
// Soft-fails to "unknown" rather than failing the build — the version
// chip is decorative, not load-bearing, and a missing version
// shouldn't break a local dev server when run outside the monorepo.
function readPlatformVersion() {
  try {
    const here = path.dirname(fileURLToPath(import.meta.url));
    // apps/web → repo root is two dirs up.
    const cargoPath = path.resolve(here, '..', '..', 'Cargo.toml');
    const content = readFileSync(cargoPath, 'utf8');
    const m = content.match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s);
    return m ? m[1] : 'unknown';
  } catch {
    return 'unknown';
  }
}
const PLATFORM_VERSION = process.env.NEXT_PUBLIC_PLATFORM_VERSION || readPlatformVersion();

// Packages that must NEVER be bundled by webpack on the server side.
// `@grpc/grpc-js` does `require('stream')` in plain CommonJS that
// webpack's resolver chokes on, and the OTel SDK chain pulls it in
// transitively from `instrumentation.ts`. `serverExternalPackages`
// covers Server Components / Route Handlers, but the instrumentation
// bundle uses a separate webpack config — hence the explicit
// `config.externals` push in the `webpack` callback below.
const otelExternals = [
  '@opentelemetry/sdk-node',
  '@opentelemetry/exporter-trace-otlp-grpc',
  '@opentelemetry/auto-instrumentations-node',
  '@opentelemetry/resources',
  '@opentelemetry/semantic-conventions',
  '@grpc/grpc-js',
  // `@sentry/node` (used in `instrumentation.ts` to ship errors to
  // GlitchTip) pulls native Node deps for source-map handling that
  // webpack can't resolve. Same treatment as the OTel stack.
  '@sentry/node',
  // pino's transport mechanism (used by `lib/logger.ts` to wire
  // pino-pretty in dev) spawns a worker thread that dynamically
  // requires the target module at runtime. Next.js's bundler can't
  // see that dynamic require, so without externalising pino +
  // pino-pretty the worker tries to load
  // `.next/server/vendor-chunks/lib/worker.js` which never gets
  // emitted, and the dev server crashes on the first server-side
  // log call (which Playwright trips immediately via TopBar's
  // location fetch).
  'pino',
  'pino-pretty',
];

// Baseline security headers applied to every response. Traefik in front
// of this app may layer additional headers (HSTS preload, CSP); the
// values here are the safe defaults that don't depend on the deployment
// topology.
const securityHeaders = [
  {
    key: 'Strict-Transport-Security',
    value: 'max-age=31536000; includeSubDomains',
  },
  { key: 'X-Frame-Options', value: 'DENY' },
  { key: 'X-Content-Type-Options', value: 'nosniff' },
  { key: 'Referrer-Policy', value: 'strict-origin-when-cross-origin' },
  {
    key: 'Permissions-Policy',
    value: 'camera=(), microphone=(), geolocation=(), interest-cohort=()',
  },
];

// `output: 'standalone'` is required by the production Dockerfile
// (copies .next/standalone into the runtime image) but the trace-copy
// step relies on filesystem symlinks into pnpm's nested node_modules
// layout — which Windows refuses without Developer Mode / admin
// elevation, EPERM'ing `pnpm --filter web build` for local Windows
// contributors. Default OFF so local builds always succeed; the
// Dockerfile sets NEXT_STANDALONE_BUILD=1 so prod images still get
// the standalone bundle.
const enableStandalone = process.env.NEXT_STANDALONE_BUILD === '1';

/** @type {import('next').NextConfig} */
const nextConfig = {
  ...(enableStandalone ? { output: 'standalone' } : {}),
  poweredByHeader: false,
  reactStrictMode: true,
  experimental: {
    typedRoutes: true,
  },
  // Exposed to client + server code as `process.env.NEXT_PUBLIC_PLATFORM_VERSION`.
  // Inlined at build time, so changing this requires a rebuild — which
  // is exactly what we want (a version chip shouldn't drift from the
  // bundle it ships with).
  env: {
    NEXT_PUBLIC_PLATFORM_VERSION: PLATFORM_VERSION,
  },
  // `holo` is a workspace package shipping TS/TSX SOURCE with real runtime
  // JSX. Unlike `api-client-ts` (types only, fully erased at build time) it
  // must actually be compiled, so Next has to be told to transpile it.
  transpilePackages: ['holo'],
  serverExternalPackages: otelExternals,
  async headers() {
    return [
      {
        source: '/:path*',
        headers: securityHeaders,
      },
    ];
  },
  // Defensive: the Revolut return URL lives in SERVER config
  // (`RevolutConfig.return_url`), which may still point at the old
  // `/support/return` path from before the /support → /donate split.
  // This catches a stale server config so a customer isn't 404'd
  // after a successful payment. This is the payment-return handler
  // ONLY — the `/support` help page itself must never redirect to
  // `/donate`.
  async redirects() {
    return [
      {
        source: '/support/return',
        destination: '/donate/return',
        permanent: true,
      },
    ];
  },
  webpack: (config, { isServer }) => {
    if (isServer) {
      const externals = config.externals;
      if (Array.isArray(externals)) {
        externals.push(...otelExternals);
      } else if (externals !== undefined) {
        config.externals = [externals, ...otelExternals];
      } else {
        config.externals = [...otelExternals];
      }
    }
    return config;
  },
};

export default nextConfig;
