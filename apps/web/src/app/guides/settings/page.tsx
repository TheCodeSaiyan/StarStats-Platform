import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Settings',
  description:
    'The account pages, and the one setting that leaves clock-based tiles empty until you save it.',
};

/* Facts traced at v1.8.167, not remembered:
 *   - section list: app/settings/page.tsx h2s — Appearance, Local time,
 *     Account info, Email verification, RSI handle ownership, Change
 *     sign-in email, Change password, Delete account (2FA moved to
 *     /settings/2fa and the old card is now a "Moved to its own page"
 *     pointer)
 *
 * Local time leads because it is the only setting whose absence makes
 * working features look broken: the clock rules do not run at all without
 * a stored zone, so the tiles are empty rather than wrong. Silent
 * emptiness reads as a bug, so say it out loud. */
export default function SettingsGuidePage() {
  return (
    <MarketingSurface
      navId="guides"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Guides', href: '/guides' },
        { label: 'Settings' },
      ]}
      title="Settings"
      ctx="Guides · calibration and consent"
    >
      <DocsIndex active="/guides/settings" />
    <div className="ss-about">
      <div className="ss-placard" style={{ marginBottom: 'var(--s5)' }}>
        Guides
      </div>

      <h1
        style={{
          fontSize: 'clamp(40px, 6vw, 64px)',
          letterSpacing: 'var(--tracking-tight)',
          lineHeight: 1.05,
          margin: '0 0 var(--s4)',
          fontWeight: 600,
        }}
      >
        Settings.
      </h1>

      <p
        className="ss-lede"
        style={{
          fontSize: 'var(--fs-lg)',
          color: 'var(--fg-muted)',
          lineHeight: 1.55,
          margin: '0 0 var(--s7)',
          maxWidth: '60ch',
        }}
      >
        Most of <Link href={'/settings' as Route}>/settings</Link> is what
        you&apos;d expect. One card is worth reading about, because
        without it some tiles stay empty and look broken.
      </p>

      <section className="ss-about-section" id="local-time">
        <div className="ss-about-section-eyebrow">01 — Local time</div>
        <h2>Set this or the clock tiles stay blank.</h2>
        <p>
          Anything phrased in clock time — whether you fly late, which
          weekday is yours — can&apos;t be worked out without knowing your
          timezone. We record when things happened, not what time it felt
          like where you were sitting.
        </p>
        <p>
          The picker pre-fills your browser&apos;s zone, but{' '}
          <strong>nothing is stored until you press Save</strong>. Until
          then it looks set and isn&apos;t, which is the one genuinely
          confusing thing on the page.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          We store a zone name rather than an offset, so the answer stays
          right across daylight saving. A fixed offset would be an hour out
          for half the year — precisely at the evening boundary these tiles
          care about.
        </p>
      </section>

      <section className="ss-about-section" id="handle">
        <div className="ss-about-section-eyebrow">02 — RSI handle</div>
        <h2>Proving the handle is yours.</h2>
        <p>
          <strong>RSI handle ownership</strong> is step 04 of{' '}
          <Link href={'/docs' as Route}>setup</Link>, and it&apos;s here if
          you skipped it. You get a code, paste it into your public RSI bio,
          save on RSI, then press <strong>Check now</strong>.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Nothing polls. The check happens only when you press the button,
          so saving your bio and walking away won&apos;t verify you.
        </p>
      </section>

      <section className="ss-about-section" id="account">
        <div className="ss-about-section-eyebrow">03 — Account</div>
        <h2>The rest of the page.</h2>
        <p>
          <strong>Account info</strong> and{' '}
          <strong>Email verification</strong> — who you are and whether
          we&apos;ve confirmed your address.
        </p>
        <p>
          <strong>Change sign-in email</strong> and{' '}
          <strong>Change password</strong> — both confirm before they take
          effect.
        </p>
        <p>
          <strong>Appearance</strong> — theme. If the desktop app has cloud
          sync on, this and the app&apos;s own appearance setting follow you
          between machines.
        </p>
        <p>
          <Link href={'/settings/2fa' as Route}>Two-factor</Link> has its
          own page rather than a card, so the setup flow has room.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>Delete account</strong> is at the bottom and means it.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Elsewhere</div>
        <h2>Adjacent pages people look for here.</h2>
        <p>
          <Link href={'/downloads' as Route}>Connected uplinks</Link> — paired
          machines, and where you generate a pairing code or revoke one.{' '}
          <Link href={'/guides/sharing' as Route}>Sharing</Link> — who can
          see your profile, which is deliberately not a settings card.
        </p>
        <p>
          <Link href={'/changelog' as Route}>Changelog</Link> for what
          shipped, <Link href={'/roadmap' as Route}>roadmap</Link> for
          what&apos;s next.
        </p>
      </section>
    </div>
    </MarketingSurface>
  );
}
