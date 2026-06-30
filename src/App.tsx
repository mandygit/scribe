import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import './styles.css';
import { DICTATION_HOTKEYS } from './contracts';
import { messageFromUnknownError } from './error-utils';
import { formatClock, formatDate, formatDuration, meetingTitle } from './format';
import {
  type AppStatus,
  type AudioDevice,
  getAppStatus,
  getMeetingHistoryDetail,
  isTauriRuntime,
  listAudioDevices,
  listMeetingHistory,
  type MeetingHistoryDetail,
  type MeetingHistoryItem,
  type MeetingSummary,
  type ResonanceSettings,
  startRecording,
  stopRecording,
  summarizeMeeting,
  transcribeMeeting,
  updateAudioProcessingSettings,
  updateDictationSettings,
  updatePrivacySettings,
  updateTranscriberSettings,
} from './tauri-commands';

const FALLBACK_SETTINGS: ResonanceSettings = {
  microphoneDeviceId: null,
  enableSystemAudio: true,
  enableEchoCancellation: true,
  enableRealtimeNudges: true,
  rawAudioRetentionDays: 7,
  analyzerProvider: 'localOllama',
  cloudAnalysisEnabled: false,
  cloudVideoReviewEnabled: false,
  transcriberBinPath: null,
  transcriberModelPath: null,
  speakerEmbeddingModelPath: null,
  speakerSegmentationModelPath: null,
  dictationHotkey: 'cmd+shift+d',
  dictationPolishEnabled: false,
};

type View = 'meetings' | 'settings';
type RecordPhase = 'idle' | 'recording' | 'stopping' | 'transcribing';

