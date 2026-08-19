import type { Metadata } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Terms of Service',
  description:
    'The terms you agree to when you use StarStats: what the service is, what it promises, what it does not, and the law it runs under.',
};

const LAST_UPDATED = '16 July 2026';
const CONTACT_EMAIL = 'dojo@thecodesaiyan.io';

/* Mirrors the wrappers in privacy/page.tsx so the two legal pages read as
 * one document set. Deliberately duplicated rather than shared: they are
 * the only two consumers, and lifting a component out of a legal page to
 * a shared module invites restyling it for a third caller later. If a
 * third legal page ever lands, extract then. */
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

const codeStyle: React.CSSProperties = {
  fontFamily: 'var(--font-mono)',
  fontSize: '0.92em',
  background: 'var(--bg-elev)',
  padding: '1px 6px',
  borderRadius: 'var(--r-sm)',
};

export default function TermsPage() {
  return (
    <main>
      <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
        Legal · Terms
      </div>
      <h1
        style={{
          margin: 0,
          fontSize: 'clamp(40px, 6vw, 64px)',
          fontWeight: 600,
          letterSpacing: 'var(--tracking-tight)',
        }}
      >
        Terms of Service
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
        These are the terms you agree to when you use StarStats. They are
        short on purpose. StarStats is free, open source, and run by one
        person — there is no company here, and dressing this up in the
        language of one would only obscure what you are actually agreeing
        to. If anything below is unclear, email{' '}
        <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a> and ask.
      </p>

      <PolicySection num="1" title="Who you are agreeing with">
        <p style={{ margin: 0 }}>
          StarStats is built and operated by a single maintainer, not a
          company. Where these terms say &ldquo;we&rdquo; or &ldquo;I&rdquo;,
          they mean that one person. Where they say &ldquo;you&rdquo;, they
          mean you — the person using the service or running the desktop
          client.
        </p>
        <p style={{ marginBottom: 0 }}>
          You need to be old enough to agree to terms on your own behalf in
          the country you live in. If you are not, do not create an account.
        </p>
      </PolicySection>

      <PolicySection num="2" title="What StarStats is, and what it is not">
        <p style={{ marginTop: 0 }}>
          StarStats reads the log file that Star Citizen already writes to
          your own disk, turns it into a record of your own play, and — only
          if you ask it to — syncs that record to this website.
        </p>
        <p>It does not do any of the following, by design:</p>
        <ul style={listStyle}>
          <li>Inject a DLL, overlay, or anything else into the game</li>
          <li>Attach to the game process or read its memory</li>
          <li>Sniff or modify game network traffic</li>
          <li>Run macros or automate any in-game action</li>
        </ul>
        <p style={{ marginBottom: 0 }}>
          This is not a marketing promise; it is a design constraint, and it
          is why StarStats is safe to run alongside anti-cheat. The reasoning
          is written up in{' '}
          <a href="https://github.com/TheCodeSaiyan/StarStats-Platform/blob/main/EAC-SAFETY.md">
            EAC-SAFETY.md
          </a>
          , and the source is public, so you do not have to take my word for
          any of it.
        </p>
      </PolicySection>

      <PolicySection num="3" title="Your account">
        <p style={{ marginTop: 0 }}>
          You are responsible for keeping your sign-in details and your
          device pairing tokens to yourself. If you think someone else has
          them, revoke the device from{' '}
          <Link href="/devices">your devices page</Link> and change your
          password.
        </p>
        <p style={{ marginBottom: 0 }}>
          You can delete your account and everything attached to it at any
          time. How that works, and how long deletion takes to propagate, is
          set out in the <Link href="/privacy">Privacy Policy</Link>.
        </p>
      </PolicySection>

      <PolicySection num="4" title="Acceptable use">
        <p style={{ marginTop: 0 }}>Do not:</p>
        <ul style={listStyle}>
          <li>
            Upload events that are not yours, or forge event data to
            misrepresent what happened
          </li>
          <li>
            Use StarStats to harass, stalk, or build a dossier on another
            player
          </li>
          <li>
            Attempt to break the service, exhaust it, or reach data that is
            not yours
          </li>
          <li>Use StarStats to break Cloud Imperium&rsquo;s own rules</li>
        </ul>
        <p style={{ marginBottom: 0 }}>
          Finding a security flaw and reporting it privately is not a
          violation — it is a favour, and{' '}
          <a href="https://github.com/TheCodeSaiyan/StarStats-Platform/blob/main/SECURITY.md">
            SECURITY.md
          </a>{' '}
          explains how to do it.
        </p>
      </PolicySection>

      <PolicySection num="5" title="This is a beta, and what that actually means">
        <p style={{ marginTop: 0 }}>
          StarStats is in public beta. Things will break. Features will
          change or disappear. You may hit bugs nobody has seen yet — that is
          largely the point of you being here.
        </p>
        <p>
          <strong>On your data:</strong> there is no wipe planned, and I will
          treat your synced data as something worth keeping. But I am not
          promising it survives. A migration can go wrong, and a homelab is a
          homelab. Do not treat StarStats as the only copy of anything you
          care about.
        </p>
        <p style={{ marginBottom: 0 }}>
          Worth knowing, because it makes the above less alarming than it
          sounds: your events are derived from{' '}
          <code style={codeStyle}>Game.log</code> on your own machine. If the
          server ever lost data, the desktop client can re-parse and re-upload
          from your local logs — as far back as your own log history goes.
          The server is a convenience and a viewer, not the origin of your
          record. You are.
        </p>
      </PolicySection>

      <PolicySection num="6" title="Your data, and other players&rsquo;">
        <p style={{ marginTop: 0 }}>
          Your events stay yours. Sync is off until you turn it on, and what
          is visible to anyone else is controlled by you from{' '}
          <Link href="/sharing">your sharing settings</Link>. Publishing your
          profile is a thing you choose, not a default.
        </p>
        <p style={{ marginBottom: 0 }}>
          One thing worth saying plainly, because it is easy to miss: your
          logs mention other players — whoever was in your instance, who you
          fought, who you flew with. When you sync, those names come along.
          The <Link href="/privacy">Privacy Policy</Link> sets out exactly
          what that means and what happens to it. Please read that section
          rather than skim it; it is the part with someone else in it.
        </p>
      </PolicySection>

      <PolicySection num="7" title="No warranty, and the limits of my liability">
        <p style={{ marginTop: 0 }}>
          StarStats is provided free and &ldquo;as is&rdquo;, with no
          warranty of any kind. I do not promise it will be available, that
          it will be correct, or that it will keep working tomorrow the way
          it works today.
        </p>
        <p style={{ marginBottom: 0 }}>
          To the fullest extent the law allows, I am not liable for any loss
          or damage arising out of your use of StarStats — including lost
          data, lost time, or anything that happens to your game account.
          Nothing in these terms limits liability for death or personal
          injury caused by negligence, for fraud, or for anything else that
          cannot lawfully be limited — and if you are a consumer, your
          statutory rights are unaffected by anything written here.
        </p>
      </PolicySection>

      <PolicySection num="8" title="Ending things">
        <p style={{ marginTop: 0 }}>
          You can stop using StarStats whenever you like, delete your
          account, and uninstall the client. Nothing is retained that the{' '}
          <Link href="/privacy">Privacy Policy</Link> does not account for.
        </p>
        <p style={{ marginBottom: 0 }}>
          I may suspend or remove an account that is breaking section 4, or
          that is damaging the service for other people. If that ever happens
          to you and you think it is a mistake, email me — there is no
          appeals department, there is just me, and I would rather fix a
          wrong call than defend it.
        </p>
      </PolicySection>

      <PolicySection num="9" title="Not affiliated with Cloud Imperium">
        <p style={{ marginTop: 0 }}>
          StarStats is an unofficial fan project. It is not affiliated with,
          endorsed by, or sponsored by Cloud Imperium Games or Roberts Space
          Industries. Star Citizen and all related marks and content are the
          property of their respective owners.
        </p>
        <p style={{ marginBottom: 0 }}>
          This project exists at the pleasure of the people who make the
          game. If CIG ever ask for something to change, it changes. More on
          where that line sits is on the <Link href="/about">About page</Link>
          .
        </p>
      </PolicySection>

      <PolicySection num="10" title="Changes to these terms">
        <p style={{ margin: 0 }}>
          If these terms change in a way that materially affects you, the
          change will be announced before it takes effect and the date at the
          top of this page will be updated. Fixing a typo is not a material
          change and will not be announced. Continuing to use StarStats after
          a change means you accept it; if you do not, delete your account.
        </p>
      </PolicySection>

      <PolicySection num="11" title="Governing law">
        <p style={{ margin: 0 }}>
          These terms are governed by the law of England and Wales, and the
          courts of England and Wales have exclusive jurisdiction over any
          dispute arising from them. If you are a consumer resident elsewhere
          in the UK or the EU, this does not deprive you of the protection of
          the mandatory law of the country you live in.
        </p>
      </PolicySection>

      <p style={{ marginTop: 'var(--s6)' }}>
        <Link href="/">← Back to StarStats</Link>
      </p>
    </main>
  );
}
