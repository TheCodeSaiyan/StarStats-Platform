import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The `__APP_VERSION__` build-time define used to live here, sourced
// from this app's `package.json`. It was removed because the workspace
// version of record lives in the root `Cargo.toml`, and the React
// header now fetches that via `api.getAppVersion()` (Tauri command
// returning `env!("CARGO_PKG_VERSION")`). Keeping a parallel
// `package.json` version field would just be a second thing to bump
// on every release — and we'd inevitably forget.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: 'localhost',
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'esnext',
    minify: 'esbuild',
    sourcemap: true,
  },
});
