/**
 * What to CALL the things in each catalogue category, and what its radar
 * actually measures.
 *
 * The comparison surface was written against vehicles and then reused for the
 * other three categories without changing a word: a reader browsing weapons,
 * items or locations was asked to "Add ship…" and told the comparison "holds
 * 10 ships", and every radar sheet was captioned "Handling" even where the
 * axes were damage and range. One table here, rather than a conditional at
 * each call site, so adding a category cannot leave a stray noun behind.
 *
 * `vehicle` is deliberately "vehicle" and not "ship": the category holds
 * ground vehicles too, which is exactly the same class of mistake in reverse.
 */

import type { ReferenceCategory } from './reference-types';

export interface CategoryVocabulary {
  /** One entry, lowercase, for mid-sentence use: "Add vehicle…". */
  one: string;
  /** Several entries, lowercase: "comparison holds 10 vehicles". */
  many: string;
  /** Sentence-leading plural: "All vehicles". */
  manyTitle: string;
  /**
   * Cap over the radar sheet. The axes differ per category, so a single
   * caption cannot be honest: vehicles plot speed and agility, weapons plot
   * damage and range.
   */
  radarCap: string;
}

const VOCABULARY: Record<ReferenceCategory, CategoryVocabulary> = {
  vehicle: {
    one: 'vehicle',
    many: 'vehicles',
    manyTitle: 'All vehicles',
    radarCap: 'Handling',
  },
  weapon: {
    one: 'weapon',
    many: 'weapons',
    manyTitle: 'All weapons',
    radarCap: 'Firepower',
  },
  item: {
    one: 'item',
    many: 'items',
    manyTitle: 'All items',
    radarCap: 'Properties',
  },
  location: {
    one: 'location',
    many: 'locations',
    manyTitle: 'All locations',
    radarCap: 'Scale',
  },
};

/** Falls back to the neutral `item` wording rather than throwing: a wrong
 *  noun is a blemish, a crashed catalogue page is an outage. */
export function vocabularyFor(category: ReferenceCategory): CategoryVocabulary {
  return VOCABULARY[category] ?? VOCABULARY.item;
}
