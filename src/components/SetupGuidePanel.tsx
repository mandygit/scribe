import type { AppStatus } from '../tauri-commands';

interface SetupGuidePanelProps {
  settings: AppStatus['defaultSettings'];
  transcriberBinPath: string;
  transcriberModelPath: string;
}

export const SetupGuidePanel = ({ settings, transcriberBinPath, transcriberModelPath }: SetupGuidePanelProps) => {
  const hasWhisperBinary = transcriberBinPath.trim() !== '' || settings.transcriberBinPath !== null;
  const hasWhisperModel = transcriberModelPath.trim() !== '' || settings.transcriberModelPath !== null;

  return (
    <section className="setup-guide-panel" aria-labelledby="setup-guide-title">
      <div className="setup-guide-header">
        <div>
          <span className="status-label">First run</span>
          <h2 id="setup-guide-title">Ready Resonance for local meetings</h2>
          <p>
            Complete these local setup checks before packaging or running a clean install. Resonance degrades safely
            when optional pieces are unavailable.
          </p>
        </div>
        <strong>Local package: bun run package:mac</strong>
      </div>

      <ol className="setup-guide-list">
        <li>
          <strong>Microphone permission</strong>
          <p>
            Start a mic test to trigger macOS Microphone access. If it is denied, open System Settings &gt; Privacy
            &amp; Security &gt; Microphone and enable Resonance.
          </p>
        </li>
        <li>
          <strong>System audio permission</strong>
          <p>
            System audio uses ScreenCaptureKit and may require Screen & System Audio Recording permission. If capture is
            blocked, enable Resonance in System Settings and restart the app, or turn off system audio to continue
            mic-only.
          </p>
        </li>
        <li>
          <strong>Camera permission</strong>
          <p>
            Record and Review uses Camera permission for self-practice videos. If the preview is blocked, open System
            Settings &gt; Privacy &amp; Security &gt; Camera and enable Resonance.
          </p>
        </li>
        <li>
          <strong>Whisper transcription paths</strong>
          <p>
            {hasWhisperBinary && hasWhisperModel
              ? 'Transcription paths are configured for this install.'
              : 'Install whisper.cpp, then save the whisper-cli binary path and an absolute model path before transcribing.'}
          </p>
        </li>
        <li>
          <strong>Local Ollama analysis</strong>
          <p>
            Missing reports usually mean Ollama is not running. Install with brew install ollama, start it with ollama
            serve, then pull the default model with ollama pull llama3.2. Transcription, metrics, history, and audio
            retention still work while local analysis is unavailable.
          </p>
        </li>
      </ol>
    </section>
  );
};
