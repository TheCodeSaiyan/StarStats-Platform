import { redirect } from 'next/navigation';

export const metadata = { title: "Journey" };

// Superseded by /me + the focus lens (Mirror Plans 3/3b/4).
export default function JourneyRedirect() {
  redirect('/me');
}
