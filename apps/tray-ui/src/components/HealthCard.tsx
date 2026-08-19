import type { HealthItem, HealthId, SettingsField } from '../api';
import { healthStrings } from '../lib/useHealthStrings';
import { TrayCard, GhostButton, StatusDot } from './tray/primitives';

interface Props {
  items: HealthItem[];
  onGoToSettings: (field: SettingsField) => void;
  onDismiss: (id: HealthId) => void;
  onRetrySync?: () => void;
  onRefreshHangar?: () => void;
  onOpenUrl?: (url: string) => void;
}

const SEVERITY_TONE: Record<HealthItem['severity'], 'danger' | 'warn' | 'info'> = {
  error: 'danger',
  warn: 'warn',
  info: 'info',
};

const CTA_LABEL: Record<string, string> = {
  go_to_settings: 'Set up',
  retry_sync: 'Retry sync',
  refresh_hangar: 'Refresh now',
  open_url: 'Open',
};

// Per-id label overrides. The default `go_to_settings` label is "Set
// up", which reads as first-time configuration. For `auth_lost` the
// device WAS paired and the server invalidated the token (typically:
// keypair regeneration on a stateless redeploy with no persistent
// volume) — "Re-pair" is the truthful affordance.
const CTA_LABEL_BY_ID: Partial<Record<HealthId, string>> = {
  auth_lost: 'Re-pair',
};

function ctaLabel(it: HealthItem): string {
  if (!it.action) return 'Fix';
  return CTA_LABEL_BY_ID[it.id] ?? CTA_LABEL[it.action.kind] ?? 'Fix';
}

export function HealthCard({
  items,
  onGoToSettings,
  onDismiss,
  onRetrySync,
  onRefreshHangar,
  onOpenUrl,
}: Props) {
  if (items.length === 0) return null;

  return (
    <TrayCard
      title="Health"
      kicker={`${items.length} issue${items.length === 1 ? '' : 's'}`}
    >
      <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
        {items.map((it) => {
          const strings = healthStrings(it.params);
          return (
            <li
              key={it.fingerprint}
              style={{
                display: 'grid',
                gridTemplateColumns: 'auto 1fr auto auto',
                alignItems: 'center',
                gap: 10,
                padding: '6px 8px',
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-sm)',
              }}
            >
              <StatusDot tone={SEVERITY_TONE[it.severity]} />
              <div style={{ display: 'flex', flexDirection: 'column' }}>
                <span style={{ fontSize: 13, color: 'var(--fg)' }}>{strings.summary}</span>
                {strings.detail && (
                  <span style={{ fontSize: 11, color: 'var(--fg-dim)' }}>{strings.detail}</span>
                )}
              </div>
              {it.action ? (
                <GhostButton
                  type="button"
                  onClick={() => {
                    const a = it.action!;
                    switch (a.kind) {
                      case 'go_to_settings':
                        onGoToSettings(a.field);
                        break;
                      case 'retry_sync':
                        onRetrySync?.();
                        break;
                      case 'refresh_hangar':
                        onRefreshHangar?.();
                        break;
                      case 'open_url':
                        onOpenUrl?.(a.url);
                        break;
                    }
                  }}
                  style={{ padding: '3px 9px', fontSize: 11 }}
                >
                  {ctaLabel(it)}
                </GhostButton>
              ) : (
                <span />
              )}
              {it.dismissible ? (
                <GhostButton
                  type="button"
                  onClick={() => onDismiss(it.id)}
                  style={{ padding: '3px 9px', fontSize: 11 }}
                  aria-label={`Dismiss ${it.id}`}
                >
                  Dismiss
                </GhostButton>
              ) : (
                <span />
              )}
            </li>
          );
        })}
      </ul>
    </TrayCard>
  );
}
