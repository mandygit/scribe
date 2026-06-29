import type { MeetingTrendPoint } from '../tauri-commands';
import { formatDateTime, formatNumber, formatScore, scoreTone } from './formatting';

interface TrendsDashboardProps {
  points: MeetingTrendPoint[];
  selectedLimit: number;
  availableLimits: number[];
  message: string;
  isLoading: boolean;
  onLimitChange: (limit: number) => void;
  onRefresh: () => void;
}

interface TrendMetric {
  key: 'fillerWordCount' | 'wordsPerMinute' | 'overallScore';
  label: string;
  unit: string;
  maxValue: number;
  formatValue: (value: number | null) => string;
}

const trendMetrics = (points: MeetingTrendPoint[]): TrendMetric[] => {
  const maxFillers = Math.max(1, ...points.map((point) => point.fillerWordCount ?? 0));
  const maxPace = Math.max(1, ...points.map((point) => point.wordsPerMinute ?? 0));

  return [
    {
      key: 'overallScore',
      label: 'Overall score',
      unit: 'score',
      maxValue: 100,
      formatValue: formatScore,
    },
    {
      key: 'fillerWordCount',
      label: 'Filler count',
      unit: 'fillers',
      maxValue: maxFillers,
      formatValue: (value) => (value === null ? 'Missing' : formatNumber(value)),
    },
    {
      key: 'wordsPerMinute',
      label: 'Pace',
      unit: 'wpm',
      maxValue: maxPace,
      formatValue: (value) => (value === null ? 'Missing' : `${formatNumber(value)} wpm`),
    },
  ];
};

const metricValue = (point: MeetingTrendPoint, metric: TrendMetric): number | null => point[metric.key];

const barWidth = (value: number | null, maxValue: number): string => {
  if (value === null || maxValue <= 0) {
    return '0%';
  }

  return `${Math.min(100, Math.max(4, (value / maxValue) * 100))}%`;
};

const latestPointLabel = (point: MeetingTrendPoint): string => point.title ?? point.meetingId;

export const TrendsDashboard = ({
  points,
  selectedLimit,
  availableLimits,
  message,
  isLoading,
  onLimitChange,
  onRefresh,
}: TrendsDashboardProps) => {
  const metrics = trendMetrics(points);
  const latestPoint = points.at(-1) ?? null;
  const hasSparseData = points.some(
    (point) => point.fillerWordCount === null || point.wordsPerMinute === null || point.overallScore === null,
  );

  return (
    <section className="trends-panel" aria-labelledby="trends-title" aria-busy={isLoading}>
      <div className="trends-header">
        <div>
          <span className="status-label">Trends</span>
          <h2 id="trends-title">Recent meeting trajectory</h2>
          <p>{message}</p>
        </div>
        <div className="trends-actions">
          <label>
            Recent range
            <select
              value={selectedLimit}
              disabled={isLoading}
              onChange={(event) => onLimitChange(Number(event.currentTarget.value))}
            >
              {availableLimits.map((limit) => (
                <option key={limit} value={limit}>
                  Last {limit}
                </option>
              ))}
            </select>
          </label>
          <button type="button" onClick={onRefresh} disabled={isLoading}>
            Refresh trends
          </button>
        </div>
      </div>

      {points.length === 0 ? (
        <div className="metrics-empty compact" role="status">
          <span>+</span>
          <p>Run metrics and analysis on a few meetings to build a local trend line.</p>
        </div>
      ) : (
        <>
          <div className="trends-summary">
            <div>
              <span>Latest meeting</span>
              <strong>{latestPoint ? latestPointLabel(latestPoint) : 'Unavailable'}</strong>
            </div>
            <div>
              <span>Overall</span>
              <strong>{formatScore(latestPoint?.overallScore ?? null)}</strong>
              <p>{scoreTone(latestPoint?.overallScore ?? null)}</p>
            </div>
            <div>
              <span>Coverage</span>
              <strong>{points.length} meeting(s)</strong>
              <p>
                {hasSparseData
                  ? 'Some datapoints are missing metrics or reports.'
                  : 'All trend datapoints are complete.'}
              </p>
            </div>
          </div>

          <ul className="trend-chart-list" aria-label="Meeting trend datapoints">
            {points.map((point) => (
              <li className="trend-row" key={point.meetingId}>
                <div className="trend-row-label">
                  <strong>{latestPointLabel(point)}</strong>
                  <span>{formatDateTime(point.startedAtMs)}</span>
                </div>
                <div className="trend-bars">
                  {metrics.map((metric) => {
                    const value = metricValue(point, metric);
                    return (
                      <div className="trend-bar-group" key={metric.key}>
                        <div className="trend-bar-label">
                          <span>{metric.label}</span>
                          <strong>{metric.formatValue(value)}</strong>
                        </div>
                        <div
                          aria-label={`${metric.label}: ${metric.formatValue(value)}`}
                          aria-valuemax={metric.maxValue}
                          aria-valuemin={0}
                          aria-valuenow={value ?? undefined}
                          className="trend-bar-track"
                          role="progressbar"
                        >
                          <span data-missing={value === null} style={{ width: barWidth(value, metric.maxValue) }} />
                        </div>
                      </div>
                    );
                  })}
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
};
