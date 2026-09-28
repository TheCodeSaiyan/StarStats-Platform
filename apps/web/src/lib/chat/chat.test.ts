import { describe, expect, it } from 'vitest';
import { chatEnabledFor, chatMode } from './flag';
import { localpartOf, toLine, type EventLike } from './render';
import { cryptoStorePrefix, parseStoredSession, sessionFor } from './session';

const ev = (over: Partial<Record<keyof EventLike, unknown>> & { content?: Record<string, unknown> }): EventLike => ({
  getId: () => '$e1',
  getType: () => (over.getType as string) ?? 'm.room.message',
  getSender: () => '@wingman:starstats.app',
  getTs: () => 1000,
  getContent: () => over.content ?? { msgtype: 'm.text', body: 'o7' },
  isDecryptionFailure: () => (over.isDecryptionFailure as boolean) ?? false,
  isRedacted: () => (over.isRedacted as boolean) ?? false,
});

describe('chat flag', () => {
  it('is off unless set to staff or on', () => {
    expect(chatMode(undefined)).toBe('off');
    expect(chatMode('yes')).toBe('off');
    expect(chatMode('staff')).toBe('staff');
    expect(chatMode('on')).toBe('on');
  });

  it('staff mode shows chat to staff only, and nobody signed out sees it', () => {
    const player = { staffRoles: [] };
    const mod = { staffRoles: ['moderator'] };
    expect(chatEnabledFor(player, 'staff')).toBe(false);
    expect(chatEnabledFor(mod, 'staff')).toBe(true);
    expect(chatEnabledFor(player, 'on')).toBe(true);
    expect(chatEnabledFor(null, 'on')).toBe(false);
    expect(chatEnabledFor(mod, 'off')).toBe(false);
  });
});

describe('stored chat session', () => {
  const stored = JSON.stringify({
    handle: 'wingman',
    userId: '@wingman:starstats.app',
    deviceId: 'D1',
    accessToken: 't',
    homeserverUrl: 'https://api.example/',
  });

  it('is used only for the account that made it', () => {
    expect(sessionFor(stored, 'Wingman')?.deviceId).toBe('D1');
    expect(sessionFor(stored, 'someone_else')).toBeNull();
  });

  it('treats corrupt or partial data as absent', () => {
    expect(parseStoredSession('{not json')).toBeNull();
    expect(parseStoredSession(JSON.stringify({ handle: 'x' }))).toBeNull();
    expect(parseStoredSession(null)).toBeNull();
  });

  it('keeps each user a separate encryption store', () => {
    expect(cryptoStorePrefix('@a:s')).not.toBe(cryptoStorePrefix('@b:s'));
  });
});

describe('timeline rendering', () => {
  it('shows text messages as plain text, never their HTML', () => {
    const line = toLine(
      ev({ content: { msgtype: 'm.text', body: '<b>hi</b>', format: 'org.matrix.custom.html', formatted_body: '<img src=x onerror=alert(1)>' } }),
    );
    expect(line).toEqual({ kind: 'text', sender: '@wingman:starstats.app', text: '<b>hi</b>', ts: 1000, id: '$e1' });
  });

  it('marks what could not be decrypted instead of hiding it', () => {
    expect(toLine(ev({ isDecryptionFailure: true })).kind).toBe('undecryptable');
  });

  it('hides state, redactions and attachments', () => {
    expect(toLine(ev({ getType: 'm.room.member' })).kind).toBe('hidden');
    expect(toLine(ev({ isRedacted: true })).kind).toBe('hidden');
    expect(toLine(ev({ content: { msgtype: 'm.image', body: 'x.png' } })).kind).toBe('hidden');
  });

  it('shows a player by their handle', () => {
    expect(localpartOf('@wing_man:starstats.app')).toBe('wing_man');
  });
});
