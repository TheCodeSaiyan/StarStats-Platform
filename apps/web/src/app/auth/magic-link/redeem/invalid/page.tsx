import Link from 'next/link';

export const metadata = { title: 'Magic link' };

/**
 * Where the magic-link redeem handler (`../route.ts`) sends a link it
 * could not use: no token at all, or one that is invalid, expired or
 * already clicked.
 */
export default async function MagicLinkInvalidPage(props: {
  searchParams: Promise<{ reason?: string }>;
}) {
  const { reason } = await props.searchParams;

  if (reason === 'missing') {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Sign-in link</span>
          <h1>Missing sign-in token.</h1>
          <p className="hp-authsub">
            The link is incomplete. Open the email we sent you and click the
            link from there.
          </p>
          <Link href="/auth/magic-link" className="ss-btn ss-btn--primary">
            Request a new link
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        <span className="ss-eyebrow">Sign-in link</span>
        <h1>Sign-in link invalid or expired.</h1>
        <p className="hp-authsub">
          This link can&apos;t be used. It may have expired (links are good
          for 15 minutes), already been clicked, or never have been issued.
          Request a new one to try again.
        </p>
        <div className="ss-alert ss-alert--warn" role="alert">
          Old links are invalidated automatically when a newer one is
          requested.
        </div>
        <Link href="/auth/magic-link" className="ss-btn ss-btn--primary">
          Request a new link
        </Link>
      </div>
    </div>
  );
}
