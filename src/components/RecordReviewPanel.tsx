import type { CameraDevice, PracticeRecording, PracticeReviewResult } from '../tauri-commands';
import { PracticeReviewReport } from './PracticeReviewReport';

interface RecordReviewPanelProps {
  title: string;
  importPath: string;
  ffmpegBinPath: string;
  cloudVideoReviewEnabled: boolean;
  allowCloudVideoForThisReview: boolean;
  cameraDevices: CameraDevice[];
  currentRecording: PracticeRecording | null;
  history: PracticeRecording[];
  result: PracticeReviewResult | null;
  message: string;
  isBusy: boolean;
  isCameraRecording: boolean;
  onTitleChange: (value: string) => void;
  onImportPathChange: (value: string) => void;
  onFfmpegBinPathChange: (value: string) => void;
  onCloudVideoReviewEnabledChange: (value: boolean) => void;
  onAllowCloudVideoForThisReviewChange: (value: boolean) => void;
  onLoadCameras: () => void;
  onStartCameraRecording: () => void;
  onStopCameraRecording: () => void;
  onImportVideo: () => void;
  onAnalyzeAudio: () => void;
  onAnalyzeCombined: () => void;
  onRefreshHistory: () => void;
  videoPreviewRef: (element: HTMLVideoElement | null) => void;
}

export const RecordReviewPanel = ({
  title,
  importPath,
  ffmpegBinPath,
  cloudVideoReviewEnabled,
  allowCloudVideoForThisReview,
  cameraDevices,
  currentRecording,
  history,
  result,
  message,
  isBusy,
  isCameraRecording,
  onTitleChange,
  onImportPathChange,
  onFfmpegBinPathChange,
  onCloudVideoReviewEnabledChange,
  onAllowCloudVideoForThisReviewChange,
  onLoadCameras,
  onStartCameraRecording,
  onStopCameraRecording,
  onImportVideo,
  onAnalyzeAudio,
  onAnalyzeCombined,
  onRefreshHistory,
  videoPreviewRef,
}: RecordReviewPanelProps) => (
  <section className="record-review-panel" aria-labelledby="record-review-title">
    <div>
      <span className="status-label">Record and Review</span>
      <h2 id="record-review-title">Practice on camera</h2>
      <p aria-live="polite">{message}</p>
      <p>
        Record a 1-15 minute self-practice video or import a local .mp4, .mov, or .webm. Audio review stays local;
        visual review requires explicit cloud consent and sends sampled frames to OpenAI.
      </p>
    </div>

    <div className="import-recording-grid">
      <label>
        Practice title
        <input
          type="text"
          value={title}
          onChange={(event) => onTitleChange(event.currentTarget.value)}
          placeholder="Investor pitch rehearsal"
          disabled={isBusy || isCameraRecording}
        />
      </label>
      <label>
        Practice video path
        <input
          type="text"
          value={importPath}
          onChange={(event) => onImportPathChange(event.currentTarget.value)}
          placeholder="/Users/you/Movies/practice.mp4"
          disabled={isBusy || isCameraRecording}
        />
      </label>
      <label>
        ffmpeg path
        <input
          type="text"
          value={ffmpegBinPath}
          onChange={(event) => onFfmpegBinPathChange(event.currentTarget.value)}
          placeholder="/opt/homebrew/bin/ffmpeg"
          disabled={isBusy || isCameraRecording}
        />
      </label>
    </div>

    <section className="voice-profile-panel" aria-label="Camera setup">
      <div>
        <h3>Camera setup</h3>
        <p>
          macOS may ask for Camera permission. Resonance records through the app window and saves the resulting video
          under app data for retention cleanup.
        </p>
        <video
          ref={videoPreviewRef}
          className="practice-camera-preview"
          autoPlay
          muted
          playsInline
          aria-label="Camera preview"
        />
        {cameraDevices.length > 0 ? (
          <ul>
            {cameraDevices.map((device) => (
              <li key={device.id}>{device.name}</li>
            ))}
          </ul>
        ) : null}
      </div>
      <div className="voice-profile-actions">
        <button type="button" onClick={onLoadCameras} disabled={isBusy || isCameraRecording}>
          Check camera
        </button>
        <button type="button" onClick={onStartCameraRecording} disabled={isBusy || isCameraRecording}>
          Start camera practice
        </button>
        <button type="button" onClick={onStopCameraRecording} disabled={!isCameraRecording}>
          Stop and save practice
        </button>
        <button type="button" onClick={onImportVideo} disabled={isBusy || importPath.trim().length === 0}>
          Import practice video
        </button>
      </div>
    </section>

    {currentRecording ? (
      <section className="recording-path" aria-label="Selected practice recording">
        <strong>{currentRecording.title ?? currentRecording.id}</strong>
        <p>Video: {currentRecording.videoFilePath}</p>
        <p>Status: {currentRecording.analysisStatus}</p>
      </section>
    ) : null}

    <section className="privacy-opt-in" aria-label="Practice review controls">
      <button type="button" onClick={onAnalyzeAudio} disabled={isBusy || currentRecording === null}>
        Run local audio review
      </button>
      <label>
        <input
          type="checkbox"
          checked={cloudVideoReviewEnabled}
          onChange={(event) => onCloudVideoReviewEnabledChange(event.currentTarget.checked)}
          disabled={isBusy}
        />
        Enable cloud video review setting
      </label>
      <label>
        <input
          type="checkbox"
          checked={allowCloudVideoForThisReview}
          onChange={(event) => onAllowCloudVideoForThisReviewChange(event.currentTarget.checked)}
          disabled={isBusy || !cloudVideoReviewEnabled}
        />
        I confirm this review may send sampled practice-video frames to OpenAI
      </label>
      <p>
        Visual review sends sampled frames from the selected practice video to OpenAI. Use this only if you are
        comfortable sharing those images with that provider.
      </p>
      <button type="button" onClick={onAnalyzeCombined} disabled={isBusy || currentRecording === null}>
        Run combined review
      </button>
    </section>

    <PracticeReviewReport result={result} />

    <section aria-label="Practice history">
      <div className="privacy-header">
        <h3>Practice history</h3>
        <button type="button" onClick={onRefreshHistory} disabled={isBusy}>
          Refresh practice history
        </button>
      </div>
      {history.length > 0 ? (
        <ul>
          {history.map((recording) => (
            <li key={recording.id}>
              <strong>{recording.title ?? recording.id}</strong> · {recording.sourceKind} · {recording.analysisStatus}
              {recording.cloudVideoUsed ? ' · cloud video used' : ' · local only'}
            </li>
          ))}
        </ul>
      ) : (
        <p>No practice recordings yet.</p>
      )}
    </section>
  </section>
);
