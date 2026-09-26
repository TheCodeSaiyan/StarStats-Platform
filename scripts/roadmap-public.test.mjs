// Tests for scripts/lib/roadmap-public.mjs. Run with:
//
//   node --test scripts/roadmap-public.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';

import { parsePublicChoice, publicOptionId } from './lib/roadmap-public.mjs';

const FIELD = {
  options: [
    { id: 'opt-yes', name: 'Yes' },
    { id: 'opt-no', name: 'No' },
  ],
};

test('parsePublicChoice accepts yes and no in the usual spellings', () => {
  for (const v of ['yes', 'YES', ' Yes ', 'true', 'public']) {
    assert.equal(parsePublicChoice(v), 'Yes', v);
  }
  for (const v of ['no', 'No', 'false', 'private']) {
    assert.equal(parsePublicChoice(v), 'No', v);
  }
});

test('parsePublicChoice rejects anything else rather than guessing', () => {
  for (const v of [undefined, '', 'maybe', 'y']) {
    assert.throws(() => parsePublicChoice(v), /--public expects yes or no/);
  }
});

test('publicOptionId maps a choice to its option id', () => {
  assert.equal(publicOptionId(FIELD, 'Yes'), 'opt-yes');
  assert.equal(publicOptionId(FIELD, 'No'), 'opt-no');
});

test('publicOptionId fails loudly when the board lacks the option', () => {
  assert.throws(
    () => publicOptionId({ options: [{ id: 'x', name: 'Yes' }] }, 'No'),
    /no "No" option/,
  );
});
