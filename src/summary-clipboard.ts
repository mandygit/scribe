import type { MeetingActionItem, MeetingSummary } from './contracts';

const escapeHtml = (value: string): string => value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

const actionItemLine = (item: MeetingActionItem): string => {
  const owner = item.owner ? `${item.owner}: ` : '';
  const due = item.due ? ` (Due: ${item.due})` : '';
  return `${owner}${item.task}${due}`;
};

const htmlSection = (title: string, items: string[]): string =>
  items.length === 0
    ? ''
    : `<p><strong>${title}</strong></p><ul>${items.map((item) => `<li>${escapeHtml(item)}</li>`).join('')}</ul>`;

const textSection = (title: string, items: string[]): string =>
  items.length === 0 ? '' : `${title}:\n${items.map((item) => `- ${item}`).join('\n')}\n`;

export interface SummaryClipboardPayload {
  html: string;
  text: string;
}

// Mirrors NotesView's sections exactly (executive summary, decisions, open
// questions, action items) as plain bullet points and bold section labels —
// deliberately no tables/borders, since those paste badly into Slack/Teams.
export const buildSummaryClipboardPayload = (summary: MeetingSummary): SummaryClipboardPayload => {
  const actionItemLines = summary.actionItems.map(actionItemLine);

  const html = [
    summary.executiveSummary ? `<p>${escapeHtml(summary.executiveSummary)}</p>` : '',
    htmlSection('Decisions', summary.decisions),
    htmlSection('Open questions', summary.openQuestions),
    htmlSection('Action items', actionItemLines),
  ]
    .filter(Boolean)
    .join('');

  const text = [
    summary.executiveSummary,
    textSection('Decisions', summary.decisions),
    textSection('Open questions', summary.openQuestions),
    textSection('Action items', actionItemLines),
  ]
    .filter(Boolean)
    .join('\n');

  return { html, text };
};

// Writes both text/html and text/plain so pasting into Slack/Teams (both
// accept rich HTML paste) keeps bold labels and bullet points; falls back to
// plain text if the webview doesn't support multi-format clipboard writes.
export const copySummaryToClipboard = async (summary: MeetingSummary): Promise<void> => {
  const { html, text } = buildSummaryClipboardPayload(summary);
  if (typeof ClipboardItem !== 'undefined' && navigator.clipboard?.write) {
    await navigator.clipboard.write([
      new ClipboardItem({
        'text/html': new Blob([html], { type: 'text/html' }),
        'text/plain': new Blob([text], { type: 'text/plain' }),
      }),
    ]);
    return;
  }
  await navigator.clipboard.writeText(text);
};
