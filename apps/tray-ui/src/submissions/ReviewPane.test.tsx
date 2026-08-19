import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ReviewPane, type UnknownShape } from './ReviewPane';
import type { PiiToken } from './PiiToggle';

function shape(
  hash: string,
  interest: number,
  count: number,
  pii: PiiToken[] = []
): UnknownShape {
  return {
    shape_hash: hash,
    raw_example: `raw_${hash}`,
    interest_score: interest,
    occurrence_count: count,
    detected_pii: pii,
  };
}

describe('ReviewPane', () => {
  it('lists shapes sorted by interest × occurrence', () => {
    render(
      <ReviewPane
        shapes={[shape('a', 60, 1), shape('b', 80, 3), shape('c', 50, 5)]}
        onSubmit={() => {}}
        onDismiss={() => {}}
      />
    );
    const rows = screen.getAllByTestId('shape-row');
    // c: 50*5=250, b: 80*3=240, a: 60*1=60
    expect(rows[0]).toHaveTextContent('c');
    expect(rows[1]).toHaveTextContent('b');
    expect(rows[2]).toHaveTextContent('a');
  });

  it('renders empty state when no shapes', () => {
    render(<ReviewPane shapes={[]} onSubmit={() => {}} onDismiss={() => {}} />);
    expect(screen.getByText(/no unknown lines/i)).toBeInTheDocument();
  });

  it('submit invokes onSubmit with redacted raw_example', async () => {
    const pii: PiiToken[] = [
      {
        kind: 'own_handle',
        start: 0,
        end: 3,
        suggested_redaction: '[H]',
        default_redact: true,
      },
    ];
    const s = shape('x', 60, 1, pii);
    s.raw_example = 'Jim is here';
    const onSubmit = vi.fn();
    render(
      <ReviewPane shapes={[s]} onSubmit={onSubmit} onDismiss={() => {}} />
    );
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        shape_hash: 'x',
        raw_example: '[H] is here',
      })
    );
  });

  it('flipping a PII toggle off sends raw_example without redaction', async () => {
    const pii: PiiToken[] = [
      {
        kind: 'own_handle',
        start: 0,
        end: 3,
        suggested_redaction: '[H]',
        default_redact: true,
      },
    ];
    const s = shape('x', 60, 1, pii);
    s.raw_example = 'Jim is here';
    const onSubmit = vi.fn();
    render(
      <ReviewPane shapes={[s]} onSubmit={onSubmit} onDismiss={() => {}} />
    );
    await userEvent.click(screen.getByRole('checkbox'));
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        raw_example: 'Jim is here',
      })
    );
  });

  it('dismiss invokes onDismiss with the shape hash', async () => {
    const onDismiss = vi.fn();
    render(
      <ReviewPane
        shapes={[shape('x', 60, 1)]}
        onSubmit={() => {}}
        onDismiss={onDismiss}
      />
    );
    await userEvent.click(screen.getByRole('button', { name: /dismiss/i }));
    expect(onDismiss).toHaveBeenCalledWith('x');
  });

  it('passes suggested_event_name and notes through onSubmit', async () => {
    const onSubmit = vi.fn();
    render(
      <ReviewPane
        shapes={[shape('x', 60, 1)]}
        onSubmit={onSubmit}
        onDismiss={() => {}}
      />
    );
    await userEvent.type(
      screen.getByPlaceholderText(/suggested event name/i),
      'actor_death'
    );
    await userEvent.type(
      screen.getByPlaceholderText(/notes for the rule author/i),
      'spotted in PU'
    );
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        suggested_event_name: 'actor_death',
        notes: 'spotted in PU',
      })
    );
  });

  it('disables submit until an attribution choice is made', async () => {
    const onSubmit = vi.fn();
    render(
      <ReviewPane
        shapes={[shape('x', 60, 1)]}
        onSubmit={onSubmit}
        onDismiss={() => {}}
      />
    );
    // Forced explicit choice: no radio is pre-selected, so the button
    // starts disabled and clicking it is a no-op.
    const submitBtn = screen.getByRole('button', { name: /submit/i });
    expect(submitBtn).toBeDisabled();
    await userEvent.click(submitBtn);
    expect(onSubmit).not.toHaveBeenCalled();
    // Picking either option unlocks Submit.
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    expect(submitBtn).toBeEnabled();
  });

  it('sends attributed: true when the attribute-to-me option is chosen', async () => {
    const onSubmit = vi.fn();
    render(
      <ReviewPane
        shapes={[shape('x', 60, 1)]}
        onSubmit={onSubmit}
        onDismiss={() => {}}
      />
    );
    await userEvent.click(screen.getByRole('radio', { name: /attribute/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ attributed: true })
    );
  });

  it('sends attributed: false when the anonymous option is chosen', async () => {
    const onSubmit = vi.fn();
    render(
      <ReviewPane
        shapes={[shape('x', 60, 1)]}
        onSubmit={onSubmit}
        onDismiss={() => {}}
      />
    );
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ attributed: false })
    );
  });

  it('labels the attribute option with the handle when provided', () => {
    render(
      <ReviewPane
        shapes={[shape('x', 60, 1)]}
        onSubmit={() => {}}
        onDismiss={() => {}}
        handle="Daisy"
      />
    );
    expect(
      screen.getByRole('radio', { name: /attribute to @Daisy/i })
    ).toBeInTheDocument();
  });
});
