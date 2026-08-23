/**
 * /uploads has been folded into /downloads (the Emitter) as a per-device "Activity"
 * tab. See StarStats Audit v2 §03 + §07: pairing identity and the
 * batch stream are two views of the same pipeline and belong together.
 *
 * This stub stays so existing bookmarks and inbound links resolve.
 */
import { redirect } from 'next/navigation';

export const metadata = { title: "Uploads" };

export default async function UploadsPage() {
  redirect('/downloads');
}
