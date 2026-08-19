import { describe, it, expect } from 'vitest';
import { WIDGET_META, boundsForWidget, titleForWidget } from './widget-meta';
import { REGISTERED_IDS } from './registry';
import { GRID_COLS, MIN_W, MIN_H, MAX_H } from './grid-layout';

describe('WIDGET_META', () => {
  it('has a title + description + bounds for every registered widget', () => {
    for (const id of REGISTERED_IDS) {
      const meta = WIDGET_META[id];
      expect(meta, `meta for ${id}`).toBeDefined();
      expect(meta.title.length).toBeGreaterThan(0);
      expect(meta.description.length).toBeGreaterThan(0);
    }
  });

  it('declares a sane size envelope for every widget', () => {
    for (const [id, meta] of Object.entries(WIDGET_META)) {
      const b = meta.bounds;
      expect(b.minW, `${id} minW`).toBeGreaterThanOrEqual(MIN_W);
      expect(b.maxW, `${id} maxW`).toBeLessThanOrEqual(GRID_COLS);
      expect(b.maxW, `${id} maxW>=minW`).toBeGreaterThanOrEqual(b.minW);
      expect(b.minH, `${id} minH`).toBeGreaterThanOrEqual(MIN_H);
      expect(b.maxH, `${id} maxH`).toBeLessThanOrEqual(MAX_H);
      expect(b.maxH, `${id} maxH>=minH`).toBeGreaterThanOrEqual(b.minH);
    }
  });

  it('boundsForWidget resolves a known id and falls back for an unknown one', () => {
    expect(boundsForWidget('heatmap')).toEqual(WIDGET_META.heatmap.bounds);
    const fallback = boundsForWidget('not-a-widget');
    expect(fallback.minW).toBeGreaterThanOrEqual(MIN_W);
    expect(fallback.maxW).toBeLessThanOrEqual(GRID_COLS);
  });

  it('titleForWidget returns the mapped title', () => {
    expect(titleForWidget('sessions')).toBe('Play sessions');
    expect(titleForWidget('objectives')).toBe('Mission objectives');
  });

  // Fit-based envelope (the "never scroll, never waste space" rule). A
  // tile shows a BOUNDED summary and links to full depth via "See more";
  // its height is sized to fit that summary — short for stat tiles, tall
  // enough for the capped list.
  it('big-number stat widgets stay short so they never leave wasted space', () => {
    // Their content is a fixed handful of readouts — a tall ceiling would
    // only ever produce an empty box.
    for (const id of ['economy', 'spend', 'records', 'lives', 'objectives'] as const) {
      expect(boundsForWidget(id).maxH, `${id} maxH`).toBeLessThanOrEqual(6);
    }
  });

  it('list / ranking widgets fit their capped top-N summary (minH >= 5)', () => {
    for (const id of ['fleet', 'routes', 'locations', 'docking', 'hangar'] as const) {
      expect(boundsForWidget(id).minH, `${id} minH`).toBeGreaterThanOrEqual(5);
    }
  });
});
