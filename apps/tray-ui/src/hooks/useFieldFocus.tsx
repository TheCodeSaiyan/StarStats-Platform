import { createContext, useCallback, useContext, useRef, type ReactNode } from 'react';
import type { SettingsField } from '../api';

type FieldRefMap = Partial<Record<SettingsField, HTMLElement | null>>;

interface FieldFocusContext {
  register: (field: SettingsField, el: HTMLElement | null) => void;
  focus: (field: SettingsField) => void;
}

const Ctx = createContext<FieldFocusContext | null>(null);

export function FieldFocusProvider({ children }: { children: ReactNode }) {
  const refs = useRef<FieldRefMap>({});
  // Bumped on every focus() call; any in-flight RAF loop carrying an
  // older value aborts on its next tick rather than fighting with a
  // newer target.
  const focusToken = useRef(0);

  const register = useCallback((field: SettingsField, el: HTMLElement | null) => {
    refs.current[field] = el;
  }, []);

  const focus = useCallback((field: SettingsField) => {
    // The caller typically does `setView('settings')` immediately
    // before calling focus(). React has to commit + mount
    // SettingsPane before our ref is registered, so we retry on
    // animation frames up to a small bound. Bounded so a missing
    // ref doesn't busy-loop forever.
    const myToken = ++focusToken.current;
    let attempts = 0;
    const tryFocus = () => {
      if (myToken !== focusToken.current) return; // superseded
      const el = refs.current[field];
      if (el) {
        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
        const input = el.querySelector<HTMLElement>('input, select, textarea, button');
        (input ?? el).focus();
        return;
      }
      if (attempts++ < 10) {
        window.requestAnimationFrame(tryFocus);
      }
    };
    window.requestAnimationFrame(tryFocus);
  }, []);

  return <Ctx.Provider value={{ register, focus }}>{children}</Ctx.Provider>;
}

export function useFieldFocus(): FieldFocusContext {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error('useFieldFocus must be used inside <FieldFocusProvider>');
  return ctx;
}
