import { describe, it, expect } from 'vitest';
import type { ReleaseChannel } from '../api';
import { shouldShowChannelMismatchBanner } from './channelMismatch';

const CHANNELS: ReleaseChannel[] = ['alpha', 'beta', 'rc', 'live'];

describe('shouldShowChannelMismatchBanner', () => {
  it('hides the banner when build and configured channels match (every channel)', () => {
    for (const ch of CHANNELS) {
      expect(shouldShowChannelMismatchBanner(ch, ch, null)).toBe(false);
      // A stale ack from a prior build is still irrelevant when channels match.
      expect(shouldShowChannelMismatchBanner(ch, ch, 'beta')).toBe(false);
    }
  });

  it('shows the banner on mismatch when ack is null', () => {
    expect(shouldShowChannelMismatchBanner('alpha', 'live', null)).toBe(true);
    expect(shouldShowChannelMismatchBanner('beta', 'alpha', null)).toBe(true);
  });

  it('hides the banner on mismatch when ack equals the current build channel', () => {
    expect(shouldShowChannelMismatchBanner('alpha', 'live', 'alpha')).toBe(false);
    expect(shouldShowChannelMismatchBanner('beta', 'rc', 'beta')).toBe(false);
  });

  it('re-shows the banner when ack is stale (user upgraded into a different build)', () => {
    // User dismissed last time on the alpha build; now running beta — re-decide.
    expect(shouldShowChannelMismatchBanner('beta', 'live', 'alpha')).toBe(true);
    expect(shouldShowChannelMismatchBanner('rc', 'live', 'beta')).toBe(true);
  });

  it('does not suppress when ack matches the configured channel (only build matters)', () => {
    // Only the BUILD channel suppresses; matching the dropdown should not.
    expect(shouldShowChannelMismatchBanner('alpha', 'live', 'live')).toBe(true);
    expect(shouldShowChannelMismatchBanner('beta', 'rc', 'rc')).toBe(true);
  });
});
