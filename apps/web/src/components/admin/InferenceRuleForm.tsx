'use client';

/**
 * Structured author form for `PublishInferenceRuleRequest` (inference
 * rule publishing, Task 6). Mirrors the #3 "Publish rule" panel's
 * posture — a plain `<form action={…}>` server action, inline input
 * styling copied verbatim from that panel — but the request body here
 * is nested (a trigger pattern + N followup patterns + an emit
 * template, each with its own field_equals/fields map), so free-text
 * JSON entry would be error-prone. Instead the form holds structured
 * `FormState` in React state and serialises it into a single hidden
 * `definition` field via `assembleRule` (see `./inference-rule.ts`,
 * unit-tested there) — the server action only ever sees one opaque
 * JSON string, same wire contract as if the caller had hand-built the
 * request body.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component 500s with "React is not
// defined" under test without it (the prod Next build uses the
// automatic runtime and wouldn't need it).
import React, { useState } from 'react';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { assembleRule, type FormState, type KV, type PatternState } from './inference-rule';

function emptyPattern(): PatternState {
  return { event_type: '', field_equals: [] };
}

function emptyState(): FormState {
  return {
    id: '',
    confidence: '0.8',
    window_secs: '30',
    trigger: emptyPattern(),
    followups: [],
    emit: { event_type: '', fields: [] },
  };
}

/** Replaces the row at `index` with `next`, leaving the rest untouched. */
function replaceAt<T>(rows: T[], index: number, next: T): T[] {
  return rows.map((row, i) => (i === index ? next : row));
}

/** Returns a new array with the row at `index` removed. */
function removeAt<T>(rows: T[], index: number): T[] {
  return rows.filter((_, i) => i !== index);
}

function KvRows({
  rows,
  onChange,
  testIdPrefix,
}: {
  rows: KV[];
  onChange: (next: KV[]) => void;
  testIdPrefix: string;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {rows.map((row, i) => (
        <div key={i} style={{ display: 'flex', gap: 6 }}>
          <input
            type="text"
            value={row.key}
            placeholder="field"
            data-testid={`${testIdPrefix}-key-${i}`}
            className="hp-adminfield mono" style={{ flex: 1 }}
            onChange={(e) =>
              onChange(replaceAt(rows, i, { ...row, key: e.target.value }))
            }
          />
          <input
            type="text"
            value={row.value}
            placeholder="value"
            data-testid={`${testIdPrefix}-value-${i}`}
            className="hp-adminfield mono" style={{ flex: 1 }}
            onChange={(e) =>
              onChange(replaceAt(rows, i, { ...row, value: e.target.value }))
            }
          />
          <button
            type="button"
            className="ss-btn"
            data-testid={`${testIdPrefix}-remove-${i}`}
            style={{
              padding: '6px 10px',
              borderRadius: 0,
              border: '1px solid var(--border-strong)',
              background: 'var(--bg-elev)',
              color: 'var(--fg)',
              fontSize: 12,
              cursor: 'pointer',
            }}
            onClick={() => onChange(removeAt(rows, i))}
          >
            Remove
          </button>
        </div>
      ))}
      <button
        type="button"
        className="ss-btn"
        data-testid={`${testIdPrefix}-add`}
        style={{
          alignSelf: 'flex-start',
          padding: '6px 10px',
          borderRadius: 0,
          border: '1px solid var(--border-strong)',
          background: 'var(--bg-elev)',
          color: 'var(--fg)',
          fontSize: 12,
          cursor: 'pointer',
        }}
        onClick={() => onChange([...rows, { key: '', value: '' }])}
      >
        + Add field
      </button>
    </div>
  );
}

function EventTypeSelect({
  value,
  eventTypes,
  testId,
  onChange,
}: {
  value: string;
  eventTypes: string[];
  testId: string;
  onChange: (next: string) => void;
}) {
  return (
    <select
      value={value}
      required
      data-testid={testId}
      className="hp-adminfield"
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="">— select event type —</option>
      {eventTypes.map((et) => (
        <option key={et} value={et}>
          {et}
        </option>
      ))}
    </select>
  );
}

function PatternEditor({
  title,
  pattern,
  eventTypes,
  testIdPrefix,
  onChange,
}: {
  title: string;
  pattern: PatternState;
  eventTypes: string[];
  testIdPrefix: string;
  onChange: (next: PatternState) => void;
}) {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        padding: 10,
        border: '1px solid var(--border)',
        borderRadius: 0,
      }}
    >
      <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>
        {title}
      </span>
      <label className="hp-kvvalue">
        <span className="hp-kvlabel">event_type</span>
        <EventTypeSelect
          value={pattern.event_type}
          eventTypes={eventTypes}
          testId={`${testIdPrefix}-event-type`}
          onChange={(event_type) => onChange({ ...pattern, event_type })}
        />
      </label>
      <label className="hp-kvvalue">
        <span className="hp-kvlabel">field_equals</span>
        <KvRows
          rows={pattern.field_equals}
          testIdPrefix={`${testIdPrefix}-field-equals`}
          onChange={(field_equals) => onChange({ ...pattern, field_equals })}
        />
      </label>
    </div>
  );
}

