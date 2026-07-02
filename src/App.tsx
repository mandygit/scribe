import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import './styles.css';
import { DICTATION_HOTKEYS, PERMISSION_ROWS, SUMMARIZER_PROVIDERS, THEME_PREFERENCES } from './contracts';
import { messageFromUnknownError } from './error-utils';
import { formatClock, formatDate, formatDuration, meetingTitle } from './format';
import { copySummaryToClipboard } from './summary-clipboard';
import {
  type AppStatus,
  type AudioDevice,
  checkPermissions,
  type DictationSessionRecord,
  type DictationStatsSummary,
  deleteDictationSession,
  deleteMeeting,
  getAppStatus,
  getDictationStatsSummary,
  getMeetingHistoryDetail,
  isTauriRuntime,
  listAudioDevices,
  listDictationSessions,
  listMeetingHistory,
  listMeetingTrends,
  listSummarizerModels,
  type MeetingHistoryDetail,
  type MeetingHistoryItem,
  type MeetingSummary,
  type MeetingTrendPoint,
  openPermissionSettings,
  type PermissionsSnapshot,
  type ResonanceSettings,
  sendCompletionNotification,
  startRecording,
  stopRecording,
  summarizeMeeting,
  transcribeMeeting,
  updateAudioProcessingSettings,
  updateDictationSettings,
  updatePrivacySettings,
  updateMeetingTitle,
  updateSummarizerSettings,
  updateThemePreference,
  updateTranscriberSettings,
} from './tauri-commands';

const PERMISSIONS_ONBOARDING_SEEN_KEY = 'scribe-permissions-onboarding-seen';

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
  summarizerProvider: 'lmStudio',
  summarizerHost: '127.0.0.1',
  summarizerPort: 1234,
  summarizerModel: null,
  themePreference: 'system',
};

