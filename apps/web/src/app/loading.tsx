import { PageSkeleton } from '@/components/shell/PageSkeleton';

/**
 * Root loading skeleton. Rendered by Next.js while a route segment's
 * server-side data is in flight. Pure CSS pulse, no JS animation library,
 * framed by `PageSkeleton` so it wears the projection rather than drawing
 * bare into the layout.
 */

export default function Loading() {
  return (
    <PageSkeleton label="Loading…">
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
    </PageSkeleton>
  );
}