export default function App() {
  const tauri = useMemo(() => isTauriRuntime(), []);
  const [view, setView] = useState<View>('meetings');
  const [settings, setSettings] = useState<ResonanceSettings>(FALLBACK_SETTINGS);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [history, setHistory] = useState<MeetingHistoryItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<MeetingHistoryDetail | null>(null);
  const [phase, setPhase] = useState<RecordPhase>('idle');
  const [elapsed, setElapsed] = useState(0);
  const [summarizing, setSummarizing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const timerRef = useRef<number | null>(null);

  const refreshHistory = useCallback(async () => {
    if (!tauri) return;
    try {
      const page = await listMeetingHistory(null, 25, 0);
      setHistory(page.items);
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not load meeting history.'));
    }
  }, [tauri]);

  useEffect(() => {
    if (!tauri) return;
    void (async () => {
      try {
        const status: AppStatus = await getAppStatus();
        setSettings(status.defaultSettings);
      } catch (cause) {
        setError(messageFromUnknownError(cause, 'Could not reach the desktop backend.'));
      }
      try {
        setDevices(await listAudioDevices());
      } catch {
        /* device list is non-critical */
      }
      await refreshHistory();
    })();
  }, [tauri, refreshHistory]);

  const openMeeting = useCallback(async (meetingId: string) => {
    setSelectedId(meetingId);
    setView('meetings');
    try {
      setDetail(await getMeetingHistoryDetail(meetingId));
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not open this meeting.'));
    }
  }, []);

  const handleSummarize = useCallback(async (meetingId: string) => {
    setError(null);
    setSummarizing(true);
    try {
      await summarizeMeeting(meetingId);
      setDetail(await getMeetingHistoryDetail(meetingId));
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not generate notes. Is LM Studio installed?'));
    } finally {
      setSummarizing(false);
    }
  }, []);

  const startTimer = useCallback(() => {
    setElapsed(0);
    timerRef.current = window.setInterval(() => setElapsed((value) => value + 1), 1000);
  }, []);

  const stopTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  useEffect(() => () => stopTimer(), [stopTimer]);

  const handleStart = useCallback(async () => {
    setError(null);
    const meetingId = `meeting-${Date.now()}`;
    try {
      await startRecording(meetingId, settings.microphoneDeviceId ?? undefined);
      setPhase('recording');
      startTimer();
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not start recording.'));
    }
  }, [settings.microphoneDeviceId, startTimer]);

  const handleStop = useCallback(async () => {
    setPhase('stopping');
    stopTimer();
    try {
      const recording = await stopRecording();
      setPhase('transcribing');
      await transcribeMeeting(recording.meetingId);
      await refreshHistory();
      await openMeeting(recording.meetingId);
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Recording stopped, but processing failed.'));
    } finally {
      setPhase('idle');
    }
  }, [openMeeting, refreshHistory, stopTimer]);

  const isRecording = phase === 'recording';
  const isBusy = phase === 'stopping' || phase === 'transcribing';

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <BrandMark />
          <span className="brand-name">Scribe</span>
        </div>

        <button
          type="button"
          className={`record-btn${isRecording ? ' is-recording' : ''}`}
          onClick={isRecording ? handleStop : handleStart}
          disabled={!tauri || isBusy}
        >
          {isBusy ? (
            <>
              <Spinner /> {phase === 'transcribing' ? 'Transcribing…' : 'Stopping…'}
            </>
          ) : isRecording ? (
            <>
              <span className="dot" /> Stop · {formatClock(elapsed)}
            </>
          ) : (
            <>
              <span className="dot" /> Record meeting
            </>
          )}
        </button>

        <div>
          <p className="nav-label">Recent</p>
          <div className="meeting-list">
            {history.length === 0 && (
              <p className="mi-meta" style={{ padding: '6px 10px' }}>
                No meetings yet
              </p>
            )}
            {history.map((item) => (
              <button
                key={item.meetingId}
                type="button"
                className={`meeting-item${selectedId === item.meetingId ? ' is-active' : ''}${
                  item.transcriptSegmentCount === 0 ? ' is-muted' : ''
                }`}
                onClick={() => void openMeeting(item.meetingId)}
              >
                <p className="mi-title">{meetingTitle(item)}</p>
                <p className="mi-meta">
                  {formatDate(item.startedAtMs)} · {formatDuration(item.durationMs)}
                </p>
              </button>
            ))}
          </div>
        </div>

        <div className="sidebar-foot">
          <button
            type="button"
            className={`nav-btn${view === 'settings' ? ' is-active' : ''}`}
            onClick={() => setView('settings')}
          >
            <Icon name="settings" /> Settings
          </button>
        </div>
      </aside>

      <main className="main">
        <div className="main-narrow">
          {!tauri && (
            <div className="banner error">
              <Icon name="alert" />
              Open Scribe from the desktop app to record and process meetings.
            </div>
          )}
          {error && (
            <div className="banner error">
              <Icon name="alert" />
              {error}
            </div>
          )}

          {view === 'settings' ? (
            <SettingsView settings={settings} devices={devices} onSettings={setSettings} onError={setError} />
          ) : detail ? (
            <MeetingDetailView
              detail={detail}
              processing={phase === 'transcribing'}
              summarizing={summarizing}
              canSummarize={tauri}
              onSummarize={() => void handleSummarize(detail.meeting.meetingId)}
            />
          ) : (
            <EmptyState recording={isRecording} />
          )}
        </div>
      </main>

      {isRecording && (
        <div className="pill" role="status">
          <span className="pdot" />
          <span className="plabel">Recording</span>
          <span className="ptime">{formatClock(elapsed)}</span>
          <button type="button" className="pstop" aria-label="Stop recording" onClick={() => void handleStop()}>
            <Icon name="stop" />
          </button>
        </div>
      )}
    </div>
  );
}

const ICON_PATHS: Record<string, string> = {
  settings:
    'M10.3 4.3c.4-1.8 2.9-1.8 3.3 0a1.7 1.7 0 0 0 2.6 1.1c1.5-.9 3.3.8 2.4 2.4a1.7 1.7 0 0 0 1 2.5c1.8.5 1.8 3 0 3.4a1.7 1.7 0 0 0-1 2.6c.9 1.5-.8 3.3-2.4 2.4a1.7 1.7 0 0 0-2.6 1c-.4 1.8-2.9 1.8-3.3 0a1.7 1.7 0 0 0-2.6-1.1c-1.5.9-3.3-.8-2.4-2.4a1.7 1.7 0 0 0-1-2.5c-1.8-.5-1.8-3 0-3.4a1.7 1.7 0 0 0 1-2.6c-.9-1.5.8-3.3 2.4-2.4a1.7 1.7 0 0 0 2.6-1z M12 12m-3 0a3 3 0 1 0 6 0a3 3 0 1 0-6 0',
  calendar: 'M4 7a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2zM16 3v4M8 3v4M4 11h16',
  alert: 'M12 9v4M12 17h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z',
  mic: 'M9 6a3 3 0 0 1 6 0v5a3 3 0 0 1-6 0zM5 11a7 7 0 0 0 14 0M12 18v3',
  notes: 'M5 5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2zM9 7h6M9 11h6M9 15h4',
  sparkles: 'M12 3l1.8 5L19 10l-5.2 2L12 17l-1.8-5L5 10l5.2-2zM18 16l.7 2 .3 2 .3-2 .7-2 .7-.3-.7-.3z',
  stop: 'M7 7a1 1 0 0 1 1-1h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1z',
  refresh: 'M20 11a8 8 0 0 0-13.7-5L4 8M4 4v4h4M4 13a8 8 0 0 0 13.7 5L20 16M20 20v-4h-4',
  loader: 'M12 3a9 9 0 1 0 9 9',
};

