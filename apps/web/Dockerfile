# syntax=docker/dockerfile:1.7
# Next.js 15 standalone build.
# Final image: ~200 MB (Node 20 alpine + standalone bundle).

FROM node:20-alpine@sha256:fb4cd12c85ee03686f6af5362a0b0d56d50c58a04632e6c0fb8363f609372293 AS deps
WORKDIR /app
RUN apk add --no-cache libc6-compat

# Workspace manifests
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml turbo.json ./
COPY apps/web/package.json apps/web/
COPY packages/api-client-ts/package.json packages/api-client-ts/

RUN corepack enable && corepack prepare pnpm@9.15.0 --activate \
    && pnpm install --frozen-lockfile --filter web... --filter api-client-ts...

FROM node:20-alpine@sha256:fb4cd12c85ee03686f6af5362a0b0d56d50c58a04632e6c0fb8363f609372293 AS builder
WORKDIR /app
ENV NEXT_TELEMETRY_DISABLED=1
# Opt into Next.js `output: 'standalone'`. Gated in next.config.mjs so
# local Windows builds (where the trace-copy step EPERMs on pnpm's
# symlinked node_modules) succeed by default; the runtime stage below
# copies .next/standalone, so this env var MUST stay set in this image.
ENV NEXT_STANDALONE_BUILD=1

# Pin Server Actions encryption key across builds. Without this,
# `next build` generates a fresh random key per build, so every Web
# image gets different content-addressed Server Action IDs — and any
# browser tab open before a deploy 404s on the next form submit
# ("UnrecognizedActionError: Server Action … was not found on the
# server"). Supplied as a docker build-arg sourced from a GH Actions
# secret + matching runtime env. MUST be the same value at build
# time and at runtime.
#
# Generate once with `openssl rand -base64 32`. Empty value is
# allowed (matches the pre-fix behaviour) but defeats the purpose;
# CI should always pass a real key.
ARG NEXT_SERVER_ACTIONS_ENCRYPTION_KEY=""
ENV NEXT_SERVER_ACTIONS_ENCRYPTION_KEY=${NEXT_SERVER_ACTIONS_ENCRYPTION_KEY}

COPY --from=deps /app/node_modules ./node_modules
COPY --from=deps /app/apps/web/node_modules ./apps/web/node_modules
COPY . .
RUN corepack enable && pnpm --filter web build

FROM node:20-alpine@sha256:fb4cd12c85ee03686f6af5362a0b0d56d50c58a04632e6c0fb8363f609372293 AS runtime
WORKDIR /app
ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV PORT=3000
ENV HOSTNAME="0.0.0.0"

RUN apk add --no-cache wget tini \
    && addgroup --system --gid 65532 nextjs \
    && adduser --system --uid 65532 --ingroup nextjs nextjs

COPY --from=builder --chown=nextjs:nextjs /app/apps/web/.next/standalone ./
COPY --from=builder --chown=nextjs:nextjs /app/apps/web/.next/static ./apps/web/.next/static
COPY --from=builder --chown=nextjs:nextjs /app/apps/web/public ./apps/web/public

USER nextjs:nextjs
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 \
    CMD wget --quiet --spider http://localhost:3000/api/healthz || exit 1

ENTRYPOINT ["tini", "--", "node", "apps/web/server.js"]
