import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import DictationPill from './DictationPill';
import MeetingPopup from './MeetingPopup';
import RecordingIndicator from './RecordingIndicator';

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Root element not found');
}

// Each floating window (dictation pill, meeting popup, recording indicator)
// renders in its own transparent Tauri window via `index.html?view=<name>`.
// All windows share this entry, so Vite bundles every view's stylesheet into
// each. The `<name>-view` class on <html> gates that view's own page-level
// rules (transparency, centering) so they only apply in that window and
// never leak into the main Scribe window.
const VIEWS = {
  pill: { className: 'pill-view', Component: DictationPill },
  'meeting-popup': { className: 'meeting-popup-view', Component: MeetingPopup },
  'recording-indicator': { className: 'recording-indicator-view', Component: RecordingIndicator },
} as const;

const view = new URLSearchParams(window.location.search).get('view');
const matchedView = view !== null && view in VIEWS ? VIEWS[view as keyof typeof VIEWS] : null;
if (matchedView) {
  document.documentElement.classList.add(matchedView.className);
}

const ViewComponent = matchedView?.Component ?? App;

createRoot(rootElement).render(
  <StrictMode>
    <ViewComponent />
  </StrictMode>,
);