export function InferenceRuleForm({
  eventTypes,
  action,
}: {
  eventTypes: string[];
  action: (formData: FormData) => void | Promise<void>;
}) {
  const [state, setState] = useState<FormState>(() => emptyState());

  const definition = JSON.stringify(assembleRule(state));

  return (
    <form
      action={action}
      data-testid="inference-rule-form"
      style={{ display: 'flex', flexDirection: 'column', gap: 16 }}
    >
      <input type="hidden" name="definition" value={definition} />

      <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
        <label className="hp-kvvalue" style={{ flex: '1 1 220px' }}>
          <span className="hp-kvlabel">id</span>
          <input
            type="text"
            required
            pattern="[A-Za-z0-9_.\-]+"
            value={state.id}
            placeholder="e.g. combat.kill_streak"
            data-testid="inference-rule-id-input"
            className="hp-adminfield mono"
            onChange={(e) => setState({ ...state, id: e.target.value })}
          />
        </label>
        <label className="hp-kvvalue" style={{ flex: '0 1 140px' }}>
          <span className="hp-kvlabel">confidence</span>
          <input
            type="number"
            step="0.01"
            min={0}
            max={1}
            required
            value={state.confidence}
            data-testid="inference-rule-confidence-input"
            className="hp-adminfield"
            onChange={(e) => setState({ ...state, confidence: e.target.value })}
          />
        </label>
        <label className="hp-kvvalue" style={{ flex: '0 1 140px' }}>
          <span className="hp-kvlabel">window_secs</span>
          <input
            type="number"
            step="1"
            min={1}
            required
            value={state.window_secs}
            data-testid="inference-rule-window-secs-input"
            className="hp-adminfield"
            onChange={(e) => setState({ ...state, window_secs: e.target.value })}
          />
        </label>
      </div>

      <PatternEditor
        title="Trigger"
        pattern={state.trigger}
        eventTypes={eventTypes}
        testIdPrefix="inference-rule-trigger"
        onChange={(trigger) => setState({ ...state, trigger })}
      />

      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>
          Followups
        </span>
        {state.followups.map((followup, i) => (
          <div key={i} style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <PatternEditor
              title={`Followup ${i + 1}`}
              pattern={followup}
              eventTypes={eventTypes}
              testIdPrefix={`inference-rule-followup-${i}`}
              onChange={(next) =>
                setState({
                  ...state,
                  followups: replaceAt(state.followups, i, next),
                })
              }
            />
            <button
              type="button"
              className="ss-btn"
              data-testid={`inference-rule-followup-remove-${i}`}
              style={{
                alignSelf: 'flex-start',
                padding: '6px 10px',
                borderRadius: 0,
                border: '1px solid var(--border-strong)',
                background: 'var(--bg-elev)',
                color: 'var(--fg)',
                fontSize: 12,
                cursor: 'pointer',
              }}
              onClick={() =>
                setState({
                  ...state,
                  followups: removeAt(state.followups, i),
                })
              }
            >
              Remove followup {i + 1}
            </button>
          </div>
        ))}
        <button
          type="button"
          className="ss-btn"
          data-testid="inference-rule-followup-add"
          style={{
            alignSelf: 'flex-start',
            padding: '6px 10px',
            borderRadius: 0,
            border: '1px solid var(--border-strong)',
            background: 'var(--bg-elev)',
            color: 'var(--fg)',
            fontSize: 12,
            cursor: 'pointer',
          }}
          onClick={() =>
            setState({ ...state, followups: [...state.followups, emptyPattern()] })
          }
        >
          + Add followup
        </button>
      </div>

      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
          padding: 10,
          border: '1px solid var(--border)',
          borderRadius: 0,
        }}
      >
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>
          Emit
        </span>
        <label className="hp-kvvalue">
          <span className="hp-kvlabel">event_type</span>
          <EventTypeSelect
            value={state.emit.event_type}
            eventTypes={eventTypes}
            testId="inference-rule-emit-event-type"
            onChange={(event_type) =>
              setState({ ...state, emit: { ...state.emit, event_type } })
            }
          />
        </label>
        <label className="hp-kvvalue">
          <span className="hp-kvlabel">
            fields (values may reference{' '}
            <code>{'${trigger.<field>}'}</code> /{' '}
            <code>{'${followups.<n>.<field>}'}</code>)
          </span>
          <KvRows
            rows={state.emit.fields}
            testIdPrefix="inference-rule-emit-fields"
            onChange={(fields) =>
              setState({ ...state, emit: { ...state.emit, fields } })
            }
          />
        </label>
      </div>

      <ConfirmSubmitButton
        confirm="Publish this inference rule to all collectors?"
        pendingLabel="Publishing…"
        data-testid="inference-rule-submit"
        className="ss-btn ss-btn--primary"
        style={{
          alignSelf: 'flex-start',
          padding: '8px 16px',
          borderRadius: 0,
          fontSize: 13,
        }}
      >
        Publish rule
      </ConfirmSubmitButton>
    </form>
  );
}
