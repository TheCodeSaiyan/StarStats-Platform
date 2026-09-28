/**
 * What one timeline event shows as, in plain terms. Messages are rendered
 * as plain text only: `formatted_body` (HTML) is never used, so no message
 * can put markup on the page. Links stay text until the on-device link
 * warnings exist.
 */
export type ChatLine =
  | { kind: 'text'; sender: string; text: string; ts: number; id: string }
  | { kind: 'undecryptable'; sender: string; ts: number; id: string }
  | { kind: 'hidden' };

/** The subset of a matrix-js-sdk MatrixEvent this needs. */
export interface EventLike {
  getId(): string | undefined;
  getType(): string;
  getSender(): string | undefined;
  getTs(): number;
  getContent(): Record<string, unknown>;
  isDecryptionFailure(): boolean;
  isRedacted(): boolean;
}

export function toLine(ev: EventLike): ChatLine {
  const id = ev.getId() ?? '';
  const sender = ev.getSender() ?? '';
  const ts = ev.getTs();
  if (ev.isRedacted()) return { kind: 'hidden' };
  if (ev.isDecryptionFailure()) return { kind: 'undecryptable', sender, ts, id };
  if (ev.getType() !== 'm.room.message') return { kind: 'hidden' };
  const content = ev.getContent();
  const msgtype = content.msgtype;
  const body = content.body;
  if ((msgtype === 'm.text' || msgtype === 'm.emote' || msgtype === 'm.notice') && typeof body === 'string') {
    return { kind: 'text', sender, text: body, ts, id };
  }
  // Attachments and anything else: not in v1, so not shown.
  return { kind: 'hidden' };
}

/** `@wing_man:starstats.app` -> `wing_man`. */
export function localpartOf(userId: string): string {
  const m = /^@([^:]+):/.exec(userId);
  return m ? m[1] : userId;
}

/** The longest message the composer accepts. */
export const MAX_MESSAGE_CHARS = 2000;
