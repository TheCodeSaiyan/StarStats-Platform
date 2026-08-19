import { describe, expect, it } from 'vitest';
import { clusterSubmissions } from './cluster';
import type { AdminParserSubmissionSummary } from '@/lib/api';

function row(
  overrides: Partial<AdminParserSubmissionSummary>,
): AdminParserSubmissionSummary {
  return {
    id: 1,
    shape_hash: 'hash-1',
    coarse_shape: 'shape-a',
    shell_tag: null,
    raw_example_preview: null,
    status: 'pending',
    submitter_count: 1,
    total_occurrence_count: 1,
    first_submitted_at: '2026-07-01T00:00:00Z',
    last_submitted_at: '2026-07-01T00:00:00Z',
    ...overrides,
  };
}

describe('clusterSubmissions', () => {
  it('groups rows sharing a coarse_shape into one cluster', () => {
    const rows: AdminParserSubmissionSummary[] = [
      row({
        id: 1,
        shape_hash: 'hash-1',
        coarse_shape: 'shape-a',
        submitter_count: 2,
        total_occurrence_count: 5,
      }),
      row({
        id: 2,
        shape_hash: 'hash-2',
        coarse_shape: 'shape-a',
        submitter_count: 3,
        total_occurrence_count: 10,
      }),
      row({
        id: 3,
        shape_hash: 'hash-3',
        coarse_shape: 'shape-b',
        submitter_count: 1,
        total_occurrence_count: 1,
      }),
    ];

    const clusters = clusterSubmissions(rows);

    expect(clusters).toHaveLength(2);
  });

  it('sums occurrences and submitters for a shared cluster', () => {
    const rows: AdminParserSubmissionSummary[] = [
      row({
        id: 1,
        shape_hash: 'hash-1',
        coarse_shape: 'shape-a',
        submitter_count: 2,
        total_occurrence_count: 5,
      }),
      row({
        id: 2,
        shape_hash: 'hash-2',
        coarse_shape: 'shape-a',
        submitter_count: 3,
        total_occurrence_count: 10,
      }),
      row({
        id: 3,
        shape_hash: 'hash-3',
        coarse_shape: 'shape-b',
        submitter_count: 1,
        total_occurrence_count: 1,
      }),
    ];

    const clusters = clusterSubmissions(rows);
    const shapeA = clusters.find((c) => c.coarseShape === 'shape-a');

    expect(shapeA).toBeDefined();
    expect(shapeA?.members).toHaveLength(2);
    expect(shapeA?.totalOccurrences).toBe(15);
    expect(shapeA?.totalSubmitters).toBe(5);
  });

  it('picks the higher-occurrence member as the representative', () => {
    const rows: AdminParserSubmissionSummary[] = [
      row({
        id: 1,
        shape_hash: 'hash-1',
        coarse_shape: 'shape-a',
        submitter_count: 2,
        total_occurrence_count: 5,
      }),
      row({
        id: 2,
        shape_hash: 'hash-2',
        coarse_shape: 'shape-a',
        submitter_count: 3,
        total_occurrence_count: 10,
      }),
    ];

    const clusters = clusterSubmissions(rows);

    expect(clusters[0]?.representative.id).toBe(2);
  });

  it('ranks clusters by summed occurrences descending', () => {
    const rows: AdminParserSubmissionSummary[] = [
      row({
        id: 1,
        shape_hash: 'hash-1',
        coarse_shape: 'shape-low',
        total_occurrence_count: 2,
      }),
      row({
        id: 2,
        shape_hash: 'hash-2',
        coarse_shape: 'shape-high',
        total_occurrence_count: 100,
      }),
    ];

    const clusters = clusterSubmissions(rows);

    expect(clusters.map((c) => c.coarseShape)).toEqual([
      'shape-high',
      'shape-low',
    ]);
  });

  it('breaks ties in summed occurrences by coarseShape localeCompare', () => {
    const rows: AdminParserSubmissionSummary[] = [
      row({
        id: 1,
        shape_hash: 'hash-1',
        coarse_shape: 'shape-b',
        total_occurrence_count: 5,
      }),
      row({
        id: 2,
        shape_hash: 'hash-2',
        coarse_shape: 'shape-a',
        total_occurrence_count: 5,
      }),
    ];

    const clusters = clusterSubmissions(rows);

    expect(clusters.map((c) => c.coarseShape)).toEqual([
      'shape-a',
      'shape-b',
    ]);
  });
});
