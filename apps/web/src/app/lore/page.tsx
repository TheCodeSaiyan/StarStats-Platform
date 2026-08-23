import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Universe primer',
  description:
    'A concise newcomer primer to the Star Citizen universe: the UEE, major factions, notable systems, and key historical events.',
};

function LoreSection({
  label,
  title,
  children,
}: {
  label: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section
      className="ss-card"
      style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s5)' }}
    >
      {/* Category placard — encodes what the section IS (not an ordinal). */}
      <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
        {label}
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

export default function LorePage() {
  return (
    <MarketingSurface
      navId="home"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Lore' },
      ]}
      title="Lore"
      ctx="The in-universe framing"
    >
      <DocsIndex active="/lore" />
    <div>
      <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
        Lore · Star Citizen
      </div>
      <h1
        style={{
          margin: 0,
          fontSize: 'clamp(40px, 6vw, 64px)',
          fontWeight: 600,
          letterSpacing: 'var(--tracking-tight)',
        }}
      >
        Universe primer
      </h1>
      <hr className="ss-rule" style={{ margin: 'var(--s5) 0 var(--s2)' }} />
      <p
        style={{
          color: 'var(--fg)',
          fontSize: 'var(--fs-base)',
          lineHeight: 1.65,
          marginTop: 'var(--s4)',
        }}
      >
        Cliff notes for newcomers stepping into the 30th century. This
        primer sketches the political shape of human space, the species
        you will meet, the systems that anchor daily life, and the
        events that explain why the galaxy looks the way it does today.
      </p>

      <LoreSection label="Governance" title="The UEE & timeline">
        <p style={{ margin: 0 }}>
          Star Citizen is set in the 30th century in a galaxy dominated
          by the United Empire of Earth, a human government that grew
          out of earlier polities — the United Nations of Earth and
          then the United Planets of Earth. Humanity&apos;s expansion
          accelerated after the discovery of jump points, naturally
          occurring wormholes that link star systems and make practical
          interstellar travel possible. The UEE is governed by an
          Imperator and a Senate, and draws a sharp distinction between
          citizens (typically earned through service) and civilians.
          The current era follows the fall of the Messer dynasty and a
          long, ongoing reform movement reshaping the empire&apos;s
          relationship with its frontier worlds and alien neighbours.
        </p>
      </LoreSection>

      <LoreSection label="Peoples" title="Factions">
        <p style={{ margin: 0 }}>
          Beyond the UEE, several established civilizations and a
          constant churn of smaller powers share the galaxy.
        </p>
        <ul style={listStyle}>
          <li>
            <strong>UEE.</strong> The dominant human state, organised
            around a militarised civil service and a citizen / civilian
            divide.
          </li>
          <li>
            <strong>Banu.</strong> A merchant species organised as the
            Banu Protectorate — a loose confederation of trading
            worlds. First contact with humanity occurred in the 25th
            century and established trade, not war, as a viable
            template.
          </li>
          <li>
            <strong>Xi&apos;an.</strong> A long-lived, methodical
            species governing the Xi&apos;an Empire. Relations with
            humanity were hostile for centuries before normalising into
            a cautious peace.
          </li>
          <li>
            <strong>Vanduul.</strong> A nomadic, clan-based species in
            chronic conflict with the UEE along the frontier,
            particularly around lost human systems.
          </li>
          <li>
            <strong>Tevarin.</strong> A proud warrior culture twice
            defeated by humanity in the Tevarin Wars; survivors live
            mostly within UEE space.
          </li>
        </ul>
      </LoreSection>

      <LoreSection label="Geography" title="Notable systems">
        <p style={{ margin: 0 }}>
          A handful of star systems carry outsized weight in everyday
          life and current events.
        </p>
        <ul style={listStyle}>
          <li>
            <strong>Sol.</strong> Humanity&apos;s home system and the
            symbolic heart of the empire, with Earth as its political
            and cultural centre.
          </li>
          <li>
            <strong>Terra.</strong> A prosperous, Earth-like world that
            has grown into a political and commercial rival to Sol,
            often associated with reformist sentiment.
          </li>
          <li>
            <strong>Stanton.</strong> A fully corporate-owned system
            whose four planets belong to major megacorporations — the
            setting most players first encounter.
          </li>
          <li>
            <strong>Pyro.</strong> A lawless, unclaimed system adjacent
            to Stanton, dominated by outlaw groups and contested
            resource sites.
          </li>
          <li>
            <strong>Nyx, Cathcart, and other border systems.</strong>{' '}
            Frontier and unclaimed space where smugglers, prospectors,
            and independents operate beyond steady UEE reach.
          </li>
        </ul>
      </LoreSection>

      <LoreSection label="History" title="Key historical events">
        <p style={{ margin: 0 }}>
          A short list of turning points that shape the present day.
        </p>
        <ul style={listStyle}>
          <li>
            <strong>Discovery of jump points.</strong> Nick
            Croshaw&apos;s transit through the first known jump point
            opened practical interstellar travel and triggered
            humanity&apos;s expansion beyond Sol.
          </li>
          <li>
            <strong>First contact with the Banu.</strong> Humanity&apos;s
            first meeting with another spacefaring civilization,
            establishing trade rather than war as a possible template.
          </li>
          <li>
            <strong>Tevarin Wars.</strong> Two conflicts in which the
            UNE / UPE defeated the Tevarin, ending their independent
            civilization and absorbing survivors into human space.
          </li>
          <li>
            <strong>The Messer Era.</strong> A long period of
            authoritarian rule under the Messer dynasty, marked by
            militarism, censorship, and aggressive expansion before its
            eventual collapse and reform.
          </li>
          <li>
            <strong>Loss of systems to the Vanduul.</strong> Sustained
            Vanduul raids and incursions led to the loss of frontier
            human worlds and remain a defining security concern.
          </li>
        </ul>
      </LoreSection>
    </div>
    </MarketingSurface>
  );
}
