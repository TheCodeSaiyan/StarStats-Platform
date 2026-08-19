/**
 * Compose a public profile URL on the StarStats web app from the
 * configured `web_origin` and the user's claimed RSI handle.
 *
 * Returns `null` when either input is missing/empty so callers can
 * render a disabled "Open on web" affordance without conditional
 * string-building scattered through the JSX.
 *
 * The origin is normalised by trimming any trailing slashes; the
 * handle is URI-encoded so unusual characters (spaces, slashes, etc.)
 * don't produce a malformed URL or path-injection vector.
 */
export function composeProfileUrl(
  webOrigin: string | null,
  handle: string | null,
): string | null {
  if (!webOrigin || !handle) return null;
  const trimmedOrigin = webOrigin.replace(/\/+$/, '');
  if (!trimmedOrigin) return null;
  return `${trimmedOrigin}/u/${encodeURIComponent(handle)}`;
}