type View = 'meetings' | 'trends' | 'dictation' | 'settings';
const DICTATION_HISTORY_LIMIT = 50;
const TRENDS_LIMIT = 20;
type RecordPhase = 'idle' | 'recording' | 'stopping';

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
  const [dictationStats, setDictationStats] = useState<DictationStatsSummary | null>(null);
  const [dictationSessions, setDictationSessions] = useState<DictationSessionRecord[]>([]);
  const [trendPoints, setTrendPoints] = useState<MeetingTrendPoint[]>([]);
  const [permissions, setPermissions] = useState<PermissionsSnapshot | null>(null);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [confirmDialog, setConfirmDialog] = useState<{ message: string; onConfirm: () => void } | null>(null);
  const [transcribingMeetingId, setTranscribingMeetingId] = useState<string | null>(null);
  const timerRef = useRef<number | null>(null);
  const detailMeetingIdRef = useRef<string | null>(null);

  const refreshPermissions = useCallback(async () => {
    if (!tauri) return;
    try {
      setPermissions(await checkPermissions());
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not check permissions.'));
    }
  }, [tauri]);

  const refreshDevices = useCallback(async () => {
    if (!tauri) return;
    try {
      setDevices(await listAudioDevices());
    } catch {
      /* device list is non-critical */
    }
  }, [tauri]);

  const refreshHistory = useCallback(async () => {
    if (!tauri) return;
    try {
      const page = await listMeetingHistory(null, 25, 0);
      setHistory(page.items);
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not load meeting history.'));
    }
  }, [tauri]);

  const refreshTrends = useCallback(async () => {
    if (!tauri) return;
    try {
      const result = await listMeetingTrends(TRENDS_LIMIT);
      setTrendPoints(result.points);
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not load meeting trends.'));
    }
  }, [tauri]);

  const refreshDictation = useCallback(async () => {
    if (!tauri) return;
    try {
      const [stats, page] = await Promise.all([
        getDictationStatsSummary(),
        listDictationSessions(DICTATION_HISTORY_LIMIT, 0),
      ]);
      setDictationStats(stats);
      setDictationSessions(page.items);
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not load dictation history.'));
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
      await refreshDevices();
      try {
        const snapshot = await checkPermissions();
        setPermissions(snapshot);
        const hasSeenOnboarding = window.localStorage.getItem(PERMISSIONS_ONBOARDING_SEEN_KEY) === 'true';
        const allGranted = Object.values(snapshot).every((status) => status === 'granted');
        if (!hasSeenOnboarding && !allGranted) {
          setShowOnboarding(true);
        }
      } catch {
        /* permission check is best-effort on launch; Settings can retry */
      }
      await refreshHistory();
    })();
  }, [tauri, refreshHistory, refreshDevices]);

  useEffect(() => {
    if (!tauri || view !== 'settings') return;
    void refreshDevices();
    const intervalId = window.setInterval(() => void refreshDevices(), 3000);
    return () => window.clearInterval(intervalId);
  }, [tauri, view, refreshDevices]);

  useEffect(() => {
    const applyResolvedTheme = (query: MediaQueryList | MediaQueryListEvent) => {
      const resolved =
        settings.themePreference === 'system'
          ? (query.matches ? 'dark' : 'light')
          : settings.themePreference;
      document.documentElement.dataset.theme = resolved;
    };
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    applyResolvedTheme(media);
    if (settings.themePreference !== 'system') return;
    media.addEventListener('change', applyResolvedTheme);
    return () => media.removeEventListener('change', applyResolvedTheme);
  }, [settings.themePreference]);

  useEffect(() => {
    if (view === 'dictation') {
      void refreshDictation();
    } else if (view === 'trends') {
      void refreshTrends();
    }
  }, [view, refreshDictation, refreshTrends]);

  const openMeeting = useCallback(async (meetingId: string) => {
    setSelectedId(meetingId);
    setView('meetings');
    try {
      setDetail(await getMeetingHistoryDetail(meetingId));
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not open this meeting.'));
    }
  }, []);

  useEffect(() => {
    detailMeetingIdRef.current = detail?.meeting.meetingId ?? null;
  }, [detail]);

  const requestConfirm = useCallback((message: string, onConfirm: () => void) => {
    setConfirmDialog({ message, onConfirm });
  }, []);

  const handleDeleteMeeting = useCallback(
    (meetingId: string) => {
      requestConfirm('Delete this meeting recording and its transcript and notes? This cannot be undone.', () => {
        void (async () => {
          setError(null);
          try {
            await deleteMeeting(meetingId);
            if (selectedId === meetingId) {
              setSelectedId(null);
              setDetail(null);
            }
            await refreshHistory();
          } catch (cause) {
            setError(messageFromUnknownError(cause, 'Could not delete this meeting.'));
          }
        })();
      });
    },
    [refreshHistory, requestConfirm, selectedId],
  );

  const handleRenameMeeting = useCallback(async (meetingId: string, title: string) => {
    const normalized = title.trim() || null;
    setError(null);
    try {
      await updateMeetingTitle(meetingId, title);
      setHistory((current) =>
        current.map((item) => (item.meetingId === meetingId ? { ...item, title: normalized } : item)),
      );
      setDetail((current) =>
        current && current.meeting.meetingId === meetingId
          ? { ...current, meeting: { ...current.meeting, title: normalized } }
          : current,
      );
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not rename this meeting.'));
    }
  }, []);

  const handleDeleteDictationSession = useCallback(
    (sessionId: string) => {
      requestConfirm('Delete this dictation session?', () => {
        void (async () => {
          setError(null);
          try {
            await deleteDictationSession(sessionId);
            await refreshDictation();
          } catch (cause) {
            setError(messageFromUnknownError(cause, 'Could not delete this dictation session.'));
          }
        })();
      });
    },
    [refreshDictation, requestConfirm],
  );

  const handleSummarize = useCallback(async (meetingId: string) => {
    setError(null);
    setSummarizing(true);
    try {
      await summarizeMeeting(meetingId);
      const updated = await getMeetingHistoryDetail(meetingId);
      setDetail(updated);
      setHistory((current) =>
        current.map((item) =>
          item.meetingId === meetingId ? { ...item, title: updated.meeting.title } : item,
        ),
      );
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

  // Transcribing a long meeting can take much longer than the meeting
  // itself (a slow local whisper-cli run), so it happens in the background
  // instead of blocking the record button / "stop" flow on it.
  const transcribeInBackground = useCallback(
    async (meetingId: string) => {
      setTranscribingMeetingId(meetingId);
      try {
        await transcribeMeeting(meetingId);
        await refreshHistory();
        if (detailMeetingIdRef.current === meetingId) {
          await openMeeting(meetingId);
        }
        try {
          await sendCompletionNotification('Transcription ready', 'Your meeting transcript has finished processing.');
        } catch {
          /* notification is best-effort */
        }
      } catch (cause) {
        setError(messageFromUnknownError(cause, 'Transcription failed.'));
        await refreshHistory();
      } finally {
        setTranscribingMeetingId((current) => (current === meetingId ? null : current));
      }
    },
    [openMeeting, refreshHistory],
  );

  const handleStop = useCallback(async () => {
    setPhase('stopping');
    stopTimer();
    try {
      const recording = await stopRecording();
      await refreshHistory();
      await openMeeting(recording.meetingId);
      void transcribeInBackground(recording.meetingId);
    } catch (cause) {
      setError(messageFromUnknownError(cause, 'Could not stop recording.'));
    } finally {
      setPhase('idle');
    }
  }, [openMeeting, refreshHistory, stopTimer, transcribeInBackground]);

  const dismissOnboarding = useCallback(() => {
    window.localStorage.setItem(PERMISSIONS_ONBOARDING_SEEN_KEY, 'true');
    setShowOnboarding(false);
  }, []);

  const isRecording = phase === 'recording';
  const isBusy = phase === 'stopping';

  return (
    <div className="app">
      {showOnboarding && permissions && (
        <PermissionsOnboarding
          permissions={permissions}
          onRecheck={refreshPermissions}
          onContinue={dismissOnboarding}
        />
      )}
      {confirmDialog && (
        <ConfirmDialog
          message={confirmDialog.message}
          onCancel={() => setConfirmDialog(null)}
          onConfirm={() => {
            confirmDialog.onConfirm();
            setConfirmDialog(null);
          }}
        />
      )}
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
              <Spinner /> Stopping…
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

        <div className="sidebar-recent">
          <p className="nav-label">Recent</p>
          <div className="meeting-list">
            {history.length === 0 && (
              <p className="mi-meta" style={{ padding: '6px 10px' }}>
                No meetings yet
              </p>
            )}
            {history.map((item) => (
              <div
                key={item.meetingId}
                className={`meeting-item${selectedId === item.meetingId ? ' is-active' : ''}${
                  item.transcriptSegmentCount === 0 ? ' is-muted' : ''
                }`}
              >
                <button type="button" className="mi-open" onClick={() => void openMeeting(item.meetingId)}>
                  <p className="mi-title">{meetingTitle(item)}</p>
                  <p className="mi-meta">
                    {transcribingMeetingId === item.meetingId
                      ? 'Transcribing…'
                      : `${formatDate(item.startedAtMs)} · ${formatDuration(item.durationMs)}`}
                  </p>
                </button>
                <button
                  type="button"
                  className="mi-delete"
                  aria-label="Delete meeting"
                  onClick={() => void handleDeleteMeeting(item.meetingId)}
                >
                  <Icon name="trash" size={13} />
                </button>
              </div>
            ))}
          </div>
        </div>

        <div className="sidebar-foot">
          <button
            type="button"
            className={`nav-btn${view === 'trends' ? ' is-active' : ''}`}
            onClick={() => setView('trends')}
          >
            <Icon name="trend" /> Trends
          </button>
          <button
            type="button"
            className={`nav-btn${view === 'dictation' ? ' is-active' : ''}`}
            onClick={() => setView('dictation')}
          >
            <Icon name="activity" /> Dictation
          </button>
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
            <SettingsView
              settings={settings}
              devices={devices}
              onSettings={setSettings}
              onError={setError}
              onReviewPermissions={() => {
                void refreshPermissions();
                setShowOnboarding(true);
              }}
            />
          ) : view === 'dictation' ? (
            <DictationView
              stats={dictationStats}
              sessions={dictationSessions}
              onDelete={(sessionId) => void handleDeleteDictationSession(sessionId)}
            />
          ) : view === 'trends' ? (
            <TrendsView points={trendPoints} />
          ) : detail ? (
            <MeetingDetailView
              detail={detail}
              processing={transcribingMeetingId === detail.meeting.meetingId}
              summarizing={summarizing}
              canSummarize={tauri}
              onSummarize={() => void handleSummarize(detail.meeting.meetingId)}
              onDelete={() => void handleDeleteMeeting(detail.meeting.meetingId)}
              onRename={(title) => void handleRenameMeeting(detail.meeting.meetingId, title)}
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
  activity: 'M3 12h4l2-7 4 14 2-7h6',
  trend: 'M3 17l6-6 4 4 8-8M15 7h6v6',
  trash: 'M5 7h14M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2m2 0-.8 12.2a2 2 0 0 1-2 1.8H7.8a2 2 0 0 1-2-1.8L5 7h14z',
  copy: 'M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2zM4 16V4a2 2 0 0 1 2-2h10',
  check: 'M5 12l5 5L19 7',
  edit: 'M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z',
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

const formatSessionTimestamp = (ms: number): string =>
  new Date(ms).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });

function DictationView({
  stats,
  sessions,
  onDelete,
}: {
  stats: DictationStatsSummary | null;
  sessions: DictationSessionRecord[];
  onDelete: (sessionId: string) => void;
}) {
  const hasSessions = sessions.length > 0;

  return (
    <div>
      <div className="meeting-header">
        <div>
          <h1>Dictation</h1>
          <p className="meeting-sub">How much you've dictated on this Mac.</p>
        </div>
      </div>

      <div className="stat-grid">
        <StatCard label="Sessions" value={stats ? stats.totalSessions.toLocaleString() : '—'} />
        <StatCard label="Words dictated" value={stats ? stats.totalWords.toLocaleString() : '—'} />
        <StatCard
          label="Avg. pace"
          value={stats && stats.totalSessions > 0 ? `${Math.round(stats.averageWordsPerMinute)} wpm` : '—'}
        />
        <StatCard label="Total time" value={stats ? formatDuration(stats.totalDurationMs) : '—'} />
      </div>

      <h2 className="section-title">History</h2>
      {hasSessions ? (
        <div className="dictation-list">
          {sessions.map((session) => (
            <div className="dictation-item" key={session.id}>
              <span className="di-time">{formatSessionTimestamp(session.startedAtMs)}</span>
              <span className="di-metric">{formatDuration(session.durationMs)}</span>
              <span className="di-metric">{session.wordCount} words</span>
              <span className="di-metric">{Math.round(session.wordsPerMinute)} wpm</span>
              <button
                type="button"
                className="mi-delete"
                aria-label="Delete dictation session"
                onClick={() => onDelete(session.id)}
              >
                <Icon name="trash" size={13} />
              </button>
            </div>
          ))}
        </div>
      ) : (
        <div className="empty" style={{ height: 'auto', padding: '32px 0' }}>
          <span className="ill">
            <Icon name="mic" size={26} />
          </span>
          <h2>No dictations yet</h2>
          <p>Double-press your dictation hotkey to start, press it again to insert. Sessions show up here.</p>
        </div>
      )}
    </div>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat-card">
      <p className="stat-value">{value}</p>
      <p className="stat-label">{label}</p>
    </div>
  );
}

const SPARKLINE_WIDTH = 600;
const SPARKLINE_HEIGHT = 64;
const SPARKLINE_PADDING = 6;

/** A minimal hand-rolled line chart, matching the app's hand-rolled icon style
 * rather than pulling in a charting dependency for three sparklines. */
function Sparkline({ values, color }: { values: (number | null)[]; color: string }) {
  const defined = values
    .map((value, index) => ({ value, index }))
    .filter((entry): entry is { value: number; index: number } => entry.value !== null);

  if (defined.length < 2) {
    return <div className="sparkline-empty">Not enough meetings with this metric yet.</div>;
  }

  const min = Math.min(...defined.map((entry) => entry.value));
  const max = Math.max(...defined.map((entry) => entry.value));
  const span = max - min || 1;
  const stepX = (SPARKLINE_WIDTH - SPARKLINE_PADDING * 2) / Math.max(values.length - 1, 1);

  const toPoint = (entry: { value: number; index: number }) => {
    const x = SPARKLINE_PADDING + entry.index * stepX;
    const y =
      SPARKLINE_HEIGHT - SPARKLINE_PADDING - ((entry.value - min) / span) * (SPARKLINE_HEIGHT - SPARKLINE_PADDING * 2);
    return { x, y };
  };

  // Breaks the line at gaps where a meeting has no value for this metric yet.
  let path = '';
  let previousIndex: number | null = null;
  for (const entry of defined) {
    const { x, y } = toPoint(entry);
    const command = previousIndex !== null && entry.index === previousIndex + 1 ? 'L' : 'M';
    path += `${command}${x},${y} `;
    previousIndex = entry.index;
  }

  return (
    <svg
      className="sparkline"
      viewBox={`0 0 ${SPARKLINE_WIDTH} ${SPARKLINE_HEIGHT}`}
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <path d={path} fill="none" stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
      {defined.map((entry) => {
        const { x, y } = toPoint(entry);
        return <circle key={entry.index} cx={x} cy={y} r={2.5} fill={color} />;
      })}
    </svg>
  );
}

function TrendCard({
  label,
  points,
  values,
  format,
  color,
}: {
  label: string;
  points: MeetingTrendPoint[];
  values: (number | null)[];
  format: (value: number) => string;
  color: string;
}) {
  const latest = [...values].reverse().find((value) => value !== null) ?? null;
  const withData = values.filter((value) => value !== null).length;

  return (
    <div className="trend-card">
      <div className="trend-head">
        <div>
          <p className="stat-label">{label}</p>
          <p className="stat-value">{latest !== null ? format(latest) : '—'}</p>
        </div>
        <span className="trend-count">
          {withData} of {points.length} meetings
        </span>
      </div>
      <Sparkline values={values} color={color} />
    </div>
  );
}

function TrendsView({ points }: { points: MeetingTrendPoint[] }) {
  const hasPoints = points.length > 0;

  return (
    <div>
      <div className="meeting-header">
        <div>
          <h1>Trends</h1>
          <p className="meeting-sub">How your speaking has changed across recent meetings.</p>
        </div>
      </div>

      {hasPoints ? (
        <div className="trend-list">
          <TrendCard
            label="Overall score"
            points={points}
            values={points.map((point) => point.overallScore)}
            format={(value) => `${Math.round(value)}`}
            color="var(--accent)"
          />
          <TrendCard
            label="Pace"
            points={points}
            values={points.map((point) => point.wordsPerMinute)}
            format={(value) => `${Math.round(value)} wpm`}
            color="var(--brand)"
          />
          <TrendCard
            label="Filler words"
            points={points}
            values={points.map((point) => point.fillerWordCount)}
            format={(value) => `${Math.round(value)}`}
            color="var(--danger)"
          />
        </div>
      ) : (
        <div className="empty" style={{ height: 'auto', padding: '32px 0' }}>
          <span className="ill">
            <Icon name="trend" size={26} />
          </span>
          <h2>No trends yet</h2>
          <p>Record and analyze a few meetings and your pace, filler words, and score will show up here.</p>
        </div>
      )}
    </div>
  );
}

function MeetingDetailView({
  detail,
  processing,
  summarizing,
  canSummarize,
  onSummarize,
  onDelete,
  onRename,
}: {
  detail: MeetingHistoryDetail;
  processing: boolean;
  summarizing: boolean;
  canSummarize: boolean;
  onSummarize: () => void;
  onDelete: () => void;
  onRename: (title: string) => void;
}) {
  const { meeting, transcriptSegments, summary } = detail;
  const segmentCount = transcriptSegments.length;
  const hasTranscript = segmentCount > 0;
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'error'>('idle');
  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(meetingTitle(meeting));

  useEffect(() => {
    if (!isEditingTitle) {
      setTitleDraft(meetingTitle(meeting));
    }
  }, [meeting, isEditingTitle]);

  const commitTitle = useCallback(() => {
    setIsEditingTitle(false);
    if (titleDraft.trim() !== (meeting.title ?? '').trim()) {
      onRename(titleDraft);
    }
  }, [titleDraft, meeting.title, onRename]);

  const handleCopy = useCallback(async () => {
    if (!summary) return;
    try {
      await copySummaryToClipboard(summary);
      setCopyState('copied');
    } catch {
      setCopyState('error');
    }
    window.setTimeout(() => setCopyState('idle'), 2000);
  }, [summary]);

  return (
    <div>
      <div className="meeting-header">
        <div>
          {isEditingTitle ? (
            <input
              type="text"
              className="meeting-title-input"
              value={titleDraft}
              autoFocus
              onChange={(event) => setTitleDraft(event.target.value)}
              onBlur={commitTitle}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.currentTarget.blur();
                } else if (event.key === 'Escape') {
                  setTitleDraft(meetingTitle(meeting));
                  setIsEditingTitle(false);
                }
              }}
            />
          ) : (
            <h1 className="meeting-title-display" onClick={() => setIsEditingTitle(true)}>
              {meetingTitle(meeting)}
              <button
                type="button"
                className="title-edit-btn"
                aria-label="Rename meeting"
                onClick={(event) => {
                  event.stopPropagation();
                  setIsEditingTitle(true);
                }}
              >
                <Icon name="edit" size={13} />
              </button>
            </h1>
          )}
          <p className="meeting-sub">
            <Icon name="calendar" /> {formatDate(meeting.startedAtMs)}
            <span>·</span> {formatDuration(meeting.durationMs)}
            <span>·</span> {meeting.transcriptSegmentCount} segments
          </p>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          {summary && hasTranscript && (
            <button type="button" className="ghost-btn" onClick={() => void handleCopy()}>
              <Icon name={copyState === 'copied' ? 'check' : 'copy'} />
              {copyState === 'copied' ? 'Copied' : copyState === 'error' ? 'Copy failed' : 'Copy'}
            </button>
          )}
          {summary && hasTranscript && (
            <button type="button" className="ghost-btn" disabled={summarizing} onClick={onSummarize}>
              {summarizing ? <Spinner /> : <Icon name="refresh" />} Regenerate
            </button>
          )}
          <button type="button" className="ghost-btn" onClick={onDelete}>
            <Icon name="trash" /> Delete
          </button>
        </div>
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

function PermissionsOnboarding({
  permissions,
  onRecheck,
  onContinue,
}: {
  permissions: PermissionsSnapshot;
  onRecheck: () => void;
  onContinue: () => void;
}) {
  const [openingPane, setOpeningPane] = useState<string | null>(null);
  const allGranted = Object.values(permissions).every((status) => status === 'granted');

  const openPane = useCallback(async (pane: 'Microphone' | 'ScreenCapture' | 'Accessibility') => {
    setOpeningPane(pane);
    try {
      await openPermissionSettings(pane);
    } catch {
      /* best-effort deep link; the settings-group description already explains the manual path */
    } finally {
      setOpeningPane(null);
    }
  }, []);

  return (
    <div className="onboarding-overlay">
      <div className="onboarding-card">
        <h1>Set up Scribe</h1>
        <p className="meeting-sub">
          Scribe records and transcribes entirely on this Mac. It needs a few permissions to work — grant what you can,
          skip the rest for now.
        </p>
        <div className="permission-list">
          {PERMISSION_ROWS.map((row) => {
            const status = permissions[row.key];
            return (
              <div className="permission-row" key={row.key}>
                <div>
                  <div className="field-label">{row.label}</div>
                  <p className="field-desc">{row.description}</p>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span className={`permission-status${status === 'granted' ? ' is-granted' : ' is-denied'}`}>
                    {status === 'granted' ? 'Granted' : 'Not granted'}
                  </span>
                  {status !== 'granted' && (
                    <button
                      type="button"
                      className="ghost-btn"
                      disabled={openingPane === row.pane}
                      onClick={() => void openPane(row.pane)}
                    >
                      Open Settings
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
        <div className="onboarding-actions">
          <button type="button" className="ghost-btn" onClick={onRecheck}>
            <Icon name="refresh" /> Recheck
          </button>
          <button type="button" className="primary-btn" onClick={onContinue}>
            {allGranted ? 'Continue' : 'Continue without granting everything'}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Tauri's webview doesn't reliably implement `window.confirm`, so destructive
 * actions (delete meeting, delete dictation session) route through this
 * instead of the native dialog. */
function ConfirmDialog({
  message,
  onCancel,
  onConfirm,
}: {
  message: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="onboarding-overlay">
      <div className="onboarding-card confirm-card">
        <p className="confirm-message">{message}</p>
        <div className="onboarding-actions">
          <button type="button" className="ghost-btn" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="primary-btn is-danger" onClick={onConfirm}>
            <Icon name="trash" /> Delete
          </button>
        </div>
      </div>
    </div>
  );
}

function SettingsView({
  settings,
  devices,
  onSettings,
  onError,
  onReviewPermissions,
}: {
  settings: ResonanceSettings;
  devices: AudioDevice[];
  onSettings: (settings: ResonanceSettings) => void;
  onError: (message: string | null) => void;
  onReviewPermissions: () => void;
}) {
  const [transcriberBin, setTranscriberBin] = useState(settings.transcriberBinPath ?? '');
  const [transcriberModel, setTranscriberModel] = useState(settings.transcriberModelPath ?? '');
  const [summarizerHostInput, setSummarizerHostInput] = useState(settings.summarizerHost);
  const [summarizerPortInput, setSummarizerPortInput] = useState(String(settings.summarizerPort));
  const [summarizerModelInput, setSummarizerModelInput] = useState(settings.summarizerModel ?? '');
  const [modelOptions, setModelOptions] = useState<string[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [detectStatus, setDetectStatus] = useState<string | null>(null);

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

  const saveSummarizer = useCallback(
    async (
      next: Partial<
        Pick<ResonanceSettings, 'summarizerProvider' | 'summarizerHost' | 'summarizerPort' | 'summarizerModel'>
      >,
    ) => {
      onError(null);
      try {
        const updated = await updateSummarizerSettings(
          next.summarizerProvider ?? settings.summarizerProvider,
          next.summarizerHost ?? settings.summarizerHost,
          next.summarizerPort ?? settings.summarizerPort,
          next.summarizerModel !== undefined ? next.summarizerModel : settings.summarizerModel,
        );
        onSettings(updated);
        setSummarizerHostInput(updated.summarizerHost);
        setSummarizerPortInput(String(updated.summarizerPort));
        setSummarizerModelInput(updated.summarizerModel ?? '');
      } catch (cause) {
        onError(messageFromUnknownError(cause, 'Could not update the local model settings.'));
      }
    },
    [
      onError,
      onSettings,
      settings.summarizerHost,
      settings.summarizerModel,
      settings.summarizerPort,
      settings.summarizerProvider,
    ],
  );

  const detectModels = useCallback(async () => {
    setDetecting(true);
    setDetectStatus(null);
    try {
      const port = Number(summarizerPortInput) || settings.summarizerPort;
      const models = await listSummarizerModels(settings.summarizerProvider, summarizerHostInput, port);
      setModelOptions(models);
      setDetectStatus(
        models.length > 0
          ? `Found ${models.length} model${models.length === 1 ? '' : 's'}.`
          : 'Connected, but no models were listed — type the model name below.',
      );
    } catch (cause) {
      setModelOptions([]);
      setDetectStatus(messageFromUnknownError(cause, 'Could not reach that server.'));
    } finally {
      setDetecting(false);
    }
  }, [settings.summarizerPort, settings.summarizerProvider, summarizerHostInput, summarizerPortInput]);

  const saveTheme = useCallback(
    async (themePreference: ResonanceSettings['themePreference']) => {
      onError(null);
      try {
        onSettings(await updateThemePreference(themePreference));
      } catch (cause) {
        onError(messageFromUnknownError(cause, 'Could not update appearance.'));
      }
    },
    [onError, onSettings],
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
        <h2>Appearance</h2>
        <p className="hint">Choose how Scribe looks, or follow your Mac's system setting.</p>
        <div className="field">
          <div>
            <div className="field-label">Theme</div>
            <p className="field-desc">System matches your Mac's light/dark setting automatically.</p>
          </div>
          <select
            value={settings.themePreference}
            onChange={(event) =>
              void saveTheme(event.target.value as ResonanceSettings['themePreference'])
            }
          >
            {THEME_PREFERENCES.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>
      </section>

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
        <h2>Local model</h2>
        <p className="hint">
          Meeting notes are written by a model running on this Mac — point Scribe at whatever you have installed.
        </p>
        <div className="field">
          <div>
            <div className="field-label">Provider</div>
            <p className="field-desc">
              LM Studio and Ollama are detected automatically; pick Custom for anything else.
            </p>
          </div>
          <select
            value={settings.summarizerProvider}
            onChange={(event) => {
              const provider = event.target.value as ResonanceSettings['summarizerProvider'];
              const preset = SUMMARIZER_PROVIDERS.find((entry) => entry.value === provider);
              setModelOptions([]);
              setDetectStatus(null);
              if (preset) {
                setSummarizerHostInput(preset.defaultHost);
                setSummarizerPortInput(String(preset.defaultPort));
                void saveSummarizer({
                  summarizerProvider: provider,
                  summarizerHost: preset.defaultHost,
                  summarizerPort: preset.defaultPort,
                });
              }
            }}
          >
            {SUMMARIZER_PROVIDERS.map((provider) => (
              <option key={provider.value} value={provider.value}>
                {provider.label}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <div>
            <div className="field-label">Address</div>
            <p className="field-desc">Host and port the server is listening on.</p>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <input
              type="text"
              value={summarizerHostInput}
              onChange={(event) => setSummarizerHostInput(event.target.value)}
              onBlur={() => void saveSummarizer({ summarizerHost: summarizerHostInput })}
              style={{ minWidth: 140 }}
            />
            <span className="field-desc">:</span>
            <input
              type="text"
              value={summarizerPortInput}
              onChange={(event) => setSummarizerPortInput(event.target.value)}
              onBlur={() => {
                const port = Number(summarizerPortInput);
                if (Number.isInteger(port) && port > 0) {
                  void saveSummarizer({ summarizerPort: port });
                }
              }}
              style={{ width: 84 }}
            />
          </div>
        </div>
        <div className="field">
          <div>
            <div className="field-label">Model</div>
            <p className="field-desc">
              {detectStatus ?? 'Detect the models this server has available, or type a model name.'}
            </p>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {modelOptions.length > 0 ? (
              <select
                value={summarizerModelInput}
                onChange={(event) => {
                  setSummarizerModelInput(event.target.value);
                  void saveSummarizer({ summarizerModel: event.target.value || null });
                }}
              >
                <option value="">Choose a model</option>
                {modelOptions.map((model) => (
                  <option key={model} value={model}>
                    {model}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={summarizerModelInput}
                placeholder="e.g. llama3.2"
                onChange={(event) => setSummarizerModelInput(event.target.value)}
                onBlur={() => void saveSummarizer({ summarizerModel: summarizerModelInput.trim() || null })}
              />
            )}
            <button type="button" className="ghost-btn" disabled={detecting} onClick={() => void detectModels()}>
              {detecting ? <Spinner /> : <Icon name="refresh" />} Detect
            </button>
          </div>
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
        <h2>Permissions</h2>
        <p className="hint">Microphone, Screen Recording, and Accessibility — review status and re-grant if needed.</p>
        <div style={{ marginTop: 4 }}>
          <button type="button" className="ghost-btn" onClick={onReviewPermissions}>
            Review permissions
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
