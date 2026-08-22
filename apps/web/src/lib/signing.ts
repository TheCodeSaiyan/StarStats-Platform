/**
 * Facts about how the Windows installers are code-signed, in one place
 * because two separate surfaces tell the user how to verify a download
 * (`/downloads` and `/docs`) and they must never drift apart.
 *
 * Since Azure Trusted Signing went live the certificate is CA-issued and
 * publicly trusted, so Windows names the publisher instead of saying
 * "Unknown publisher". The signing identity is configured in
 * `.github/workflows/release.yml` (signing path A) — see
 * `docs/RUNBOOK-ARTIFACT-SIGNING.md`.
 */

/** Organisation Windows shows as the verified publisher. */
export const CODE_SIGNING_PUBLISHER = 'TheCodeSaiyan Ltd';

/**
 * Full subject distinguished name on the certificate, as rendered by the
 * Windows signature-details dialog. Shown verbatim so a user comparing
 * the two sees an exact match rather than a paraphrase.
 */
export const CODE_SIGNING_SUBJECT_DN =
  'CN=TheCodeSaiyan Ltd, O=TheCodeSaiyan Ltd, L="Eton Wick, Windsor", C=GB';

/**
 * Why we publish the publisher NAME and deliberately do not publish a
 * certificate thumbprint.
 *
 * Azure Trusted Signing issues short-lived certificates and rotates them
 * continuously — a thumbprint printed on this page would be stale within
 * days, and a user who checked it against a newer (perfectly valid)
 * release would conclude the download had been tampered with. The stable
 * identity across every rotation is the subject name, so that is what we
 * ask people to check. The previous self-signed certificate did not
 * rotate, which is why the old copy could publish one.
 */
export const THUMBPRINT_OMITTED_REASON =
  'Trusted Signing rotates certificates continuously, so a published thumbprint would go stale; the publisher name is stable across rotations.';
