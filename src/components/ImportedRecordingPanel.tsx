import type {
  ImportedRecordingSummaryResult,
  RecordingMetadata,
  VoiceDiarizationMatchResult,
  VoiceDiarizationResult,
  VoiceDiarizationStatus,
  VoiceMatcherStatus,
  VoiceMatchResult,
  VoiceProfileStatus,
} from '../tauri-commands';

interface ImportedRecordingPanelProps {
  sourcePath: string;
  ffmpegBinPath: string;
  isTranscriberModelConfigured: boolean;
  includeSpeakingImprovements: boolean;
  useMatchedVoiceCoaching: boolean;
  cloudVideoReviewEnabled: boolean;
  allowCloudVideoForThisReview: boolean;
  voiceProfileStatus: VoiceProfileStatus;
  voiceMatcherStatus: VoiceMatcherStatus;
  voiceDiarizationStatus: VoiceDiarizationStatus;
  voiceDiarizationResult: VoiceDiarizationResult | null;
  voiceDiarizationMatchResult: VoiceDiarizationMatchResult | null;
  voiceMatchResult: VoiceMatchResult | null;
  importedVoiceMatchResult: VoiceMatchResult | null;
  lastRecording: RecordingMetadata | null;
  result: ImportedRecordingSummaryResult | null;
  message: string;
  isBusy: boolean;
  onSourcePathChange: (value: string) => void;
  onFfmpegBinPathChange: (value: string) => void;
  onIncludeSpeakingImprovementsChange: (value: boolean) => void;
  onUseMatchedVoiceCoachingChange: (value: boolean) => void;
  onCloudVideoReviewEnabledChange: (value: boolean) => void;
  onAllowCloudVideoForThisReviewChange: (value: boolean) => void;
  onEnrollVoiceProfile: () => void;
  onPrepareVoiceProfileForMatching: () => void;
  onTestVoiceProfileMatch: () => void;
  onMatchImportedRecordingVoice: () => void;
  onDiarizeImportedRecordingSpeakers: () => void;
  onMatchImportedRecordingSpeakerSegments: () => void;
  onDeleteVoiceProfile: () => void;
  onImport: () => void;
}

