/**
 * Splits the "fields (comma / newline separated)" textarea value from
 * the Publish rule form into the `fields: string[]` the
 * `PublishRuleRequest` body expects.
 *
 * Extracted as a pure function (rather than left inline in the
 * `publishRuleAction` server action) so the splitting logic can be
 * unit-tested directly — a server action with `'use server'` can't be
 * imported into a plain vitest module.
 */
export function parseFieldsInput(raw: string): string[] {
  return raw
    .split(/[\n,]/)
    .map((f) => f.trim())
    .filter(Boolean);
}
