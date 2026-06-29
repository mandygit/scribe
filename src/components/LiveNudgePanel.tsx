import type { LiveNudgeEvent } from '../tauri-commands';
import { NUDGE_CATEGORY_LABELS } from './labels';

interface LiveNudgePanelProps {
  visibleLiveNudges: LiveNudgeEvent[];
  liveNudges: LiveNudgeEvent[];
  onDismissNudge: (nudgeId: string) => void;
}

export const LiveNudgePanel = ({ visibleLiveNudges, liveNudges, onDismissNudge }: LiveNudgePanelProps) => (
  <section className="nudge-panel" aria-labelledby="nudge-title">
    <div className="metrics-panel-header">
      <div>
        <span className="status-label">Coach replay</span>
        <h2 id="nudge-title">Coaching nudges</h2>
        <p>Recent prompts generated from transcript-backed checks after a session stops.</p>
      </div>
      <span className="metrics-state">
        {visibleLiveNudges.length > 0 ? `${visibleLiveNudges.length} active nudge(s)` : 'Quiet'}
      </span>
    </div>

    {visibleLiveNudges.length > 0 ? (
      <ol className="nudge-list" aria-live="polite">
        {visibleLiveNudges.map((nudge) => (
          <li className={`nudge-card ${nudge.severity}`} key={nudge.id}>
            <div>
              <span>{NUDGE_CATEGORY_LABELS[nudge.category]}</span>
              <strong>{nudge.message}</strong>
              <p>{nudge.suggestion}</p>
            </div>
            <div className="nudge-evidence">
              <blockquote>{nudge.evidence}</blockquote>
              <button type="button" onClick={() => onDismissNudge(nudge.id)}>
                Dismiss
              </button>
            </div>
          </li>
        ))}
      </ol>
    ) : liveNudges.length > 0 ? (
      <div className="metrics-empty compact" role="status">
        <span aria-hidden="true">✓</span>
        <p>All current nudges are dismissed. New live coaching prompts will appear here.</p>
      </div>
    ) : (
      <div className="metrics-empty compact" role="status">
        <span aria-hidden="true">•</span>
        <p>Stop a session to replay filler, hedging, pace, and talk-time nudges from the saved transcript.</p>
      </div>
    )}
  </section>
);
