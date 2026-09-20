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

// The K/D provenance tests that stood here are gone with the `deathsInferred`
// prop. They asserted a marker gated on `body_class = "inferred"`, a value
// nothing in the pipeline ever writes — so the "every death was observed" case
// they covered was the ONLY one that ever occurred, and the marker never
// rendered. The caveat is now unconditional and travels as `kdNote`.
