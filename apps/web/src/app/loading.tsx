/**
 * Root loading skeleton. Rendered by Next.js while a route segment's
 * server-side data is in flight. Server Component — pure CSS pulse,
 * no JS animation library.
 */

export default function Loading() {
  return (
    <main aria-busy="true" aria-label="Loading">
      <div
        className="skeleton"
        style={{ height: 32, width: '60%', marginBottom: 24 }}
      />
      <div
        className="skeleton"
        style={{ height: 16, width: '90%', marginBottom: 12 }}
      />
      <div
        className="skeleton"
        style={{ height: 16, width: '75%' }}
      />
    </main>
  );
}
