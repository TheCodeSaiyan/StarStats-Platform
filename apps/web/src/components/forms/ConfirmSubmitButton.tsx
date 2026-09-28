'use client';

/**
 * Submit button for destructive / mutating server-action forms (M-W11).
 *
 * Two guarantees the raw `<button className="ss-btn ss-btn--danger">`
 * pattern lacked:
 *   1. Optional confirmation — pass `confirm="…"` and a native
 *      `window.confirm()` gates the submit, so single-click destructive
 *      actions (revoke device, suspend owner, revoke share) can't fire
 *      by accident.
 *   2. Pending state — mirrors `RefreshSubmitButton`: `useFormStatus`
 *      disables the button and sets `aria-busy` while the action is in
 *      flight, killing double-submit and giving feedback before the
 *      redirect lands.
 *
 * MUST be rendered inside a `<form action={…}>`. `name`/`value` pass
 * through so it works as one of several submit buttons in a shared form
 * (e.g. the admin share-report Dismiss / Revoke / Suspend row).
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component 500s with "React is not
// defined" under test without it (the prod Next build uses the
// automatic runtime and wouldn't need it).
import React, { type ButtonHTMLAttributes, type ReactNode } from 'react';
import { useFormStatus } from 'react-dom';

type Props = {
  children: ReactNode;
  /** Shown in place of children while the action is pending. */
  pendingLabel?: ReactNode;
  /** When set, `window.confirm(confirm)` must pass before the submit. */
  confirm?: string;
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'type' | 'disabled'>;

export function ConfirmSubmitButton({
  children,
  pendingLabel,
  confirm,
  className = 'ss-btn ss-btn--danger',
  onClick,
  ...rest
}: Props) {
  const { pending } = useFormStatus();
  return (
    <button
      {...rest}
      type="submit"
      className={className}
      disabled={pending}
      aria-busy={pending || undefined}
      onClick={(e) => {
        if (confirm && !window.confirm(confirm)) {
          e.preventDefault();
          return;
        }
        // A form action reads `new FormData(form)`, which leaves out the
        // submitting button, and this button disables itself while the
        // action is pending, which would drop it anyway. So a named
        // button writes its name/value into a hidden field before the
        // submit, or multi-outcome forms (the moderation queues) would
        // submit with no outcome at all and quietly do nothing.
        const form = e.currentTarget.form;
        if (form && rest.name) {
          let field = form.querySelector<HTMLInputElement>('input[data-confirm-submitter]');
          if (!field) {
            field = document.createElement('input');
            field.type = 'hidden';
            field.setAttribute('data-confirm-submitter', '');
            form.appendChild(field);
          }
          field.name = rest.name;
          field.value = String(rest.value ?? '');
        }
        onClick?.(e);
      }}
    >
      {pending && pendingLabel ? pendingLabel : children}
    </button>
  );
}
