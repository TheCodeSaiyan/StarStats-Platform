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

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