function Icon({ name, size = 16 }: { name: keyof typeof ICON_PATHS | string; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ flexShrink: 0 }}
    >
      <path d={ICON_PATHS[name] ?? ''} />
    </svg>
  );
}

function BrandMark() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <rect x="0" y="0" width="24" height="24" rx="5.5" fill="var(--brand)" />
      <g fill="#ffffff">
        <rect x="6.2" y="10" width="1.6" height="4" rx="0.8" />
        <rect x="8.8" y="8" width="1.6" height="8" rx="0.8" />
        <rect x="11.2" y="6" width="1.6" height="12" rx="0.8" />
        <rect x="13.6" y="8.5" width="1.6" height="7" rx="0.8" />
        <rect x="16.2" y="10.25" width="1.6" height="3.5" rx="0.8" />
      </g>
    </svg>
  );
}

function Spinner() {
  return (
    <span className="spin" aria-hidden="true">
      <Icon name="loader" />
    </span>
  );
}

function EmptyState({ recording }: { recording: boolean }) {
  return (
    <div className="empty">
      <span className="ill">
        <Icon name={recording ? 'mic' : 'notes'} size={30} />
      </span>
      <h2>{recording ? 'Listening to your meeting' : 'No meeting selected'}</h2>
      <p>
        {recording
          ? 'Notes appear here once you stop and the recording is transcribed.'
          : 'Hit record when your meeting starts, or pick a past meeting from the left to read its notes.'}
      </p>
    </div>
  );
}

function MeetingDetailView({
  detail,
  processing,
  summarizing,
  canSummarize,
  onSummarize,
}: {
  detail: MeetingHistoryDetail;
  processing: boolean;
  summarizing: boolean;
  canSummarize: boolean;
  onSummarize: () => void;
}) {
  const { meeting, transcriptSegments, summary } = detail;
  const segmentCount = transcriptSegments.length;
  const hasTranscript = segmentCount > 0;

  return (
    <div>
      <div className="meeting-header">
        <div>
          <h1>{meetingTitle(meeting)}</h1>
          <p className="meeting-sub">
            <Icon name="calendar" /> {formatDate(meeting.startedAtMs)}
            <span>·</span> {formatDuration(meeting.durationMs)}
            <span>·</span> {meeting.transcriptSegmentCount} segments
          </p>
        </div>
        {summary && hasTranscript && (
          <button type="button" className="ghost-btn" disabled={summarizing} onClick={onSummarize}>
            {summarizing ? <Spinner /> : <Icon name="refresh" />} Regenerate
          </button>
        )}
      </div>

      {summarizing ? (
        <div className="pending">
          <Spinner />
          <span className="label" style={{ flex: 1 }}>
            Summarizing with Gemma on device…
            <br />
            <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>
              Loading the model can take up to a minute the first time.
            </span>
          </span>
        </div>
      ) : summary ? (
        <NotesView summary={summary} />
      ) : processing ? (
        <div className="pending">
          <Spinner />
          <span className="label">Transcribing the recording…</span>
        </div>
      ) : hasTranscript ? (
        <div className="pending">
          <Icon name="sparkles" />
          <span className="label" style={{ flex: 1 }}>
            Generate on-device notes from this transcript with Gemma.
          </span>
          <button type="button" className="primary-btn" disabled={!canSummarize} onClick={onSummarize}>
            <Icon name="sparkles" />
            Generate notes
          </button>
        </div>
      ) : (
        <div className="pending">
          <Icon name="sparkles" />
          <span className="label">Transcribe this meeting first to generate notes.</span>
        </div>
      )}

      <h2 className="section-title">Transcript</h2>
      {segmentCount === 0 ? (
        <p className="mi-meta">No transcript yet.</p>
      ) : (
        <div className="transcript">
          {transcriptSegments.map((segment) => (
            <div className="segment" key={segment.sequenceNumber}>
              <p className="speaker">
                {segment.speakerLabel ?? 'Speaker'}
                <span className="ts">{formatClock(Math.round(segment.startedAtMs / 1000))}</span>
              </p>
              <p className="text">{segment.text}</p>
            </div>
          ))}
        </div>
      )}
      {detail.transcriptTruncated && (
        <p className="mi-meta" style={{ marginTop: 12 }}>
          Transcript truncated.
        </p>
      )}
    </div>
  );
}

