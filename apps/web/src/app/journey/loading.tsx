import { PageSkeleton } from '@/components/shell/PageSkeleton';

export default function Loading() {
  return <PageSkeleton label="Loading timeline…" cards={5} />;
}
