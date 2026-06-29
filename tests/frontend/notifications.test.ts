import { describe, expect, it } from 'bun:test';
import {
  type CompletionNotificationDeps,
  notificationPayloadFromAnalysis,
  notifyAnalysisComplete,
  type WebNotificationApi,
} from '../../src/notifications';
import type { AnalysisResult, MeetingId } from '../../src/tauri-commands';

interface CapturedWebNotification {
  title: string;
  options: NotificationOptions;
  onclick: ((event: Event) => void) | null;
}

interface FakeNotificationDeps extends CompletionNotificationDeps {
  fallbackNotifications: Array<{ title: string; body: string }>;
  webNotifications: CapturedWebNotification[];
  focusCount: number;
}

const createFakeDeps = (
  permission: NotificationPermission | null,
  requestedPermission: NotificationPermission = permission ?? 'default',
): FakeNotificationDeps => {
  const deps: FakeNotificationDeps = {
    fallbackNotifications: [],
    webNotifications: [],
    focusCount: 0,
    webNotificationApi: () => {
      if (permission === null) {
        return null;
      }

      const api: WebNotificationApi = {
        permission,
        requestPermission: async () => requestedPermission,
        create: (title, options) => {
          const notification: CapturedWebNotification = {
            title,
            options,
            onclick: null,
          };
          deps.webNotifications.push(notification);
          return notification;
        },
      };

      return api;
    },
    sendFallbackNotification: async (title, body) => {
      deps.fallbackNotifications.push({ title, body });
    },
    focusWindow: async () => {
      deps.focusCount += 1;
    },
  };

  return deps;
};

const analysisResult: AnalysisResult = {
  meetingId: 'meeting-1',
  reportId: 'report-1',
  analysis: {
    overallScore: 72,
    summary: 'Solid meeting with a few pacing opportunities.',
    observations: [],
    suggestions: [],
  },
  scorecard: {
    overall: {
      score: 81,
      label: 'Strong',
      availableWeight: 1,
      missingDimensions: [],
    },
    dimensions: [],
  },
};

describe('completion notifications', () => {
  it('builds privacy-safe notification payloads from analysis results', () => {
    expect(notificationPayloadFromAnalysis(analysisResult)).toEqual({
      meetingId: 'meeting-1',
      reportId: 'report-1',
      overallScore: 81,
    });
  });

  it('does not send a notification when web permission is denied', async () => {
    const deps = createFakeDeps('denied');
    const openedMeetingIds: MeetingId[] = [];

    const status = await notifyAnalysisComplete(
      notificationPayloadFromAnalysis(analysisResult),
      (meetingId) => {
        openedMeetingIds.push(meetingId);
      },
      deps,
    );

    expect(status).toBe('permission-denied');
    expect(deps.webNotifications).toHaveLength(0);
    expect(deps.fallbackNotifications).toHaveLength(0);
    expect(openedMeetingIds).toHaveLength(0);
  });

  it('sends a clickable WebView notification when permission is granted', async () => {
    const deps = createFakeDeps('granted');
    const openedMeetingIds: MeetingId[] = [];

    const status = await notifyAnalysisComplete(
      notificationPayloadFromAnalysis(analysisResult),
      (meetingId) => {
        openedMeetingIds.push(meetingId);
      },
      deps,
    );

    expect(status).toBe('sent');
    expect(deps.webNotifications).toHaveLength(1);
    expect(deps.webNotifications[0]?.title).toBe('Meeting analysis ready');
    expect(deps.webNotifications[0]?.options).toEqual({
      body: 'Score 81/100. Open the saved report in Resonance.',
      tag: 'report-1',
    });

    deps.webNotifications[0]?.onclick?.(new Event('click'));
    await Promise.resolve();

    expect(openedMeetingIds).toEqual(['meeting-1']);
    expect(deps.focusCount).toBe(1);
  });

  it('requests web permission before sending a clickable notification', async () => {
    const deps = createFakeDeps('default', 'granted');

    const status = await notifyAnalysisComplete(notificationPayloadFromAnalysis(analysisResult), () => {}, deps);

    expect(status).toBe('sent');
    expect(deps.webNotifications).toHaveLength(1);
    expect(deps.fallbackNotifications).toHaveLength(0);
  });

  it('uses the macOS fallback notification when WebView notifications are unavailable', async () => {
    const deps = createFakeDeps(null);

    const status = await notifyAnalysisComplete(notificationPayloadFromAnalysis(analysisResult), () => {}, deps);

    expect(status).toBe('fallback-sent');
    expect(deps.webNotifications).toHaveLength(0);
    expect(deps.fallbackNotifications).toEqual([
      {
        title: 'Meeting analysis ready',
        body: 'Score 81/100. Open the saved report in Resonance.',
      },
    ]);
  });
});
