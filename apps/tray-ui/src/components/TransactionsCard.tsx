/**
 * Compact Transactions card for the Logs pane — shows recent shop /
 * commodity request-response pairs derived from the raw event stream
 * by `starstats_core::pair_transactions`.
 *
 * Pulled on the same cadence as the rest of the Logs pane (10s tick)
 * but as its own polling effect so a single failure here doesn't
 * block timeline / storage refresh.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { api, type Transaction, type TransactionStatus } from '../api';
import { TrayCard } from './tray/primitives';

const REFRESH_MS = 10_000;
const LIMIT = 200;
const WINDOW_SECS = 30;

const STATUS_COLOR: Record<TransactionStatus, string> = {
  pending: 'var(--warn)',
  confirmed: 'var(--ok)',
  rejected: 'var(--danger)',
  timed_out: 'var(--danger)',
  submitted: 'var(--accent)',
};

function statusLabel(s: TransactionStatus): string {
  switch (s) {
    case 'pending':
      return 'Pending';
    case 'confirmed':
      return 'Confirmed';
    case 'rejected':
      return 'Rejected';
    case 'timed_out':
      return 'Timed out';
    case 'submitted':
      return 'Submitted';
  }
}

function kindLabel(k: Transaction['kind']): string {
  switch (k) {
    case 'shop':
      return 'Shop';
    case 'commodity_buy':
      return 'Commodity buy';
    case 'commodity_sell':
      return 'Commodity sell';
  }
}

const rowStyle: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: '110px 1fr 90px 80px',
  gap: 10,
  alignItems: 'baseline',
  padding: '5px 8px',
  fontSize: 12,
  borderBottom: '1px solid var(--surface-3)',
};

export function TransactionsCard() {
  const [txs, setTxs] = useState<Transaction[] | null>(null);

  useEffect(() => {
    const signal = { aborted: false };
    const refresh = async () => {
      try {
        const result = await api.listTransactions(LIMIT, WINDOW_SECS);
        if (!signal.aborted) setTxs(result);
      } catch {
        // Silent — we just don't render the card body.
      }
    };
    void refresh();
    const handle = window.setInterval(refresh, REFRESH_MS);
    return () => {
      signal.aborted = true;
      window.clearInterval(handle);
    };
  }, []);

  if (!txs) return null;
  if (txs.length === 0) return null;

  // Newest first for the timeline-style read.
  const recent = [...txs].reverse().slice(0, 25);

  return (
    <TrayCard
      title="Transactions"
      kicker={`${txs.length} request${txs.length === 1 ? '' : 's'}`}
    >
      <div style={{ display: 'flex', flexDirection: 'column' }}>
        {recent.map((tx, idx) => (
          <div
            key={`${tx.started_at}-${idx}`}
            style={
              idx === recent.length - 1
                ? { ...rowStyle, borderBottom: 'none' }
                : rowStyle
            }
          >
            <span style={{ color: 'var(--fg-dim)' }}>{kindLabel(tx.kind)}</span>
            <span style={{ color: 'var(--fg)' }}>
              {tx.item ?? '—'}
              {tx.quantity != null ? ` × ${tx.quantity}` : ''}
            </span>
            <span style={{ color: 'var(--fg-dim)' }}>
              {new Date(tx.started_at).toLocaleTimeString()}
            </span>
            <span
              style={{
                color: STATUS_COLOR[tx.status],
                fontWeight: 500,
                textAlign: 'right',
              }}
            >
              {statusLabel(tx.status)}
            </span>
          </div>
        ))}
      </div>
    </TrayCard>
  );
}
