import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import DictationPill from './DictationPill';

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Root element not found');
}

// The dictation pill renders in its own transparent Tauri window via
// `index.html?view=pill`. Both windows share this entry, so Vite bundles both
// stylesheets into each. The `.pill-view` class on <html> gates pill.css's
// page-level rules (transparency, centering) so they only apply in the pill
// window and never leak into the main Scribe window.
const isPillView = new URLSearchParams(window.location.search).get('view') === 'pill';
if (isPillView) {
  document.documentElement.classList.add('pill-view');
}

createRoot(rootElement).render(<StrictMode>{isPillView ? <DictationPill /> : <App />}</StrictMode>);
