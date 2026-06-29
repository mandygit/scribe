import type { TranscriptStreamEvent, TranscriptStreamSummary } from '../tauri-commands';
import { formatDuration, formatNumber } from './formatting';

interface LiveTranscriptPanelProps {
  streamedSegments: TranscriptStreamEvent[];
  streamSummary: TranscriptStreamSummary | null;
  maxVisibleSegments: number;
}

export const LiveTranscriptPanel = ({
  streamedSegments,
  streamSummary,
  maxVisibleSegments,
}: LiveTranscriptPanelProps) => (
  <section className="stream-panel" aria-labelledby="stream-title">
    <div className="metrics-panel-header">
      <div>
        <span className="status-label">Transcript replay</span>
        <h2 id="stream-title">Transcript timeline</h2>
        <p>
          Shows the most recent {maxVisibleSegments} transcript events replayed while the saved session is reviewed.
        </p>
      </div>
      <span className="metrics-state">
        {streamSummary
          ? `${streamSummary.segmentCount} final segment(s)`
          : streamedSegments.length > 0
            ? 'Receiving'
            : 'Idle'}
      </span>
    </div>

    {streamedSegments.length > 0 ? (
      <ol className="stream-list" aria-live="polite">
        {streamedSegments.map((event) => (
          <li key={`${event.meetingId}-${event.segment.sequenceNumber}`}>
            <span>{formatDuration(event.segment.startedAtMs)}</span>
            <p>{event.segment.text}</p>
          </li>
        ))}
      </ol>
    ) : (
      <div className="metrics-empty compact" role="status">
        <span aria-hidden="true">v</span>
        <p>Stop a session to replay transcript segments here while the review pipeline catches up.</p>
      </div>
    )}

    {streamSummary && streamSummary.droppedEventCount > 0 ? (
      <div className="score-warning compact" role="status">
        <strong>Stream backpressure applied</strong>
        <p>
          Dropped {formatNumber(streamSummary.droppedEventCount)} UI event(s), while the persisted transcript stayed
          complete.
        </p>
      </div>
    ) : null}
  </section>
);
