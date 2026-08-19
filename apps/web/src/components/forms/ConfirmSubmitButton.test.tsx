import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/react';

import { ConfirmSubmitButton } from './ConfirmSubmitButton';

afterEach(cleanup);

describe('ConfirmSubmitButton', () => {
  it('submits the form when no confirm is set', () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <ConfirmSubmitButton>Go</ConfirmSubmitButton>
      </form>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Go' }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('blocks the submit when window.confirm is declined', () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(
      <form onSubmit={onSubmit}>
        <ConfirmSubmitButton confirm="Sure?">Delete</ConfirmSubmitButton>
      </form>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(confirmSpy).toHaveBeenCalledWith('Sure?');
    expect(onSubmit).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it('allows the submit when window.confirm is accepted', () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(
      <form onSubmit={onSubmit}>
        <ConfirmSubmitButton confirm="Sure?">Delete</ConfirmSubmitButton>
      </form>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
    confirmSpy.mockRestore();
  });

  it('passes name/value through for multi-button forms', () => {
    render(
      <form>
        <ConfirmSubmitButton name="outcome" value="user_suspended">
          Suspend
        </ConfirmSubmitButton>
      </form>,
    );
    const btn = screen.getByRole('button', {
      name: 'Suspend',
    }) as HTMLButtonElement;
    expect(btn.name).toBe('outcome');
    expect(btn.value).toBe('user_suspended');
    expect(btn.type).toBe('submit');
  });
});
