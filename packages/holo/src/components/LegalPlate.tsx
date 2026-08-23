import React from 'react';

/**
 * The kit's trademark line.
 *
 * EXPORTED, BUT NOT USED BY THIS APP'S PAGES — read the note on `LegalPlate`
 * below before reaching for it. It is kept so the component's default matches
 * the design system it came from, and so the difference is visible rather than
 * quietly erased.
 */
export const CIG_DISCLAIMER =
  'Star Citizen is a trademark of Cloud Imperium Games. StarStats is not affiliated with, ' +
  'endorsed by, or sponsored by Cloud Imperium Games or the Roberts Space Industries website.';

/**
 * Legal plate — the footer every signed-out and static surface carries.
 *
 * The disclaimer is a legal requirement for an unofficial community tool, not
 * decoration, so it ships as a component rather than as copy each screen
 * retypes.
 *
 * **PASS THE PRODUCT'S OWN ATTRIBUTION, NOT THE DEFAULT.** The kit's
 * `CIG_DISCLAIMER` and the string this product actually ships are not the same
 * text: the shipped footer names Squadron 42, asserts the Cloud Imperium
 * Rights copyright over ship/vehicle/weapon/item names AND specifications, and
 * links to `/about` for the data-sources statement. It is longer and it is
 * legal copy, which is not a porter's to reword — so the app passes its own
 * `disclaimer` and the kit's default stands unused.
 *
 * If you are tempted to drop the prop and take the default because it is
 * shorter: that is a rewrite of a legal notice.
 */
export interface LegalPlateProps {
  version?: React.ReactNode;
  licence?: React.ReactNode;
  links?: React.ReactNode;
  disclaimer?: React.ReactNode;
}

export function LegalPlate({
  version,
  licence = 'MPL-2.0',
  links,
  disclaimer = CIG_DISCLAIMER,
}: LegalPlateProps) {
  return (
    <footer className="hp-legal">
      <p className="dis">{disclaimer}</p>
      <div className="meta">
        {links ? <span className="lk">{links}</span> : null}
        <span className="sp" />
        {licence ? <span>{licence}</span> : null}
        {version ? <span>{version}</span> : null}
      </div>
    </footer>
  );
}
