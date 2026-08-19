// Brand atom: four-point compass star.
// Two stacked polygons -- outer at currentColor, inner at reduced opacity --
// so callers control hue via CSS `color` and the two-tone effect still reads
// against any theme background. Source geometry: brand book §03 .ss-star
// clip-path polygons (assets/logo/starstats-mark.svg).
import React from 'react';

type CompassStarProps = {
  size?: number;
  className?: string;
  /** When set, the SVG is treated as a meaningful image with this label.
   *  When omitted, the SVG is presentational (aria-hidden). */
  label?: string;
};

export function CompassStar({ size = 24, className, label }: CompassStarProps) {
  const labelled = Boolean(label);
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 100 100"
      width={size}
      height={size}
      role={labelled ? 'img' : undefined}
      aria-label={labelled ? label : undefined}
      aria-hidden={labelled ? undefined : true}
      focusable="false"
      className={className}
      style={{ display: 'inline-block', flexShrink: 0 }}
    >
      <polygon
        points="50,0 56,44 100,50 56,56 50,100 44,56 0,50 44,44"
        fill="currentColor"
      />
      <polygon
        points="50,12 54,46 88,50 54,54 50,88 46,54 12,50 46,46"
        fill="currentColor"
        fillOpacity="0.5"
      />
    </svg>
  );
}
