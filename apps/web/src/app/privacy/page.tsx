import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { LegalIndex } from '@/components/projection/LegalIndex';
import type { Metadata } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Privacy Policy',
  description:
    'How StarStats handles your data: what we collect, why, how long we keep it, and the rights you have over it.',
};

const LAST_UPDATED = '5 May 2026';
const CONTROLLER_EMAIL = 'dojo@thecodesaiyan.io';

/* Reusable wrappers to keep each numbered policy section visually
 * consistent. ss-card gives the surface + border, ss-placard gives
 * the small uppercased Michroma label above the heading. Legal
 * sections keep their numbers — they're referenceable structure. */
function PolicySection({
  num,
  title,
  children,
}: {
  num: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section
      className="ss-card"
      style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s5)' }}
    >
      <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
        Section {num}
      </div>
      <h2
        style={{
          margin: '0 0 var(--s3)',
          fontSize: 'var(--fs-lg)',
          fontWeight: 600,
          letterSpacing: '-0.01em',
        }}
      >
        {title}
      </h2>
      <div
        style={{
          color: 'var(--fg)',
          fontSize: 'var(--fs-base)',
          lineHeight: 1.65,
        }}
      >
        {children}
      </div>
    </section>
  );
}

const listStyle: React.CSSProperties = {
  paddingLeft: 'var(--s5)',
  marginTop: 'var(--s2)',
  marginBottom: 0,
};

