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
# `web...` resolves the whole workspace dependency graph, so every package web
# depends on needs its manifest here — without one, pnpm links the package but
# never installs ITS dependencies, and the missing piece only shows up as a
# type error deep in `next build`.
COPY packages/holo/package.json packages/holo/
COPY packages/design-tokens/package.json packages/design-tokens/
COPY packages/reference-data/package.json packages/reference-data/

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

# SOURCES FIRST, then the installed trees on top.
#
# The other order clobbers: with no `.dockerignore` in this repo, `COPY . .`
# carries whatever `node_modules` the build context happens to have — a
# developer's Windows install, in the local case — straight over the Linux
# ones from `deps`, and `next` disappears mid-build. CI checks out clean so it
# never saw this; anyone building the image locally hit it immediately.
COPY . .
COPY --from=deps /app/node_modules ./node_modules
COPY --from=deps /app/apps/web/node_modules ./apps/web/node_modules
# A WORKSPACE PACKAGE'S OWN `node_modules` IS NOT OPTIONAL.
#
# `holo` is TypeScript source consumed through `transpilePackages`, and its
# types come from ITS `@types/react`. Copying only the root and `apps/web`
# trees left `import React from 'react'` inside the package resolving to the
# JS with no types — so `React` degraded to `any`, `interface BeamButtonProps
# extends React.ButtonHTMLAttributes<...>` silently inherited nothing, and the
# build died on `Property 'children' does not exist on type BeamButtonProps`.
# A type error nowhere near the cause, in a file that compiles fine locally.
COPY --from=deps /app/packages/holo/node_modules ./packages/holo/node_modules
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
