import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MeIdentityHeader } from './MeIdentityHeader';

describe('MeIdentityHeader', () => {
  it('renders headline lifetime stats', () => {
    render(
      <MeIdentityHeader
        handle="alice"
        supporterTier="contributor"
        enlistmentDate="2021-03-01"
        totalEvents={1204}
        deaths={50}
        kills={120}
        playtimeSecs={418 * 3600}
        locationsVisited={67}
      />,
    );
    expect(screen.getByText('@alice')).toBeInTheDocument();
    expect(screen.getByText(/418h/)).toBeInTheDocument();
    expect(screen.getByText(/67/)).toBeInTheDocument();
  });

  it('formats playtime as rounded hours', () => {
    render(
      <MeIdentityHeader
        handle="bob"
        supporterTier={null}
        enlistmentDate={null}
        totalEvents={10}
        deaths={0}
        kills={0}
        playtimeSecs={90 * 60}
        locationsVisited={3}
      />,
    );
    // 90 minutes = 1.5h -> rounds to 2h
    expect(screen.getByText(/2h/)).toBeInTheDocument();
  });

  it('derives K/D as kills/deaths to one decimal', () => {
    render(
      <MeIdentityHeader
        handle="carol"
        supporterTier={null}
        enlistmentDate="2020-06-15"
        totalEvents={500}
        deaths={50}
        kills={120}
        playtimeSecs={0}
        locationsVisited={9}
      />,
    );
    // 120 / 50 = 2.4
    expect(screen.getByText('2.4')).toBeInTheDocument();
  });

  it('shows kills as the K/D when deaths is zero (no division)', () => {
    render(
      <MeIdentityHeader
        handle="dave"
        supporterTier={null}
        enlistmentDate={null}
        totalEvents={3}
        deaths={0}
        kills={7}
        playtimeSecs={0}
        locationsVisited={1}
      />,
    );
    expect(screen.getByText('7')).toBeInTheDocument();
  });

  it('renders the enlistment year when present', () => {
    render(
      <MeIdentityHeader
        handle="erin"
        supporterTier="generous"
        enlistmentDate="2019-11-02"
        totalEvents={42}
        deaths={1}
        kills={2}
        playtimeSecs={3600}
        locationsVisited={5}
      />,
    );
    expect(screen.getByText(/2019/)).toBeInTheDocument();
  });
});

describe('MeIdentityHeader K/D provenance', () => {
  it('marks K/D when some deaths were reconstructed', () => {
    // K/D is DERIVED from deaths, so a partly-reconstructed death count
    // makes the ratio itself partly a guess.
    render(
      <MeIdentityHeader
        handle="alice"
        supporterTier={null}
        enlistmentDate={null}
        totalEvents={10}
        deaths={4}
        deathsInferred={3}
        kills={8}
        playtimeSecs={0}
        locationsVisited={0}
      />,
    );
    const marked = screen.getByRole('note');
    expect(marked.getAttribute('aria-label')).toContain('3 of 4 inferred');
    expect(marked.getAttribute('aria-label')).toContain('Corpse lines');
  });

  it('leaves K/D unmarked when every death was observed', () => {
    const { container } = render(
      <MeIdentityHeader
        handle="alice"
        supporterTier={null}
        enlistmentDate={null}
        totalEvents={10}
        deaths={4}
        deathsInferred={0}
        kills={8}
        playtimeSecs={0}
        locationsVisited={0}
      />,
    );
    expect(container.querySelector('[role="note"]')).toBeNull();
  });

  it('is unmarked when the prop is absent entirely', () => {
    // Existing call sites that never pass it must not start showing a
    // provenance marker they have no data for.
    const { container } = render(
      <MeIdentityHeader
        handle="alice"
        supporterTier={null}
        enlistmentDate={null}
        totalEvents={10}
        deaths={4}
        kills={8}
        playtimeSecs={0}
        locationsVisited={0}
      />,
    );
    expect(container.querySelector('[role="note"]')).toBeNull();
  });
});
