import type { AppStatus, ResonanceSettings } from '../tauri-commands';

interface PrivacySettingsPanelProps {
  settings: AppStatus['defaultSettings'];
  retentionDays: string;
  analyzerProvider: ResonanceSettings['analyzerProvider'];
  cloudAnalysisEnabled: boolean;
  isBusy: boolean;
  onRetentionDaysChange: (value: string) => void;
  onAnalyzerProviderChange: (value: ResonanceSettings['analyzerProvider']) => void;
  onCloudAnalysisEnabledChange: (value: boolean) => void;
  onSave: () => void;
}

const analyzerProviderOptions: Array<{
  value: ResonanceSettings['analyzerProvider'];
  label: string;
  description: string;
}> = [
  {
    value: 'localOllama',
    label: 'Local Ollama',
    description: 'Transcript text stays on this Mac.',
  },
  {
    value: 'cloudOpenAi',
    label: 'Cloud OpenAI',
    description: 'Requires explicit opt-in before transcript text can leave this Mac.',
  },
  {
    value: 'cloudClaude',
    label: 'Cloud Claude',
    description: 'Requires explicit opt-in before transcript text can leave this Mac.',
  },
];

export const PrivacySettingsPanel = ({
  settings,
  retentionDays,
  analyzerProvider,
  cloudAnalysisEnabled,
  isBusy,
  onRetentionDaysChange,
  onAnalyzerProviderChange,
  onCloudAnalysisEnabledChange,
  onSave,
}: PrivacySettingsPanelProps) => (
  <section className="privacy-panel" aria-labelledby="privacy-title">
    <div className="privacy-header">
      <div>
        <span className="status-label">Privacy</span>
        <h2 id="privacy-title">Local data controls</h2>
        <p>
          Raw microphone and system audio stay local and follow the retention window below. Transcripts and reports
          remain available after audio cleanup.
        </p>
      </div>
      <strong>{settings.cloudAnalysisEnabled ? 'Cloud opt-in active' : 'Local-first default'}</strong>
    </div>

    <div className="privacy-grid">
      <label>
        Raw audio retention
        <input
          type="number"
          min="0"
          max="365"
          inputMode="numeric"
          value={retentionDays}
          disabled={isBusy}
          onChange={(event) => onRetentionDaysChange(event.currentTarget.value)}
        />
        <span>0 deletes retained raw audio on the next cleanup; max 365 days.</span>
      </label>

      <fieldset>
        <legend>Analyzer provider</legend>
        {analyzerProviderOptions.map((option) => (
          <label key={option.value}>
            <input
              type="radio"
              name="analyzer-provider"
              value={option.value}
              checked={analyzerProvider === option.value}
              disabled={isBusy || (option.value !== 'localOllama' && !cloudAnalysisEnabled)}
              onChange={() => onAnalyzerProviderChange(option.value)}
            />
            <span>
              <strong>{option.label}</strong>
              {option.description}
            </span>
          </label>
        ))}
      </fieldset>

      <div className="privacy-opt-in">
        <label>
          <input
            type="checkbox"
            checked={cloudAnalysisEnabled}
            disabled={isBusy}
            onChange={(event) => onCloudAnalysisEnabledChange(event.currentTarget.checked)}
          />
          Explicitly allow cloud analysis
        </label>
        <p>
          Cloud adapters are not connected yet; this opt-in is stored so future cloud analysis cannot be accidental.
        </p>
      </div>
    </div>

    <button type="button" onClick={onSave} disabled={isBusy}>
      Save privacy settings
    </button>
  </section>
);
