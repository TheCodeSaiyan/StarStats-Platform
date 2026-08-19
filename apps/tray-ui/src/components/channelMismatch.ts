import type { ReleaseChannel } from '../api';

/**
 * Decide whether to surface the channel-mismatch banner.
 *
 * Strategy: dismiss-per-build-channel. The Dismiss action stores
 * the *build* channel's lowercase token in `config.channel_mismatch_ack`.
 * The banner stays hidden as long as that stored value matches the
 * current build channel. If the user later upgrades into a different
 * channel's build (e.g. alpha → beta), the stored ack no longer
 * matches and the banner re-appears, prompting them to re-decide.
 *
 * Mismatches with no ack always show. Channel-matches always hide
 * (no banner to dismiss). The configured channel is intentionally
 * NOT part of the ack key — switching the dropdown shouldn't
 * re-surface a banner the user has already dismissed.
 */
export function shouldShowChannelMismatchBanner(
  buildChannel: ReleaseChannel,
  configuredChannel: ReleaseChannel,
  ack: string | null,
): boolean {
  if (buildChannel === configuredChannel) return false;
  return ack !== buildChannel;
}
