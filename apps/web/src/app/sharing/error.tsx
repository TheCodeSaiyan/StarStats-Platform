'use client';

import { RouteError } from '@/components/shell/RouteError';

export default function Error(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return <RouteError {...props} title="Couldn’t load sharing" />;
}
