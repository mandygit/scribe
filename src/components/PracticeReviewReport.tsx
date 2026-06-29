import type { PracticeReviewResult } from '../tauri-commands';

interface PracticeReviewReportProps {
  result: PracticeReviewResult | null;
}

const formatTimeRange = (startedAtMs: number, endedAtMs: number): string => {
  const formatSeconds = (value: number): string => `${Math.round(value / 1000)}s`;
  return `${formatSeconds(startedAtMs)}-${formatSeconds(endedAtMs)}`;
};

export const PracticeReviewReport = ({ result }: PracticeReviewReportProps) => {
  if (!result) {
    return null;
  }

  const { report, annotations, recording } = result;
  const privacyLabel = recording.cloudVideoUsed ? 'Cloud video used' : 'Local-only audio review';

  return (
    <article className="practice-review-report" aria-label="Practice review report">
      <div>
        <span className="status-label">{privacyLabel}</span>
        <h3>Practice review report</h3>
        <p>{report.body.summary}</p>
      </div>

      <div className="summary-columns">
        <section>
          <h4>Scores</h4>
          <p>Overall: {report.overallScore ?? 'Missing signal'}/100</p>
          <p>Audio delivery: {report.audioScore ?? 'Not run'}/100</p>
          <p>Visual delivery: {report.visualScore ?? 'Not run'}/100</p>
        </section>
        <section>
          <h4>Audio delivery</h4>
          <p>{report.body.audioSummary}</p>
        </section>
        <section>
          <h4>Visual delivery</h4>
          <p>{report.body.visualSummary}</p>
        </section>
      </div>

      <section>
        <h4>Suggestions</h4>
        <ul>
          {report.body.suggestions.map((suggestion) => (
            <li key={suggestion}>{suggestion}</li>
          ))}
        </ul>
      </section>

      <section>
        <h4>Timeline annotations</h4>
        {annotations.length > 0 ? (
          <ul>
            {annotations.map((annotation) => (
              <li key={annotation.id}>
                <strong>
                  {formatTimeRange(annotation.startedAtMs, annotation.endedAtMs)} · {annotation.category} ·{' '}
                  {annotation.severity}
                </strong>
                <p>{annotation.evidence}</p>
                <p>{annotation.suggestion}</p>
                <span>{annotation.source}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p>No timeline annotations were generated for this pass.</p>
        )}
      </section>

      <p>{report.body.privacyNote}</p>
    </article>
  );
};