export default function PrivacyPage() {
  return (
    <MarketingSurface
      navId="privacy"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Privacy' },
      ]}
      title="Privacy"
      ctx="What is collected, and what never is"
    >
      <LegalIndex active="/privacy" />
    <div>
      <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
        Legal · Privacy
      </div>
      <h1
        style={{
          margin: 0,
          fontSize: 'clamp(40px, 6vw, 64px)',
          fontWeight: 600,
          letterSpacing: 'var(--tracking-tight)',
        }}
      >
        Privacy Policy
      </h1>
      <p style={{ color: 'var(--fg-muted)', marginTop: 'var(--s2)' }}>
        Last updated: {LAST_UPDATED}
      </p>

      <hr className="ss-rule" style={{ margin: 'var(--s5) 0 var(--s2)' }} />

      <p
        style={{
          color: 'var(--fg)',
          fontSize: 'var(--fs-base)',
          lineHeight: 1.65,
          marginTop: 'var(--s4)',
        }}
      >
        This page explains what StarStats collects about you, why,
        who we share it with, how long we keep it, and the rights
        you have over it under the UK / EU General Data Protection
        Regulation (GDPR). Plain English wherever possible — if
        anything is unclear, send an email to{' '}
        <a href={`mailto:${CONTROLLER_EMAIL}`}>{CONTROLLER_EMAIL}</a>.
      </p>

      <PolicySection num="1" title="Who we are (the data controller)">
        <p style={{ margin: 0 }}>
          StarStats is operated as a personal hobby project. The data
          controller for the purposes of GDPR is the project
          maintainer, contactable at{' '}
          <a href={`mailto:${CONTROLLER_EMAIL}`}>{CONTROLLER_EMAIL}</a>.
          We do not have a designated Data Protection Officer because
          we are below the threshold that mandates one; the contact
          email above reaches a real person.
        </p>
      </PolicySection>

      <PolicySection num="2" title="What we collect and why">
        <p style={{ margin: 0 }}>
          We only collect what we need to run the service. Nothing is
          sold to third parties, used for advertising, or shared with
          analytics platforms.
        </p>

        <h3 className="hp-subheading">2.1 When you sign up</h3>
        <ul style={listStyle}>
          <li>
            <strong>Email address.</strong> Used as your login
            identifier and for transactional email traffic
            (sign-up verification, password reset, sign-in via
            one-shot magic link, and confirmation when you change
            your sign-in email). Lawful basis:{' '}
            <em>performance of a contract</em> (Art. 6(1)(b)) — we
            can&apos;t run an account for you without it.
          </li>
          <li>
            <strong>Password.</strong> Stored only as an Argon2id
            hash. We never see, log, or transmit the plain text.
            Lawful basis: contract performance.
          </li>
          <li>
            <strong>
              Two-factor authentication (TOTP) secret.
            </strong>{' '}
            Optional. If you enable 2FA, we generate a 160-bit shared
            secret and store it in the user row encrypted with
            AES-256-GCM under a key held in a file outside the
            database. We hold the secret only because authentication
            code verification requires it; it is decrypted in memory
            for each verification and never logged. Lawful basis:
            contract performance.
          </li>
          <li>
            <strong>Recovery codes (TOTP fallback).</strong> Optional.
            When you enable 2FA we mint ten one-shot recovery codes
            and store them as Argon2 hashes — we cannot show them
            again, even to you. They exist so you can sign in if you
            lose your authenticator app. Lawful basis: contract
            performance.
          </li>
          <li>
            <strong>RSI handle (Star Citizen username).</strong> The
            handle that appears in your{' '}
            <code className="hp-code">Game.log</code>. Used to tag the
            events you ingest so we can show <em>your</em> stats and
            not someone else&apos;s. Lawful basis: contract
            performance.
          </li>
        </ul>

        <h3 className="hp-subheading">
          2.2 When the desktop client uploads game events
        </h3>
        <ul style={listStyle}>
          <li>
            <strong>Parsed game events</strong> — kills, deaths,
            mission completions, vehicle changes, location changes,
            client-side errors. Each event is a structured record
            with a timestamp and your RSI handle. We do not collect
            chat, inventory, currency, or screen contents. Lawful
            basis: contract performance.
          </li>
          <li>
            <strong>Other players who appear in your events.</strong>{' '}
            Some events (e.g., a kill credit) reference a second RSI
            handle. We store that handle alongside your event so the
            event is meaningful to you. We don&apos;t build a
            profile of that second person — they appear only when
            they intersected with your gameplay, and only the
            uploader (and the people they explicitly share with) can
            see it. Lawful basis: <em>legitimate interest</em>{' '}
            (Art. 6(1)(f)) — providing meaningful personal stats
            requires referencing pseudonymous public game handles,
            on balance with the low risk to those individuals.
          </li>
        </ul>

        <h3 className="hp-subheading">2.3 When you use the website</h3>
        <ul style={listStyle}>
          <li>
            <strong>Session cookie</strong> (
            <code className="hp-code">starstats_session</code>):
            HttpOnly, Secure (in production), SameSite=Lax, 1-hour
            TTL. Holds your authentication token. Strictly necessary;
            cannot be disabled while you are signed in. No consent
            banner is required for strictly-necessary cookies under
            ePrivacy/PECR.
          </li>
          <li>
            <strong>Server logs</strong> may briefly record your IP
            address for rate-limiting and abuse prevention. IPs are
            not retained in the application database; they live in
            the rate-limiter&apos;s in-memory window and the
            short-lived web-server access log.
          </li>
        </ul>

        <h3 className="hp-subheading">2.4 When something goes wrong</h3>
        <ul style={listStyle}>
          <li>
            <strong>Error reports</strong> are sent to an
            internal error-monitoring service (GlitchTip) so we
            can fix bugs. Reports include the URL path (with
            user-identifying segments scrubbed), the type of error,
            and a stack trace. Lawful basis: legitimate interest
            (service reliability).
          </li>
        </ul>
      </PolicySection>

      <PolicySection num="3" title="Who we share data with (sub-processors)">
        <p style={{ margin: 0 }}>
          The list below is intentionally short. Every sub-processor
          here either runs on the same infrastructure as StarStats
          itself or is strictly necessary for the service to
          function:
        </p>
        <ul style={listStyle}>
          <li>
            <strong>Hosting infrastructure.</strong> StarStats runs
            on infrastructure controlled by the project maintainer.
            No third-party hosting provider has access to the
            application database.
          </li>
          <li>
            <strong>SMTP relay</strong> — transactional email
            traffic (sign-up verification, password reset,
            email change confirmation) is delivered through an
            SMTP server. The relay processes your email address
            and the verification link only. The current relay is
            documented in the deployment notes and is changed only
            with a corresponding update to this policy.
          </li>
          <li>
            <strong>GlitchTip (error monitoring).</strong>{' '}
            Runs on the same infrastructure as the rest of
            StarStats. No data leaves the StarStats deployment.
          </li>
          <li>
            <strong>SpiceDB (authorisation).</strong> Runs on the same
            infrastructure. Stores only the relationships needed for sharing
            (&quot;A can view B&apos;s stats&quot;) — no event data.
          </li>
          <li>
            <strong>Audit log mirror (MinIO).</strong> Object storage
            on the same infrastructure, holding append-only operational
            audit records. Same network boundary as the application
            database.
          </li>
          <li>
            <strong>
              robertsspaceindustries.com (RSI handle verification
              and citizen profile snapshot).
            </strong>{' '}
            When you start the RSI handle verification flow we issue
            you a short code, and when you ask us to check your bio
            we make a single HTTP{' '}
            <code className="hp-code">GET</code> against{' '}
            <code className="hp-code">
              https://robertsspaceindustries.com/citizens/&lt;your-handle&gt;
            </code>{' '}
            to look for that code in the public bio. We send only a
            generic <code className="hp-code">User-Agent</code>{' '}
            identifying StarStats and the page version; no cookie,
            no session token, no account email. Once we confirm
            the code is in your bio we delete it from our row, and
            we do not re-fetch the page outside of an explicit
            verification attempt by you. The verification flow is
            optional but is required before you can publish a public
            profile or share with an org.{' '}
            Once your handle is verified, you can also ask us to
            snapshot your public RSI citizen profile by pressing{' '}
            <em>Refresh now</em> in{' '}
            <Link href="/settings">Settings</Link>. This is a
            user-initiated, on-demand fetch — never a continuous
            background poll — and is rate-limited to one fetch per
            hour per user. The request goes to the same URL (
            <code className="hp-code">
              https://robertsspaceindustries.com/citizens/&lt;your-handle&gt;
            </code>
            ) with the same generic{' '}
            <code className="hp-code">User-Agent</code>; no cookie, no
            session token, no account email. From the page we
            extract your display name, enlistment date, primary
            location (city / region / country, as RSI displays it),
            badges (name and image URL), bio text, and the one-line
            summary of your primary org. All of this is publicly
            visible on RSI&apos;s site to anyone visiting your
            citizen page — we are not scraping anything that
            requires login. We keep one row per snapshot so you can
            see how your profile has changed over time; you can
            request deletion through the normal account-deletion
            flow, at which point snapshots are pseudonymised in the
            same way as game events.
          </li>
          <li>
            <strong>
              Star Citizen Wiki API (ship and vehicle reference data).
            </strong>{' '}
            The Star Citizen Wiki API at{' '}
            <code className="hp-code">https://api.star-citizen.wiki</code>{' '}
            is a community-run, MIT-licensed reference for in-game
            ship and vehicle metadata — it is not operated by
            Roberts Space Industries. We use it to translate the
            internal class names that appear in your{' '}
            <code className="hp-code">Game.log</code> (e.g.{' '}
            <code className="hp-code">AEGS_Avenger_Stalker</code>) into
            player-facing names (&quot;Aegis Avenger Stalker&quot;)
            so the dashboard can display events legibly. The
            exchange is server-to-server and one-directional: a
            scheduled task on the StarStats server fetches the
            reference catalogue once a day, sending only a generic{' '}
            <code className="hp-code">User-Agent</code> identifying
            StarStats — no user data, no email, no RSI handle,
            no event payload, and no IP-on-behalf-of-the-user. The
            request is never made from your browser. The cached
            data is keyed by ship class name and is never linked to
            any user.
          </li>
        </ul>
        <p style={{ marginTop: 'var(--s3)', marginBottom: 0 }}>
          We do not use Google Analytics, Meta pixels, advertising
          networks, or any third-party tracker. We do not embed
          third-party iframes that could observe your usage.
        </p>
      </PolicySection>

      <PolicySection num="4" title="International transfers">
        <p style={{ margin: 0 }}>
          Our infrastructure is hosted within the EEA. We do not
          transfer your data outside the EEA except where unavoidable
          for transactional email delivery (e.g., recipient
          mailservers operated by mail providers who may sit in other
          jurisdictions). If you have a specific concern about a
          mail provider, contact us and we will tell you which SMTP
          relay is in use at that time.
        </p>
      </PolicySection>

      <PolicySection num="5" title="How long we keep your data">
        <ul style={listStyle}>
          <li>
            <strong>Account record</strong> (email, password
            hash, handle): until you delete your account.
          </li>
          <li>
            <strong>Ingested game events</strong>: until you delete
            your account, at which point they are{' '}
            <em>pseudonymised</em> — the row count and structure
            stay so people who shared a timeline with you don&apos;t
            see holes, but your handle and raw log lines are
            replaced with a non-resolvable tombstone. The events are
            no longer linked to you.
          </li>
          <li>
            <strong>Citizen profile snapshots</strong>: until you
            delete your account. On deletion they are pseudonymised
            in the same way as game events — the snapshot rows
            remain but your handle is replaced with a
            non-resolvable tombstone, so they are no longer linked
            to you.
          </li>
          <li>
            <strong>Administrator deletion in response to misuse</strong>:
            the pseudonymisation described above is what happens when
            you delete your own account. Separately, an administrator
            may <em>purge</em> an account — deleting its ingested
            events outright rather than pseudonymising them — in
            response to abuse or misuse of the service. A purge removes
            more of your data, not less. Because the event rows are
            deleted rather than kept, people you had shared a timeline
            with will lose those rows from their own view.
          </li>
          <li>
            <strong>Email verification tokens</strong>: 24
            hours, then auto-expire and are dropped on next access.
          </li>
          <li>
            <strong>Magic-link sign-in tokens</strong>: 15 minutes,
            single-use. Once redeemed (or expired) the token row is
            no longer accepted; we keep the row briefly for audit
            purposes but it cannot be re-issued.
          </li>
          <li>
            <strong>Interim 2FA tokens</strong>: 5 minutes,
            single-use. Issued between &quot;you proved your
            password (or magic-link)&quot; and &quot;you typed an
            authentication code&quot; — useless on their own.
          </li>
          <li>
            <strong>Recovery codes</strong>: until you regenerate or
            disable 2FA. Stored as Argon2 hashes; each consumed code
            is marked used and cannot be replayed.
          </li>
          <li>
            <strong>Device pairing codes</strong>: 5 minutes.
          </li>
          <li>
            <strong>Active device tokens</strong>: until you revoke
            the device or delete your account.
          </li>
          <li>
            <strong>Audit log</strong>: 90 days in the live
            database; an archived mirror exists for operational
            integrity. The audit mirror references your account UUID
            rather than personal details where possible.
          </li>
          <li>
            <strong>Error reports</strong>: 30 days, then deleted.
          </li>
        </ul>
      </PolicySection>

      <PolicySection num="6" title="Your rights">
        <p style={{ margin: 0 }}>
          Under GDPR you have the following rights. Most can be
          exercised directly from the{' '}
          <Link href="/settings">Settings</Link> page; for the
          remainder, send an email to{' '}
          <a href={`mailto:${CONTROLLER_EMAIL}`}>{CONTROLLER_EMAIL}</a>{' '}
          and we will respond within 30 days.
        </p>
        <ul style={listStyle}>
          <li>
            <strong>Access</strong> (Art. 15): you can ask for a copy
            of the personal data we hold about you.
          </li>
          <li>
            <strong>Rectification</strong> (Art. 16): you can update
            your email or RSI handle from Settings, or by
            contacting us.
          </li>
          <li>
            <strong>Erasure</strong> (Art. 17): use the &quot;Delete
            my account&quot; control in Settings. Your account
            record is removed; your ingested events are pseudonymised
            so they are no longer linked to you. Some operational
            audit entries may be retained for the periods listed
            above under a legal-basis of legitimate interest in
            service integrity. An administrator acting on misuse may
            instead purge the events outright — see the retention
            table above.
          </li>
          <li>
            <strong>Restriction</strong> (Art. 18): contact us to
            pause processing of your account.
          </li>
          <li>
            <strong>Portability</strong> (Art. 20): contact us for
            an export of your account record and events as JSON.
          </li>
          <li>
            <strong>Objection</strong> (Art. 21): you may object to
            any processing we carry out under legitimate-interest
            basis (e.g., error monitoring); contact us.
          </li>
          <li>
            <strong>Withdraw consent</strong>: where we rely on
            consent (we don&apos;t, today, for any core processing),
            you can withdraw it at any time without affecting the
            lawfulness of prior processing.
          </li>
          <li>
            <strong>Complaint</strong>: you have the right to lodge
            a complaint with your local data-protection authority.
            In the UK that&apos;s the ICO (
            <a
              href="https://ico.org.uk/make-a-complaint/"
              rel="noopener"
            >
              ico.org.uk
            </a>
            ). In the EU, find your authority at{' '}
            <a
              href="https://edpb.europa.eu/about-edpb/about-edpb/members_en"
              rel="noopener"
            >
              edpb.europa.eu
            </a>
            .
          </li>
        </ul>
      </PolicySection>

      <PolicySection num="7" title="Children">
        <p style={{ margin: 0 }}>
          StarStats is not directed at children under 13 (under 16
          in some EU jurisdictions). We do not knowingly collect
          data from anyone under that age. If you believe a child
          has created an account, contact us and we will remove it.
        </p>
      </PolicySection>

      <PolicySection num="8" title="Automated decision-making">
        <p style={{ margin: 0 }}>
          We do not perform automated decision-making or profiling
          that produces legal or similarly significant effects on
          you. Stats and timelines are descriptive aggregations of
          events you uploaded — no scoring, ranking against other
          users, or eligibility decisions are made.
        </p>
      </PolicySection>

      <PolicySection num="9" title="Security">
        <p style={{ margin: 0 }}>
          Passwords are hashed with Argon2id. Sessions are HttpOnly +
          Secure cookies with a 1-hour TTL. The API uses RS256 JWTs
          signed by a key generated on the server. All ingress is
          served over HTTPS with HSTS. Database connections require
          TLS in our production deployment. Access to the
          deployment&apos;s underlying infrastructure is limited to
          the project maintainer.
        </p>
        <p style={{ marginTop: 'var(--s3)', marginBottom: 0 }}>
          Two-factor authentication (optional). If you enable it, we
          store your TOTP shared secret encrypted with AES-256-GCM
          under a key held in a file outside the database, with a
          fresh nonce per encryption. Recovery codes are stored only
          as Argon2 hashes — we can verify a code you give us but
          cannot read it back. Disabling 2FA wipes both the secret
          and the recovery-code rows.
        </p>
        <p style={{ marginTop: 'var(--s3)', marginBottom: 0 }}>
          Magic-link and 2FA flows use single-use, short-lived
          interim tokens (15 minutes for magic links, 5 minutes for
          the post-password 2FA token); a leaked link or interim
          token without the matching second factor is useless on
          its own. Failed sign-in attempts are timing-equalised to
          avoid revealing whether an email maps to an account.
        </p>
        <p style={{ marginTop: 'var(--s3)', marginBottom: 0 }}>
          If we ever discover a personal-data breach that is likely
          to result in a risk to your rights and freedoms, we will
          notify you and the relevant supervisory authority within
          72 hours of becoming aware, in line with Art. 33 / 34.
        </p>
      </PolicySection>

      <PolicySection num="10" title="Changes to this policy">
        <p style={{ margin: 0 }}>
          When we change this policy in a way that affects you (new
          sub-processor, new category of data, new retention period)
          we update the &quot;Last updated&quot; date at the top
          and, for material changes, post a notice on the dashboard
          for existing users. Trivial wording fixes are not
          announced.
        </p>
      </PolicySection>

      <p style={{ marginTop: 'var(--s6)' }}>
        <Link href="/">← Back to StarStats</Link>
      </p>
    </div>
    </MarketingSurface>
  );
}
