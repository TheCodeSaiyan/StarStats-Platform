/**
 * One redaction checkbox for an auto-detected PII token in an unknown
 * line. The token carries its own `default_redact` flag (driven by
 * `PiiKind` — own-handle defaults true, friend-handles default false,
 * etc.) so the initial UI state mirrors what the core PII detector
 * recommends. Flipping the checkbox bubbles a boolean to the parent
 * so the parent can re-render the redacted preview.
 */

import { useState } from 'react';

export type PiiKind =
  | 'own_handle'
  | 'friend_handle'
  | 'shard_id'
  | 'geid'
  | 'ip_port';

export interface PiiToken {
  kind: PiiKind;
  start: number;
  end: number;
  suggested_redaction: string;
  default_redact: boolean;
}

interface Props {
  token: PiiToken;
  onChange: (redact: boolean) => void;
}

export function PiiToggle({ token, onChange }: Props) {
  const [redact, setRedact] = useState(token.default_redact);
  const label = labelFor(token.kind);
  return (
    <label className="pii-toggle" data-kind={token.kind}>
      <input
        type="checkbox"
        checked={redact}
        onChange={(e) => {
          const next = e.target.checked;
          setRedact(next);
          onChange(next);
        }}
      />
      <span>
        Redact {label} → {token.suggested_redaction}
      </span>
    </label>
  );
}

function labelFor(kind: PiiKind): string {
  switch (kind) {
    case 'own_handle':
      return 'your handle';
    case 'friend_handle':
      return 'friend handle';
    case 'shard_id':
      return 'shard id';
    case 'geid':
      return 'GEID';
    case 'ip_port':
      return 'IP / port';
  }
}
