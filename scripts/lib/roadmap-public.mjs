// Pure helpers for setting ONE roadmap item's Public field during
// `normalize-roadmap-project.mjs --promote-draft ... --public <yes|no>`.
//
// Why this exists: promotion already turned a draft into a labelled Issue
// in one command, but visibility was left to a click on the board, and
// until someone made it an item never reached /roadmap or What's New.
// The audit mode's `--set-public-yes` could not be used instead, because
// it rewrites Public on EVERY item and flattens curated visibility.

/**
 * Parse the `--public` value. Returns the board's option name.
 * @param {string | undefined} v
 * @returns {'Yes' | 'No'}
 */
export function parsePublicChoice(v) {
  const s = String(v ?? '').trim().toLowerCase();
  if (s === 'yes' || s === 'true' || s === 'public') return 'Yes';
  if (s === 'no' || s === 'false' || s === 'private') return 'No';
  throw new Error(`--public expects yes or no (got ${JSON.stringify(v ?? '')})`);
}

/**
 * The option id for a choice on the Project's `Public` single-select field.
 * @param {{ options: { id: string, name: string }[] }} publicField
 * @param {'Yes' | 'No'} choice
 * @returns {string}
 */
export function publicOptionId(publicField, choice) {
  const opt = (publicField?.options ?? []).find((o) => o.name === choice);
  if (!opt) {
    throw new Error(`The "Public" field has no "${choice}" option. Add Yes/No options first.`);
  }
  return opt.id;
}
