import { getCurrentWindow } from '@tauri-apps/api/window';
import type { AnalysisResult, MeetingId, ReportId } from './contracts';
import { sendCompletionNotification } from './tauri-commands';

export interface CompletionNotificationPayload {
  meetingId: MeetingId;
  reportId: ReportId;
  overallScore: number;
}

export interface WebNotificationApi {
  permission: NotificationPermission;
  requestPermission: () => Promise<NotificationPermission>;
  create: (title: string, options: NotificationOptions) => Pick<Notification, 'onclick'>;
}

export interface CompletionNotificationDeps {
  webNotificationApi: () => WebNotificationApi | null;
  sendFallbackNotification: (title: string, body: string) => Promise<void>;
  focusWindow: () => Promise<void>;
}

export type CompletionNotificationStatus = 'sent' | 'fallback-sent' | 'permission-denied';

const notificationText = (payload: CompletionNotificationPayload): { title: string; body: string } => ({
  title: 'Meeting analysis ready',
  body: `Score ${payload.overallScore}/100. Open the saved report in Resonance.`,
});

const defaultNotificationDeps: CompletionNotificationDeps = {
  webNotificationApi: () => {
    if (typeof globalThis.Notification !== 'function') {
      return null;
    }

    return {
      get permission() {
        return globalThis.Notification.permission;
      },
      requestPermission: () => globalThis.Notification.requestPermission(),
      create: (title, options) => new globalThis.Notification(title, options),
    };
  },
  sendFallbackNotification: sendCompletionNotification,
  focusWindow: async () => {
    const window = getCurrentWindow();
    await window.show();
    await window.setFocus();
  },
};

export const notificationPayloadFromAnalysis = (analysis: AnalysisResult): CompletionNotificationPayload => ({
  meetingId: analysis.meetingId,
  reportId: analysis.reportId,
  overallScore: analysis.scorecard.overall.score ?? analysis.analysis.overallScore,
});

export const notifyAnalysisComplete = async (
  payload: CompletionNotificationPayload,
  onOpenReport: (meetingId: MeetingId) => void,
  deps: CompletionNotificationDeps = defaultNotificationDeps,
): Promise<CompletionNotificationStatus> => {
  const { title, body } = notificationText(payload);
  const webNotificationApi = deps.webNotificationApi();

  if (webNotificationApi !== null) {
    let permission = webNotificationApi.permission;
    if (permission === 'default') {
      permission = await webNotificationApi.requestPermission();
    }

    if (permission === 'granted') {
      const notification = webNotificationApi.create(title, {
        body,
        tag: payload.reportId,
      });
      notification.onclick = () => {
        void (async () => {
          await deps.focusWindow();
          onOpenReport(payload.meetingId);
        })();
      };
      return 'sent';
    }

    if (permission === 'denied') {
      return 'permission-denied';
    }
  }

  await deps.sendFallbackNotification(title, body);
  return 'fallback-sent';
};
