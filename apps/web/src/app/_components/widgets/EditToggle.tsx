'use client';
import React from 'react';
import { useEditMode } from './useEditMode';

export function EditToggle() {
  const { isEditing, setEditing } = useEditMode();
  return (
    <button
      type="button"
      className="hud-chip hud-chip--sm"
      aria-pressed={isEditing}
      aria-label={isEditing ? 'Exit edit mode' : 'Edit layout'}
      onClick={() => setEditing(!isEditing)}
    >
      {isEditing ? '✎ Done' : '✎ Edit'}
    </button>
  );
}
