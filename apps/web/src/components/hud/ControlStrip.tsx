import React from 'react';

export function ControlStrip({ children }: { children: React.ReactNode }) {
  return <div className="hud-controls">{children}</div>;
}
