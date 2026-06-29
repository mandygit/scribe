import type { AnalysisResult, MetricsCalculationResult } from '../tauri-commands';
import { formatScore, scoreTone } from './formatting';
import { REPORT_DIMENSIONS, type ReportDimensionKey } from './labels';

interface ScorecardReportProps {
  lastAnalysis: AnalysisResult | null;
  lastMetrics: MetricsCalculationResult | null;
}

const matchingObservations = (
  analysis: AnalysisResult,
  dimension: ReportDimensionKey,
): AnalysisResult['analysis']['observations'] => {
  const normalize = (value: string): string => value.toLowerCase().replace(/[^a-z]/g, '');
  const normalizedDimension = normalize(dimension);

  return analysis.analysis.observations.filter((observation) => {
    const category = normalize(observation.category);
    return category.includes(normalizedDimension) || normalizedDimension.includes(category);
  });
};

export const ScorecardReport = ({ lastAnalysis, lastMetrics }: ScorecardReportProps) => (
  <section className="scorecard-report" aria-labelledby="scorecard-title">
    <div className="scorecard-rail" aria-hidden="true" />
    <div className="scorecard-header">
      <div>
        <span className="status-label">Meeting report</span>
        <h2 id="scorecard-title">Summary and coaching</h2>
        <p>Summary first, details on demand. Built from deterministic metrics and the local coaching analysis.</p>
      </div>
      <span className="metrics-state">
        {lastAnalysis ? 'Review ready' : lastMetrics ? 'Report ready to generate' : 'Waiting for transcript'}
      </span>
    </div>

    {lastAnalysis ? (
      <>
        <div className="score-hero">
          <div>
            <span className="score-hero-label">Overall</span>
            <strong>{formatScore(lastAnalysis.scorecard.overall.score)}</strong>
            <p>{scoreTone(lastAnalysis.scorecard.overall.score)}</p>
          </div>
          {lastAnalysis.scorecard.overall.unavailableReason ? (
            <div className="score-warning" role="status">
              <strong>Report is partial</strong>
              <p>{lastAnalysis.scorecard.overall.unavailableReason}</p>
            </div>
          ) : (
            <p className="score-hero-note">
              Analyzer score: {formatScore(lastAnalysis.scorecard.analysis.score)} · Report {lastAnalysis.reportId}
            </p>
          )}
        </div>

        <section className="dimension-stack" aria-label="Score dimensions">
          {REPORT_DIMENSIONS.map((dimension) => {
            const scoreDimension = lastAnalysis.scorecard[dimension.key];
            const observations = matchingObservations(lastAnalysis, dimension.key);

            return (
              <details className="dimension-disclosure" key={dimension.key}>
                <summary>
                  <span>
                    <span className="dimension-label">{dimension.label}</span>
                    <span className="dimension-cue">{dimension.cue}</span>
                  </span>
                  <span className="dimension-score">{formatScore(scoreDimension.score)}</span>
                </summary>

                <div className="dimension-body">
                  {scoreDimension.unavailableReason ? (
                    <div className="score-warning compact" role="status">
                      <strong>Missing signal</strong>
                      <p>{scoreDimension.unavailableReason}</p>
                    </div>
                  ) : (
                    <p className="dimension-status">{scoreTone(scoreDimension.score)} for this dimension.</p>
                  )}

                  {observations.length > 0 ? (
                    <ul className="observation-list" aria-label={`${dimension.label} observations`}>
                      {observations.map((observation) => (
                        <li key={`${dimension.key}-${observation.quote}-${observation.suggestion}`}>
                          <span className="observation-role">
                            Your words{observation.speakerLabel ? ` · ${observation.speakerLabel}` : ''}
                          </span>
                          <blockquote>"{observation.quote}"</blockquote>
                          {observation.contextQuote ? (
                            <div className="observation-context">
                              <span>
                                Context
                                {observation.contextSpeakerLabel ? ` · ${observation.contextSpeakerLabel}` : ''}
                              </span>
                              <p>"{observation.contextQuote}"</p>
                            </div>
                          ) : null}
                          <p>
                            <strong>Suggestion:</strong> {observation.suggestion}
                          </p>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="dimension-empty">
                      No exact quote was attached to this dimension. Use the score as a directional signal.
                    </p>
                  )}
                </div>
              </details>
            );
          })}
        </section>
      </>
    ) : (
      <div className="metrics-empty" role="status">
        <span aria-hidden="true">^</span>
        <p>Stop a session to generate a summary, score card, and coaching notes automatically.</p>
      </div>
    )}
  </section>
);
