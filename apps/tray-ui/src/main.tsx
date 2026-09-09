import React from 'react';
import ReactDOM from 'react-dom/client';
// IBM Plex Sans (body) + IBM Plex Mono (figures) + Michroma (placards/
// eyebrows) are the shared design language's visual signature — matching the
// web app. Bundled via @fontsource so they ship in the Tauri webview's asset
// bundle (fetching from fonts.googleapis.com is blocked by the app's CSP
// `default-src 'self'`). Weights mirror apps/web/src/app/layout.tsx.
import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/500.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource/ibm-plex-sans/700.css';
import '@fontsource/ibm-plex-mono/400.css';
import '@fontsource/ibm-plex-mono/500.css';
import '@fontsource/ibm-plex-mono/600.css';
import '@fontsource/michroma/400.css';
import App from './App';
import { BootErrorBoundary } from './components/BootErrorBoundary';

// The boundary wraps App INSIDE StrictMode rather than outside it, so a
// throw from any pane is caught while StrictMode's double-invoke still
// applies in development. It only catches render errors — see the note in
// BootErrorBoundary for what a blank window still means.
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BootErrorBoundary>
      <App />
    </BootErrorBoundary>
  </React.StrictMode>
);