export const ImportedRecordingPanel = ({
  sourcePath,
  ffmpegBinPath,
  isTranscriberModelConfigured,
  includeSpeakingImprovements,
  useMatchedVoiceCoaching,
  cloudVideoReviewEnabled,
  allowCloudVideoForThisReview,
  voiceProfileStatus,
  voiceMatcherStatus,
  voiceDiarizationStatus,
  voiceDiarizationResult,
  voiceDiarizationMatchResult,
  voiceMatchResult,
  importedVoiceMatchResult,
  lastRecording,
  result,
  message,
  isBusy,
  onSourcePathChange,
  onFfmpegBinPathChange,
  onIncludeSpeakingImprovementsChange,
  onUseMatchedVoiceCoachingChange,
  onCloudVideoReviewEnabledChange,
  onAllowCloudVideoForThisReviewChange,
  onEnrollVoiceProfile,
  onPrepareVoiceProfileForMatching,
  onTestVoiceProfileMatch,
  onMatchImportedRecordingVoice,
  onDiarizeImportedRecordingSpeakers,
  onMatchImportedRecordingSpeakerSegments,
  onDeleteVoiceProfile,
  onImport,
}: ImportedRecordingPanelProps) => (
  <section className="import-recording-panel" aria-labelledby="import-recording-title">
    <div>
      <span className="status-label">Downloaded recording</span>
      <h2 id="import-recording-title">Summarize a missed meeting</h2>
      <p aria-live="polite">{message}</p>
      <p>
        Paste a local .mp4, .mov, .m4a, .mp3, or .wav path. Resonance extracts audio locally with ffmpeg, transcribes
        with your configured whisper.cpp model, then summarizes through local Ollama.
      </p>
    </div>

    <div className="import-recording-grid">
      <label>
        Recording file path
        <input
          type="text"
          value={sourcePath}
          onChange={(event) => onSourcePathChange(event.currentTarget.value)}
          placeholder="/Users/you/Downloads/team-meeting.mp4"
          disabled={isBusy}
        />
      </label>
      <label>
        ffmpeg path
        <input
          type="text"
          value={ffmpegBinPath}
          onChange={(event) => onFfmpegBinPathChange(event.currentTarget.value)}
          placeholder="/opt/homebrew/bin/ffmpeg"
          disabled={isBusy}
        />
      </label>
      {!isTranscriberModelConfigured ? (
        <p className="import-recording-warning">
          Save a whisper.cpp model path in <strong>Whisper transcription settings</strong> before extracting and
          summarizing imported recordings.
        </p>
      ) : null}
      <label className="import-recording-checkbox">
        <input
          type="checkbox"
          checked={includeSpeakingImprovements}
          onChange={(event) => onIncludeSpeakingImprovementsChange(event.currentTarget.checked)}
          disabled={isBusy}
        />
        <span>
          <strong>I am the main speaker/presenter in this recording</strong>
          <span>Fallback: only use this when you are clearly the primary speaker.</span>
        </span>
      </label>
      <label className="import-recording-checkbox">
        <input
          type="checkbox"
          checked={useMatchedVoiceCoaching}
          onChange={(event) => onUseMatchedVoiceCoachingChange(event.currentTarget.checked)}
          disabled={
            isBusy ||
            !voiceProfileStatus.matchingReady ||
            !voiceMatcherStatus.extractorReady ||
            !voiceDiarizationStatus.diarizationReady
          }
        />
        <span>
          <strong>Use my matched voice profile for speaking coaching</strong>
          <span>Recommended: coach only the speech that matches your local voice profile.</span>
        </span>
      </label>
      <label className="import-recording-checkbox">
        <input
          type="checkbox"
          checked={cloudVideoReviewEnabled}
          onChange={(event) => onCloudVideoReviewEnabledChange(event.currentTarget.checked)}
          disabled={isBusy}
        />
        <span>
          <strong>Enable cloud video review setting</strong>
          <span>Required before sampled meeting frames can be sent to OpenAI.</span>
        </span>
      </label>
      <label className="import-recording-checkbox">
        <input
          type="checkbox"
          checked={allowCloudVideoForThisReview}
          onChange={(event) => onAllowCloudVideoForThisReviewChange(event.currentTarget.checked)}
          disabled={isBusy || !cloudVideoReviewEnabled}
        />
        <span>
          <strong>I confirm this meeting review may send sampled frames to OpenAI</strong>
          <span>
            Visual feedback is attempted only for video files with matched user-speech windows. If your camera is off or
            you are not visible, Resonance keeps the result audio-only.
          </span>
        </span>
      </label>
      <section className="voice-profile-panel" aria-label="Local voice profile">
        <div>
          <strong>
            {voiceProfileStatus.matchingReady
              ? 'Voice profile prepared for matching'
              : voiceProfileStatus.isEnrolled
                ? 'Voice profile enrolled locally'
                : 'No local voice profile yet'}
          </strong>
          <p>
            {voiceProfileStatus.matchingReady
              ? 'Your local voice embedding is ready for the next imported-recording matching slice.'
              : voiceProfileStatus.isEnrolled
                ? 'Enrollment sample saved locally. Matching also needs a configured speaker embedding model.'
                : 'Run a short mic test, then enroll that local sample so future matching can identify your speech in imported recordings.'}
          </p>
          <p>
            Matcher: {voiceMatcherStatus.message}
            {voiceMatcherStatus.embeddingDimension ? ` Dimension: ${voiceMatcherStatus.embeddingDimension}.` : ''}
          </p>
          <p>Diarization: {voiceDiarizationStatus.message}</p>
        </div>
        <div className="voice-profile-actions">
          <button type="button" onClick={onEnrollVoiceProfile} disabled={isBusy || lastRecording === null}>
            Enroll from last mic test
          </button>
          <button
            type="button"
            onClick={onPrepareVoiceProfileForMatching}
            disabled={isBusy || !voiceProfileStatus.isEnrolled || !voiceMatcherStatus.extractorReady}
          >
            Prepare matching
          </button>
          <button
            type="button"
            onClick={onTestVoiceProfileMatch}
            disabled={isBusy || !voiceProfileStatus.matchingReady || lastRecording === null}
          >
            Test match with last mic test
          </button>
          <button type="button" onClick={onDeleteVoiceProfile} disabled={isBusy || !voiceProfileStatus.isEnrolled}>
            Clear voice profile
          </button>
          <button
            type="button"
            onClick={onMatchImportedRecordingVoice}
            disabled={isBusy || !voiceProfileStatus.matchingReady || sourcePath.trim().length === 0}
          >
            Check voice in recording
          </button>
          <button
            type="button"
            onClick={onDiarizeImportedRecordingSpeakers}
            disabled={
              isBusy ||
              !voiceDiarizationStatus.diarizationReady ||
              !voiceMatcherStatus.extractorReady ||
              sourcePath.trim().length === 0
            }
          >
            Preview speaker segments
          </button>
          <button
            type="button"
            onClick={onMatchImportedRecordingSpeakerSegments}
            disabled={
              isBusy ||
              !voiceProfileStatus.matchingReady ||
              !voiceDiarizationStatus.diarizationReady ||
              !voiceMatcherStatus.extractorReady ||
              sourcePath.trim().length === 0
            }
          >
            Match my speaker segments
          </button>
        </div>
        {voiceMatchResult ? (
          <p>
            {Math.round(voiceMatchResult.similarityScore * 100)}% match · Threshold{' '}
            {Math.round(voiceMatchResult.threshold * 100)}%. {voiceMatchResult.message}
          </p>
        ) : null}
        {importedVoiceMatchResult ? (
          <p>
            {Math.round(importedVoiceMatchResult.similarityScore * 100)}% imported-recording match · Threshold{' '}
            {Math.round(importedVoiceMatchResult.threshold * 100)}%. {importedVoiceMatchResult.message}
          </p>
        ) : null}
        {voiceDiarizationResult ? (
          <p>
            {voiceDiarizationResult.segmentCount} diarized speaker segment(s) across{' '}
            {voiceDiarizationResult.speakerCount} speaker(s).
          </p>
        ) : null}
        {voiceDiarizationMatchResult ? (
          <p>
            {voiceDiarizationMatchResult.matchedWindowCount} likely user speaker segment(s) matched across{' '}
            {voiceDiarizationMatchResult.speakerMatches.length} diarized speaker(s).
          </p>
        ) : null}
      </section>
      <button
        type="button"
        className="import-recording-submit"
        onClick={onImport}
        disabled={isBusy || sourcePath.trim().length === 0 || !isTranscriberModelConfigured}
      >
        Extract, transcribe, and summarize
      </button>
    </div>

    {result ? (
      <article className="import-recording-result">
        <div>
          <span className="status-label">{result.segmentCount} transcript segments</span>
          <h3>Executive summary</h3>
          <p>{result.summary.executiveSummary}</p>
        </div>

        <div className="summary-columns">
          <section>
            <h4>Speaking improvements</h4>
            {!result.speakingImprovementsRequested ? (
              <p>
                Speaking coaching was skipped because no user speech source was selected or matched. Mark yourself as
                the main speaker, or use local voice matching after preparing your voice profile.
              </p>
            ) : result.speakingImprovementsSource === 'voiceMatch' &&
              result.summary.speakingImprovements.length === 0 ? (
              <p>No speaking improvement notes found for the matched user-speech transcript.</p>
            ) : result.summary.speakingImprovements.length > 0 ? (
              <ul>
                {result.summary.speakingImprovements.map((improvement) => (
                  <li key={`${improvement.category}-${improvement.quote}`}>
                    <strong>{improvement.category}:</strong> {improvement.suggestion}
                    <q>{improvement.quote}</q>
                  </li>
                ))}
              </ul>
            ) : (
              <p>No speaking improvement notes found for the main-speaker transcript.</p>
            )}
          </section>

          <section>
            <h4>Visual delivery</h4>
            {result.visualReview === null || result.visualReview.status === 'notRequested' ? (
              <p>Visual review was not requested for this imported meeting recording.</p>
            ) : (
              <>
                <p>{result.visualReview.summary}</p>
                <p>
                  Status: {result.visualReview.status}
                  {result.visualReview.visualScore === null ? '' : ` · Score ${result.visualReview.visualScore}/100`}
                </p>
                {result.visualReview.annotations.length > 0 ? (
                  <ul>
                    {result.visualReview.annotations.map((annotation) => (
                      <li key={`${annotation.startedAtMs}-${annotation.category}-${annotation.evidence}`}>
                        <strong>
                          {annotation.category} · {annotation.severity}
                        </strong>
                        <p>{annotation.evidence}</p>
                        <p>{annotation.suggestion}</p>
                      </li>
                    ))}
                  </ul>
                ) : null}
                <p>{result.visualReview.privacyNote}</p>
              </>
            )}
          </section>

          <section>
            <h4>Action items</h4>
            {result.summary.actionItems.length > 0 ? (
              <ul>
                {result.summary.actionItems.map((item) => (
                  <li key={`${item.owner ?? 'unknown'}-${item.task}`}>
                    <strong>{item.owner ?? 'Unassigned'}:</strong> {item.task}
                    {item.due ? ` · Due ${item.due}` : ''}
                  </li>
                ))}
              </ul>
            ) : (
              <p>No action items found.</p>
            )}
          </section>

          <section>
            <h4>Decisions</h4>
            {result.summary.decisions.length > 0 ? (
              <ul>
                {result.summary.decisions.map((decision) => (
                  <li key={decision}>{decision}</li>
                ))}
              </ul>
            ) : (
              <p>No decisions found.</p>
            )}
          </section>

          <section>
            <h4>Open questions</h4>
            {result.summary.openQuestions.length > 0 ? (
              <ul>
                {result.summary.openQuestions.map((question) => (
                  <li key={question}>{question}</li>
                ))}
              </ul>
            ) : (
              <p>No open questions found.</p>
            )}
          </section>
        </div>
      </article>
    ) : null}
  </section>
);
