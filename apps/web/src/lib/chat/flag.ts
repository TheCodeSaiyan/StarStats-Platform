/**
 * Chat's launch switch, read at RUNTIME from the container env (like
 * `STARSTATS_NOINDEX`), so one image serves every stage:
 *
 *   STARSTATS_CHAT_ENABLED=off    (default) no chat anywhere in the web app
 *   STARSTATS_CHAT_ENABLED=staff  staff only, for testing in production
 *                                 before launch
 *   STARSTATS_CHAT_ENABLED=on     everyone
 *
 * The API gates chat on its own (verified handle, age declaration, no
 * restriction); this only decides whether the web shows it at all.
 */
export type ChatMode = 'off' | 'staff' | 'on';

export function chatMode(env: string | undefined = process.env.STARSTATS_CHAT_ENABLED): ChatMode {
  return env === 'on' || env === 'staff' ? env : 'off';
}

export function chatEnabledFor(
  session: { staffRoles: string[] } | null,
  mode: ChatMode = chatMode(),
): boolean {
  if (!session || mode === 'off') return false;
  return mode === 'on' || session.staffRoles.length > 0;
}
