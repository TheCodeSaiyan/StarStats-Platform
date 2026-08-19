import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';

// Tauri's invoke() is mocked per-test. Default behaviour rejects so
// any test that forgets to mock a call gets a loud failure rather
// than a silent timeout.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.reject(new Error('invoke() called without per-test mock'))),
}));