function NotesView({ summary }: { summary: MeetingSummary }) {
  return (
    <div>
      <div className="notes">
        {summary.executiveSummary && <p className="tldr">{summary.executiveSummary}</p>}
        {summary.decisions.length > 0 && (
          <>
            <h2>Decisions</h2>
            <ul>
              {summary.decisions.map((decision) => (
                <li key={decision}>{decision}</li>
              ))}
            </ul>
          </>
        )}
        {summary.openQuestions.length > 0 && (
          <>
            <h2>Open questions</h2>
            <ul>
              {summary.openQuestions.map((question) => (
                <li key={question}>{question}</li>
              ))}
            </ul>
          </>
        )}
      </div>

      {summary.actionItems.length > 0 && (
        <>
          <h2 className="section-title">Action items</h2>
          <div className="action-list">
            {summary.actionItems.map((item) => (
              <div className="action-item" key={`${item.owner ?? 'unassigned'}-${item.task}`}>
                {item.owner && <span className="owner-chip">{item.owner}</span>}
                <span className="task">{item.task}</span>
                {item.due && <span className="due">{item.due}</span>}
              </div>
            ))}
          </div>
        </>
      )}

      <div className="provenance">
        <span />
        <span className="on-device">
          <span className="seed" /> Summarized by Gemma · on device
        </span>
      </div>
    </div>
  );
}

function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} className={`toggle${on ? ' on' : ''}`} onClick={onClick} />
  );
}

