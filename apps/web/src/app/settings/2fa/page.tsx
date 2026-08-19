import { redirect } from 'next/navigation';

export const metadata = { title: "Two-factor auth" };

/**
 * Legacy /settings/2fa redirect.
 *
 * The 2FA setup wizard was absorbed into the Settings page as an
 * inline Security section per the StarStats design audit v2 §07
 * ("/settings/2fa: absorb → Settings") and §09 ("Inline 2FA wizard
 * into Settings → Security"). This stub keeps old bookmarks, email
 * links, and outbound nav references working — anyone hitting the
 * route lands directly on the new Security card.
 */
export default async function TwoFactorPage() {
  redirect('/settings#security');
}
