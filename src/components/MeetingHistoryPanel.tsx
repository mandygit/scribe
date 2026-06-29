import type { MeetingHistoryDetail, MeetingHistoryItem } from '../tauri-commands';
import { formatDateTime, formatDuration, formatScore } from './formatting';
import { HISTORY_STATUS_LABELS } from './labels';

const IMPORTED_SPEAKING_SOURCE_LABELS: Record<
  NonNullable<MeetingHistoryDetail['importedSummary']>['speakingImprovementsSource'],
  string
> = {
  none: 'none',
  mainSpeaker: 'main speaker fallback',
  voiceMatch: 'voice match',
};

interface MeetingHistoryPanelProps {
  historyMessage: string;
  historyItems: MeetingHistoryItem[];
  historyNextOffset: number | null;
  selectedHistoryDetail: MeetingHistoryDetail | null;
  historySearch: string;
  isHistoryLoading: boolean;
  onSearchChange: (value: string) => void;
  onLoadPage: (reset: boolean) => void;
  onSelectMeeting: (meetingId: string) => void;
}

export const MeetingHistoryPanel = ({
  historyMessage,
  historyItems,
  historyNextOffset,
  selectedHistoryDetail,
  historySearch,
  isHistoryLoading,
  onSearchChange,
  onLoadPage,
  onSelectMeeting,
}: MeetingHistoryPanelProps) => (
  <section className="history-panel" aria-labelledby="history-title">
    <div className="metrics-panel-header">
      <div>
        <span className="status-label">Local archive</span>
        <h2 id="history-title">Meeting history</h2>
        <p aria-live="polite">{historyMessage}</p>
      </div>
      <span className="metrics-state">{historyItems.length > 0 ? `${historyItems.length} loaded` : 'Empty'}</span>
    </div>

    <form
      className="history-search"
      onSubmit={(event) => {
        event.preventDefault();
        onLoadPage(true);
      }}
    >
      <label>
        Search local meetings
        <input
          type="search"
          value={historySearch}
          onChange={(event) => onSearchChange(event.currentTarget.value)}
          placeholder="Title, meeting id, or transcript text"
        />
      </label>
      <button type="submit" disabled={isHistoryLoading}>
        {isHistoryLoading ? 'Loading...' : 'Search'}
      </button>
      <button type="button" onClick={() => onLoadPage(true)} disabled={isHistoryLoading}>
        Refresh
      </button>
    </form>

    <div className="history-layout">
      <section className="history-list" aria-label="Stored meetings">
        {historyItems.length > 0 ? (
          <>
            {historyItems.map((meeting) => (
              <button
                type="button"
                className="history-row"
                key={meeting.meetingId}
                onClick={() => onSelectMeeting(meeting.meetingId)}
                data-active={selectedHistoryDetail?.meeting.meetingId === meeting.meetingId}
              >
                <span className="history-row-main">
                  <strong>{meeting.title ?? meeting.meetingId}</strong>
                  <span>{formatDateTime(meeting.startedAtMs)}</span>
                </span>
                <span className="history-row-meta">
                  <span>{HISTORY_STATUS_LABELS[meeting.status]}</span>
                  <span>{meeting.durationMs === null ? 'Duration pending' : formatDuration(meeting.durationMs)}</span>
                  <span>{meeting.latestReportScore === null ? 'No score' : `${meeting.latestReportScore}/100`}</span>
                </span>
              </button>
            ))}
            {historyNextOffset !== null ? (
              <button
                type="button"
                className="history-load-more"
                onClick={() => onLoadPage(false)}
                disabled={isHistoryLoading}
              >
                Load more
              </button>
            ) : null}
          </>
        ) : (
          <div className="metrics-empty compact" role="status">
            <span aria-hidden="true">?</span>
            <p>No local meetings loaded yet. Refresh after recording, transcription, or analysis.</p>
          </div>
        )}
      </section>

      <article className="history-detail" aria-label="Selected meeting detail">
        {selectedHistoryDetail ? (
          <>
            <div className="history-detail-header">
              <div>
                <span className="status-label">{HISTORY_STATUS_LABELS[selectedHistoryDetail.meeting.status]}</span>
                <h3>{selectedHistoryDetail.meeting.title ?? selectedHistoryDetail.meeting.meetingId}</h3>
                <p>
                  {formatDateTime(selectedHistoryDetail.meeting.startedAtMs)} ·{' '}
                  {selectedHistoryDetail.meeting.durationMs === null
                    ? 'duration pending'
                    : formatDuration(selectedHistoryDetail.meeting.durationMs)}
                </p>
              </div>
              <strong>
                {selectedHistoryDetail.report
                  ? formatScore(selectedHistoryDetail.report.scorecard.overall.score)
                  : 'No report'}
              </strong>
            </div>

            {selectedHistoryDetail.audioFilePath ? (
              <p className="recording-path">
                Mic audio: {selectedHistoryDetail.audioFilePath}
                {selectedHistoryDetail.systemAudioFilePath ? (
                  <>
                    <br />
                    System audio: {selectedHistoryDetail.systemAudioFilePath}
                  </>
                ) : null}
              </p>
            ) : null}

            {selectedHistoryDetail.pipelineFailure ? (
              <div className="score-warning compact" role="status">
                <strong>Pipeline stopped at {selectedHistoryDetail.pipelineFailure.failedStage}</strong>
                <p>
                  {selectedHistoryDetail.pipelineFailure.errorMessage} Raw audio and completed local artifacts were
                  preserved for retry.
                </p>
              </div>
            ) : null}

            {selectedHistoryDetail.report ? (
              <div className="history-report-summary">
                <span>Report {selectedHistoryDetail.report.reportId}</span>
                <strong>Overall {formatScore(selectedHistoryDetail.report.scorecard.overall.score)}</strong>
                <p>
                  {selectedHistoryDetail.report.analysis.observations.length} quote-grounded observation(s) saved
                  locally.
                </p>
              </div>
            ) : (
              <div className="score-warning compact" role="status">
                <strong>Report not ready</strong>
                <p>Run metrics and analysis for this meeting to attach a score card.</p>
              </div>
            )}

            {selectedHistoryDetail.importedSummary ? (
              <div className="history-report-summary">
                <span>Imported summary {selectedHistoryDetail.importedSummary.summaryId}</span>
                <strong>
                  Speaking coaching source:{' '}
                  {IMPORTED_SPEAKING_SOURCE_LABELS[selectedHistoryDetail.importedSummary.speakingImprovementsSource]}
                </strong>
                <p>{selectedHistoryDetail.importedSummary.summary.executiveSummary}</p>
              </div>
            ) : null}

            {selectedHistoryDetail.transcriptSegments.length > 0 ? (
              <>
                {selectedHistoryDetail.transcriptTruncated ? (
                  <div className="score-warning compact" role="status">
                    <strong>Transcript preview</strong>
                    <p>
                      Showing the first {selectedHistoryDetail.transcriptSegments.length} of{' '}
                      {selectedHistoryDetail.meeting.transcriptSegmentCount} segment(s) to keep the UI responsive.
                    </p>
                  </div>
                ) : null}
                <ol className="history-transcript">
                  {selectedHistoryDetail.transcriptSegments.map((segment) => (
                    <li key={`${selectedHistoryDetail.meeting.meetingId}-${segment.sequenceNumber}`}>
                      <span>{formatDuration(segment.startedAtMs)}</span>
                      <p>
                        {segment.speakerLabel ? <strong>{segment.speakerLabel}: </strong> : null}
                        {segment.text}
                      </p>
                    </li>
                  ))}
                </ol>
              </>
            ) : (
              <div className="metrics-empty compact" role="status">
                <span aria-hidden="true">-</span>
                <p>No transcript has been stored for this meeting yet.</p>
              </div>
            )}
          </>
        ) : (
          <div className="metrics-empty" role="status">
            <span aria-hidden="true">&lt;</span>
            <p>Select a meeting to inspect its transcript, audio paths, and latest report.</p>
          </div>
        )}
      </article>
    </div>
  </section>
);