function SettingsView({
  settings,
  devices,
  onSettings,
  onError,
}: {
  settings: ResonanceSettings;
  devices: AudioDevice[];
  onSettings: (settings: ResonanceSettings) => void;
  onError: (message: string | null) => void;
}) {
  const [transcriberBin, setTranscriberBin] = useState(settings.transcriberBinPath ?? '');
  const [transcriberModel, setTranscriberModel] = useState(settings.transcriberModelPath ?? '');

  const saveAudio = useCallback(
    async (next: Partial<Pick<ResonanceSettings, 'enableSystemAudio' | 'enableEchoCancellation'>>) => {
      onError(null);
      try {
        const updated = await updateAudioProcessingSettings(
          next.enableSystemAudio ?? settings.enableSystemAudio,
          next.enableEchoCancellation ?? settings.enableEchoCancellation,
        );
        onSettings(updated);
      } catch (cause) {
        onError(messageFromUnknownError(cause, 'Could not update audio settings.'));
      }
    },
    [onError, onSettings, settings.enableEchoCancellation, settings.enableSystemAudio],
  );

  const saveDictation = useCallback(
    async (next: Partial<Pick<ResonanceSettings, 'dictationHotkey' | 'dictationPolishEnabled'>>) => {
      onError(null);
      try {
        const updated = await updateDictationSettings(
          next.dictationHotkey ?? settings.dictationHotkey,
          next.dictationPolishEnabled ?? settings.dictationPolishEnabled,
        );
        onSettings(updated);
      } catch (cause) {
        onError(messageFromUnknownError(cause, 'Could not update dictation settings.'));
      }
    },
    [onError, onSettings, settings.dictationHotkey, settings.dictationPolishEnabled],
  );

  const saveRetention = useCallback(
    async (days: number) => {
      onError(null);
      try {
        const result = await updatePrivacySettings(days, settings.analyzerProvider, settings.cloudAnalysisEnabled);
        onSettings(result.settings);
      } catch (cause) {
        onError(messageFromUnknownError(cause, 'Could not update retention.'));
      }
    },
    [onError, onSettings, settings.analyzerProvider, settings.cloudAnalysisEnabled],
  );

  const saveTranscriber = useCallback(async () => {
    onError(null);
    try {
      const updated = await updateTranscriberSettings(
        transcriberBin.trim() || null,
        transcriberModel.trim() || null,
        settings.speakerEmbeddingModelPath,
        settings.speakerSegmentationModelPath,
      );
      onSettings(updated);
    } catch (cause) {
      onError(messageFromUnknownError(cause, 'Could not update transcriber paths.'));
    }
  }, [
    onError,
    onSettings,
    settings.speakerEmbeddingModelPath,
    settings.speakerSegmentationModelPath,
    transcriberBin,
    transcriberModel,
  ]);

  return (
    <div>
      <div className="meeting-header">
        <div>
          <h1>Settings</h1>
          <p className="meeting-sub">Everything stays on this Mac.</p>
        </div>
      </div>

      <section className="settings-group">
        <h2>Audio capture</h2>
        <p className="hint">Choose your microphone and how meeting audio is captured.</p>
        <div className="field">
          <div>
            <div className="field-label">Microphone</div>
            <p className="field-desc">Used as your voice channel.</p>
          </div>
          <select
            value={settings.microphoneDeviceId ?? ''}
            onChange={(event) => onSettings({ ...settings, microphoneDeviceId: event.target.value || null })}
          >
            <option value="">System default</option>
            {devices.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <div>
            <div className="field-label">Capture system audio</div>
            <p className="field-desc">Record what others say for context.</p>
          </div>
          <Toggle
            on={settings.enableSystemAudio}
            onClick={() => void saveAudio({ enableSystemAudio: !settings.enableSystemAudio })}
          />
        </div>
        <div className="field">
          <div>
            <div className="field-label">Echo cancellation</div>
            <p className="field-desc">Remove speaker bleed when you are not on headphones.</p>
          </div>
          <Toggle
            on={settings.enableEchoCancellation}
            onClick={() => void saveAudio({ enableEchoCancellation: !settings.enableEchoCancellation })}
          />
        </div>
      </section>

      <section className="settings-group">
        <h2>Dictation</h2>
        <p className="hint">Double-press the hotkey to start dictating, press it once to stop and insert.</p>
        <div className="field">
          <div>
            <div className="field-label">Hotkey</div>
            <p className="field-desc">
              Inserts dictation into whatever app is focused. Needs Accessibility permission.
            </p>
          </div>
          <select
            value={settings.dictationHotkey}
            onChange={(event) => void saveDictation({ dictationHotkey: event.target.value })}
          >
            {DICTATION_HOTKEYS.map((hotkey) => (
              <option key={hotkey.value} value={hotkey.value}>
                {hotkey.label}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <div>
            <div className="field-label">Polish with Apple Intelligence</div>
            <p className="field-desc">Clean up grammar and filler before inserting. Off inserts the raw transcript.</p>
          </div>
          <Toggle
            on={settings.dictationPolishEnabled}
            onClick={() => void saveDictation({ dictationPolishEnabled: !settings.dictationPolishEnabled })}
          />
        </div>
      </section>

      <section className="settings-group">
        <h2>Transcription</h2>
        <p className="hint">Point Scribe at your local whisper.cpp binary and model.</p>
        <div className="field">
          <div>
            <div className="field-label">whisper-cli path</div>
            <p className="field-desc">Leave blank to auto-detect on PATH.</p>
          </div>
          <input
            type="text"
            value={transcriberBin}
            placeholder="/opt/homebrew/bin/whisper-cli"
            onChange={(event) => setTranscriberBin(event.target.value)}
          />
        </div>
        <div className="field">
          <div>
            <div className="field-label">Model path</div>
            <p className="field-desc">A downloaded whisper.cpp .bin model.</p>
          </div>
          <input
            type="text"
            value={transcriberModel}
            placeholder="/path/to/ggml-small.bin"
            onChange={(event) => setTranscriberModel(event.target.value)}
          />
        </div>
        <div style={{ marginTop: 12 }}>
          <button type="button" className="primary-btn" onClick={() => void saveTranscriber()}>
            Save transcription paths
          </button>
        </div>
      </section>

      <section className="settings-group">
        <h2>Privacy</h2>
        <p className="hint">Audio is deleted after the retention window; transcripts and notes are kept.</p>
        <div className="field">
          <div>
            <div className="field-label">Keep raw audio for</div>
            <p className="field-desc">Days before recordings are deleted.</p>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <input
              type="number"
              min={1}
              max={365}
              value={settings.rawAudioRetentionDays}
              onChange={(event) => onSettings({ ...settings, rawAudioRetentionDays: Number(event.target.value) })}
              onBlur={(event) => void saveRetention(Number(event.target.value))}
            />
            <span className="field-desc">days</span>
          </div>
        </div>
      </section>
    </div>
  );
}
