import type { MetricsCalculationResult, TranscriptionResult } from '../tauri-commands';
import { formatDuration, formatNumber, formatPercent } from './formatting';

interface MetricSummaryCard {
  label: string;
  value: string;
  detail: string;
}

interface MetricsPanelProps {
  lastMetrics: MetricsCalculationResult | null;
  lastTranscription: TranscriptionResult | null;
}

const buildMetricSummaryCards = (metrics: MetricsCalculationResult): MetricSummaryCard[] => [
  {
    label: 'Pace',
    value: `${formatNumber(metrics.summary.wordsPerMinute, 1)} wpm`,
    detail: `${formatNumber(metrics.summary.wordCount)} words across ${formatDuration(metrics.summary.durationMs)}`,
  },
  {
    label: 'Fillers',
    value: formatNumber(metrics.summary.fillerWordCount),
    detail: `${formatPercent(metrics.summary.fillerWordRate)} of spoken words`,
  },
  {
    label: 'Hedging',
    value: formatNumber(metrics.summary.hedgingPhraseCount),
    detail: 'Matched cautious phrases without substring false positives',
  },
  {
    label: 'Talk time',
    value: formatDuration(metrics.summary.userTalkTimeMs),
    detail: `Longest run ${formatDuration(metrics.summary.longestMonologueMs)}`,
  },
];

export const MetricsPanel = ({ lastMetrics, lastTranscription }: MetricsPanelProps) => {
  const metricSummaryCards = lastMetrics ? buildMetricSummaryCards(lastMetrics) : [];

  return (
    <section className="metrics-panel" aria-labelledby="metrics-title">
      <div className="metrics-panel-header">
        <div>
          <span className="status-label">Speaking signals</span>
          <h2 id="metrics-title">Communication metrics</h2>
        </div>
        <span className="metrics-state">
          {lastMetrics ? 'Stored locally' : lastTranscription ? 'Ready to calculate' : 'Waiting for transcript'}
        </span>
      </div>

      {lastMetrics ? (
        <section className="metrics-grid" aria-label="Calculated communication metrics">
          {metricSummaryCards.map((metric) => (
            <article className="metric-card" key={metric.label}>
              <span>{metric.label}</span>
              <strong>{metric.value}</strong>
              <p>{metric.detail}</p>
            </article>
          ))}
        </section>
      ) : (
        <div className="metrics-empty" role="status">
          <span aria-hidden="true">-</span>
          <p>
            Finish a session to calculate pace, fillers, hedging, and talk-time signals automatically, or retry the step
            manually here.
          </p>
        </div>
      )}
    </section>
  );
};
