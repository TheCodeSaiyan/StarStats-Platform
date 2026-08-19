/**
 * ChartCard — MetricCard variant tuned for chart layouts.
 *
 * Same required props as MetricCard. Adds `height` for the chart area
 * (recharts needs an explicit pixel height inside a flex parent).
 */

'use client';

import type { ReactNode } from 'react';
import { MetricCard, type MetricCardProps } from './MetricCard';

export interface ChartCardProps extends Omit<MetricCardProps, 'children'> {
  height?: number;
  children: ReactNode;
}

export function ChartCard(props: ChartCardProps) {
  const { height = 220, children, ...rest } = props;
  return (
    <MetricCard {...rest}>
      <div className="chart-card__chart" style={{ height }}>
        {children}
      </div>
    </MetricCard>
  );
}
