/**
 * Server-side fetchers for the public roadmap surface.
 *
 * No auth — these hit the read-only public endpoints. `cache: 'no-store'`
 * mirrors `api.ts` so a freshly-shipped item doesn't sit behind Next's
 * data cache.
 */

import 'server-only';
import type { components as apiSchema } from 'api-client-ts';
import { apiBase } from './api';

export type RoadmapItemPublic = apiSchema['schemas']['RoadmapItemPublic'];
export type ChannelStatusPublic =
  apiSchema['schemas']['ChannelStatusPublic'];
export type RoadmapListResponse =
  apiSchema['schemas']['RoadmapListResponse'];
export type ChangelogEntryPublic =
  apiSchema['schemas']['ChangelogEntryPublic'];
export type ChangelogResponse = apiSchema['schemas']['ChangelogResponse'];

async function getPublic<T>(path: string): Promise<T> {
  const resp = await fetch(`${apiBase()}${path}`, {
    method: 'GET',
    cache: 'no-store',
  });
  if (!resp.ok) {
    throw new Error(`roadmap fetch ${path} → ${resp.status}`);
  }
  return (await resp.json()) as T;
}

export async function listRoadmap(): Promise<RoadmapListResponse> {
  return getPublic<RoadmapListResponse>('/v1/roadmap');
}

export async function getRoadmapItem(
  slug: string,
): Promise<RoadmapItemPublic | null> {
  const resp = await fetch(`${apiBase()}/v1/roadmap/${encodeURIComponent(slug)}`, {
    method: 'GET',
    cache: 'no-store',
  });
  if (resp.status === 404) return null;
  if (!resp.ok) throw new Error(`roadmap item fetch → ${resp.status}`);
  return (await resp.json()) as RoadmapItemPublic;
}

export async function listChangelog(): Promise<ChangelogResponse> {
  return getPublic<ChangelogResponse>('/v1/roadmap/changelog');
}
