/**
 * Pure assembly helpers for the structured inference-rule author form
 * (Task 6). Extracted out of `InferenceRuleForm` (rather than inlined
 * in component state handlers) so the risky bit — turning free-form
 * key/value rows into the `PublishInferenceRuleRequest` wire shape —
 * can be unit-tested without touching the DOM or React.
 *
 * Mirrors the `parseFieldsInput` extraction pattern from
 * `apps/web/src/app/admin/parser-submissions/[id]/fields.ts`.
 */
import type { InferenceRuleDto } from '@/lib/api';

export type KV = { key: string; value: string };

export type PatternState = {
  event_type: string;
  field_equals: KV[];
};

export type FormState = {
  id: string;
  confidence: string;
  window_secs: string;
  trigger: PatternState;
  followups: PatternState[];
  emit: {
    event_type: string;
    fields: KV[];
  };
};

/**
 * Turns KV rows into the `Record<string, string>` map the wire DTOs
 * expect. Rows with a blank (or whitespace-only) key are dropped —
 * that's the form's "empty row" affordance for adding/removing
 * key/value pairs without submitting placeholder junk.
 */
export function kvToMap(rows: KV[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const { key, value } of rows) {
    const k = key.trim();
    if (k) out[k] = value;
  }
  return out;
}

/** Assembles the full form state into an `InferenceRuleDto`-shaped object. */
export function assembleRule(s: FormState): InferenceRuleDto {
  return {
    id: s.id.trim(),
    confidence: Number(s.confidence),
    window_secs: Number(s.window_secs),
    trigger: {
      event_type: s.trigger.event_type,
      field_equals: kvToMap(s.trigger.field_equals),
    },
    followups: s.followups.map((f) => ({
      event_type: f.event_type,
      field_equals: kvToMap(f.field_equals),
    })),
    emits: {
      event_type: s.emit.event_type,
      fields: kvToMap(s.emit.fields),
    },
  };
}
