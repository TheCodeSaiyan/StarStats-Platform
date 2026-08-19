import React from 'react';
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { ControlStrip } from './ControlStrip';

describe('ControlStrip', () => {
  it('wraps children in hud-controls', () => {
    const { container } = render(<ControlStrip><button>x</button></ControlStrip>);
    expect((container.firstChild as HTMLElement).className).toContain('hud-controls');
  });
});
