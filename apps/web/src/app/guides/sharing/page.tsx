import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Sharing',
  description:
    'Your profile is private by default. Two separate levels control who can see you and which categories they see.',
};

/* Facts traced at v1.8.167, not remembered:
 *   - visibility model + the three audiences: app/sharing/page.tsx
 *     (public toggle, "Shared with specific handles", "Shared with orgs",
 *     "People sharing with you")
 *   - category list: app/settings/widget-sharing/page.tsx WIDGET_LABELS
 *
 * The two-levels framing leads because the pages are in different places
 * (/sharing vs /settings/widget-sharing) and a user who finds only the
 * first concludes sharing is all-or-nothing — then over-shares, or
 * doesn't share at all. Both failure modes come from the same gap. */
export default function SharingGuidePage() {
  return (
    <MarketingSurface
      navId="guides"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Guides', href: '/guides' },
        { label: 'Sharing' },
      ]}
      title="Sharing"
      ctx="Guides · who can see what"
    >
      <DocsIndex active="/guides/sharing" />
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
        Sharing.
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
        Private until you say otherwise. Who gets in and what they see are
        two different decisions, set on two different pages — knowing that
        is most of what this guide is for.
      </p>

      <section className="ss-about-section" id="who">
        <div className="ss-about-section-eyebrow">01 — Who can see you</div>
        <h2>Three audiences, on /sharing.</h2>
        <p>
          <Link href={'/sharing' as Route}>/sharing</Link> is where you
          decide who gets in. Your profile starts private.
        </p>
        <p>
          <strong>Public</strong> — anyone with the link sees a summary:
          how many events you have logged, your busiest event types, and an
          activity heatmap. New public profiles are held to the top five
          types and the last 30 days, and{' '}
          <Link href={'/sharing' as Route}>/sharing</Link> spells out what
          that publishes before you turn it on. Widen it if you want to; it
          will not widen on its own. The page gives you the URL once
          it&apos;s on.
        </p>
        <p>
          <strong>Specific handles</strong> — stay private, but let named
          people through one at a time.
        </p>
        <p>
          <strong>Orgs</strong> — let an organisation&apos;s members
          through as a group.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The same page lists <strong>people sharing with you</strong>, which
          is the only way to find out someone has — nothing notifies you.
        </p>
      </section>

      <section className="ss-about-section" id="feed">
        <div className="ss-about-section-eyebrow">02 — What a share shows</div>
        <h2>People you let in see the log, not just the shape.</h2>
        <p>
          Someone you have shared with opens your profile and gets your
          actual events, in the same words you read them in on{' '}
          <Link href={'/me/activity' as Route}>your own activity log</Link>.
          Until recently a share showed only counts and a heatmap — enough
          to tell that you had been playing, never what happened.
        </p>
        <p>
          <strong>New shares cover the last 30 days.</strong> That is a
          starting point rather than a ceiling: widen the window, or narrow
          a share down to particular event types, under{' '}
          <em>More options</em> when you grant it.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Events you have hidden stay hidden, whatever a share says. That
          one outranks everything else here — it is the control that decides
          what exists to be shared in the first place.
        </p>
      </section>

      <section className="ss-about-section" id="what">
        <div className="ss-about-section-eyebrow">03 — Which categories</div>
        <h2>Category visibility is a separate page.</h2>
        <p>
          <Link href={'/settings/widget-sharing' as Route}>
            Widget visibility
          </Link>{' '}
          controls which <em>categories</em> a viewer gets, whoever they
          are: combat, commerce, movement, records, and your recent-activity
          feed.
        </p>
        <p>
          The two levels multiply rather than override. A public profile
          with commerce switched off means anyone can find you and nobody
          sees what you spend. A private profile with everything switched on
          still shows nothing to anyone you haven&apos;t let in.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Two levels because &ldquo;let my org see me&rdquo; and &ldquo;let
          anyone see my spending&rdquo; are different decisions. Collapsing
          them into one switch would force you to over-share in order to
          share at all.
        </p>
      </section>

      <section className="ss-about-section" id="revoke">
        <div className="ss-about-section-eyebrow">04 — Taking it back</div>
        <h2>Every grant is reversible.</h2>
        <p>
          Going private again, dropping a handle, or leaving an org share
          all take effect immediately — there&apos;s no window where an old
          link keeps working.
        </p>
        <p>
          Machines are separate from people:{' '}
          the <Link href={'/downloads' as Route}>Emitter page</Link> lists
          the desktop apps paired to your account, and revoking one there
          stops that machine sending. That is a different action from
          un-sharing your profile, and you may want both.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Related</div>
        <h2>What leaves your machine in the first place.</h2>
        <p>
          Sharing decides who sees data we already hold. What gets
          collected at all is a different question, and{' '}
          <Link href={'/trust' as Route}>/trust</Link> answers it —
          including the part people expect to be untrue.
        </p>
      </section>
    </div>
    </MarketingSurface>
  );
}
