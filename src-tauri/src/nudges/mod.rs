use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    domain::AppError,
    rules::{self, RuleTranscriptSegment},
    transcription::{TranscriptEventSink, TranscriptStreamEvent, TranscriptStreamSummary},
};

pub const LIVE_NUDGE_EVENT: &str = "scribe://live-nudge";
const DEFAULT_THROTTLE_WINDOW_MS: u64 = 30_000;
const DEFAULT_WALL_CLOCK_THROTTLE_MS: u64 = 1_500;
const MAX_RECENT_SEGMENTS: usize = 120;
const FAST_PACE_WPM: f64 = 180.0;
const SLOW_PACE_WPM: f64 = 110.0;
const MIN_PACE_WORDS: u32 = 20;
const LONG_MONOLOGUE_MS: u64 = 45_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NudgeCategory {
    FillerWords,
    Hedging,
    Pace,
    TalkTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NudgeSeverity {
    Info,
    Caution,
    Urgent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveNudgeEvent {
    pub id: String,
    pub meeting_id: String,
    pub category: NudgeCategory,
    pub severity: NudgeSeverity,
    pub message: String,
    pub suggestion: String,
    pub evidence: String,
    pub occurred_at_ms: u64,
}

pub trait NudgeEventSink {
    fn emit_nudge(&mut self, event: LiveNudgeEvent) -> Result<(), AppError>;
}

#[derive(Debug, Clone)]
pub struct LiveNudgePipeline {
    segments: VecDeque<RuleTranscriptSegment>,
    last_emitted_at_ms: BTreeMap<NudgeCategory, u64>,
    last_wall_clock_emit: Option<Instant>,
    throttle_window_ms: u64,
    wall_clock_throttle: Duration,
    max_recent_segments: usize,
}

impl Default for LiveNudgePipeline {
    fn default() -> Self {
        Self::new(DEFAULT_THROTTLE_WINDOW_MS)
    }
}

impl LiveNudgePipeline {
    pub fn new(throttle_window_ms: u64) -> Self {
        Self::new_with_limits(
            throttle_window_ms,
            Duration::from_millis(DEFAULT_WALL_CLOCK_THROTTLE_MS),
            MAX_RECENT_SEGMENTS,
        )
    }

    pub fn new_with_limits(
        throttle_window_ms: u64,
        wall_clock_throttle: Duration,
        max_recent_segments: usize,
    ) -> Self {
        Self {
            segments: VecDeque::new(),
            last_emitted_at_ms: BTreeMap::new(),
            last_wall_clock_emit: None,
            throttle_window_ms,
            wall_clock_throttle,
            max_recent_segments: max_recent_segments.max(1),
        }
    }

    pub fn process_segment(&mut self, event: &TranscriptStreamEvent) -> Vec<LiveNudgeEvent> {
        let segment = RuleTranscriptSegment {
            text: event.segment.text.clone(),
            started_at_ms: event.segment.started_at_ms,
            ended_at_ms: event.segment.ended_at_ms,
        };
        self.segments.push_back(segment.clone());
        while self.segments.len() > self.max_recent_segments {
            self.segments.pop_front();
        }

        [
            self.filler_nudge(event, &segment),
            self.hedging_nudge(event, &segment),
            self.pace_nudge(event),
            self.talk_time_nudge(event),
        ]
        .into_iter()
        .flatten()
        .filter(|nudge| self.should_emit(nudge.category, nudge.occurred_at_ms, Instant::now()))
        .collect()
    }

    fn should_emit(
        &mut self,
        category: NudgeCategory,
        occurred_at_ms: u64,
        emitted_at: Instant,
    ) -> bool {
        if let Some(previous_wall_clock_emit) = self.last_wall_clock_emit {
            if emitted_at.duration_since(previous_wall_clock_emit) < self.wall_clock_throttle {
                return false;
            }
        }

        let Some(previous_at_ms) = self.last_emitted_at_ms.get(&category).copied() else {
            self.last_emitted_at_ms.insert(category, occurred_at_ms);
            self.last_wall_clock_emit = Some(emitted_at);
            return true;
        };

        if occurred_at_ms.saturating_sub(previous_at_ms) < self.throttle_window_ms {
            return false;
        }

        self.last_emitted_at_ms.insert(category, occurred_at_ms);
        self.last_wall_clock_emit = Some(emitted_at);
        true
    }

    fn filler_nudge(
        &self,
        event: &TranscriptStreamEvent,
        segment: &RuleTranscriptSegment,
    ) -> Option<LiveNudgeEvent> {
        let summary = rules::calculate_metrics(std::slice::from_ref(segment));
        if summary.filler_word_count == 0 {
            return None;
        }

        Some(nudge(
            event,
            NudgeCategory::FillerWords,
            NudgeSeverity::Caution,
            "Filler words are clustering.",
            "Pause for one beat before the next thought.",
        ))
    }

    fn hedging_nudge(
        &self,
        event: &TranscriptStreamEvent,
        segment: &RuleTranscriptSegment,
    ) -> Option<LiveNudgeEvent> {
        let summary = rules::calculate_metrics(std::slice::from_ref(segment));
        if summary.hedging_phrase_count == 0 {
            return None;
        }

        Some(nudge(
            event,
            NudgeCategory::Hedging,
            NudgeSeverity::Info,
            "Commitment language softened.",
            "Try stating the recommendation directly.",
        ))
    }

    fn pace_nudge(&self, event: &TranscriptStreamEvent) -> Option<LiveNudgeEvent> {
        let segments = self.segments.iter().cloned().collect::<Vec<_>>();
        let summary = rules::calculate_metrics(&segments);
        if summary.word_count < MIN_PACE_WORDS {
            return None;
        }

        if summary.words_per_minute > FAST_PACE_WPM {
            return Some(nudge(
                event,
                NudgeCategory::Pace,
                NudgeSeverity::Caution,
                "Pace is running fast.",
                "Slow down and leave space for listeners to process.",
            ));
        }

        if summary.words_per_minute < SLOW_PACE_WPM {
            return Some(nudge(
                event,
                NudgeCategory::Pace,
                NudgeSeverity::Info,
                "Pace is drifting slow.",
                "Tighten the next sentence to keep momentum.",
            ));
        }

        None
    }

    fn talk_time_nudge(&self, event: &TranscriptStreamEvent) -> Option<LiveNudgeEvent> {
        let segments = self.segments.iter().cloned().collect::<Vec<_>>();
        let summary = rules::calculate_metrics(&segments);
        if summary.longest_monologue_ms < LONG_MONOLOGUE_MS {
            return None;
        }

        Some(nudge(
            event,
            NudgeCategory::TalkTime,
            NudgeSeverity::Urgent,
            "Long monologue detected.",
            "Invite a response or summarize before continuing.",
        ))
    }
}

pub struct NudgeTranscriptEventSink<T, N> {
    transcript_sink: T,
    nudge_sink: N,
    pipeline: LiveNudgePipeline,
}

impl<T, N> NudgeTranscriptEventSink<T, N> {
    pub fn new(transcript_sink: T, nudge_sink: N, pipeline: LiveNudgePipeline) -> Self {
        Self {
            transcript_sink,
            nudge_sink,
            pipeline,
        }
    }
}

impl<T: TranscriptEventSink, N: NudgeEventSink> TranscriptEventSink
    for NudgeTranscriptEventSink<T, N>
{
    fn emit_segment(&mut self, event: TranscriptStreamEvent) -> Result<(), AppError> {
        self.transcript_sink.emit_segment(event.clone())?;
        // Nudges coach the user's own speaking; remote participants' segments
        // from the system-audio track must not trigger them.
        if !crate::transcription::is_user_segment(event.segment.speaker_label.as_deref()) {
            return Ok(());
        }
        for nudge in self.pipeline.process_segment(&event) {
            if let Err(error) = self.nudge_sink.emit_nudge(nudge) {
                eprintln!(
                    "live_nudge_emit_failed: code={}, message={}",
                    error.code, error.message
                );
            }
        }
        Ok(())
    }

    fn complete(&mut self, summary: TranscriptStreamSummary) -> Result<(), AppError> {
        self.transcript_sink.complete(summary)
    }

    fn dropped_event_count(&self) -> u64 {
        self.transcript_sink.dropped_event_count()
    }
}

fn nudge(
    event: &TranscriptStreamEvent,
    category: NudgeCategory,
    severity: NudgeSeverity,
    message: &str,
    suggestion: &str,
) -> LiveNudgeEvent {
    LiveNudgeEvent {
        id: format!(
            "{}-nudge-{:?}-{}",
            event.meeting_id, category, event.segment.sequence_number
        ),
        meeting_id: event.meeting_id.clone(),
        category,
        severity,
        message: message.to_string(),
        suggestion: suggestion.to_string(),
        evidence: event.segment.text.clone(),
        occurred_at_ms: event.segment.ended_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AcceptingTranscriptSink;

    impl TranscriptEventSink for AcceptingTranscriptSink {
        fn emit_segment(&mut self, _event: TranscriptStreamEvent) -> Result<(), AppError> {
            Ok(())
        }

        fn complete(&mut self, _summary: TranscriptStreamSummary) -> Result<(), AppError> {
            Ok(())
        }
    }

    struct FailingNudgeSink;

    impl NudgeEventSink for FailingNudgeSink {
        fn emit_nudge(&mut self, _event: LiveNudgeEvent) -> Result<(), AppError> {
            Err(AppError {
                code: "test_nudge_emit_failed".to_string(),
                message: "nudge sink failed".to_string(),
                details: None,
            })
        }
    }

    fn stream_event(
        sequence_number: u32,
        text: &str,
        started_at_ms: u64,
        ended_at_ms: u64,
    ) -> TranscriptStreamEvent {
        TranscriptStreamEvent {
            meeting_id: "meeting-nudges".to_string(),
            segment: crate::transcription::TranscriptSegment {
                sequence_number,
                speaker_label: None,
                text: text.to_string(),
                started_at_ms,
                ended_at_ms,
            },
            is_final: true,
        }
    }

    #[test]
    fn emits_filler_and_hedging_nudges_from_segment_text() {
        let mut pipeline = LiveNudgePipeline::new_with_limits(30_000, Duration::ZERO, 120);

        let nudges = pipeline.process_segment(&stream_event(
            1,
            "Um, I think we should pause here.",
            0,
            2_000,
        ));

        assert_eq!(nudges.len(), 2);
        assert_eq!(nudges[0].category, NudgeCategory::FillerWords);
        assert_eq!(nudges[1].category, NudgeCategory::Hedging);
    }

    #[test]
    fn emits_pace_nudge_after_enough_words() {
        let mut pipeline = LiveNudgePipeline::new_with_limits(30_000, Duration::ZERO, 120);
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty one";

        let nudges = pipeline.process_segment(&stream_event(1, text, 0, 5_000));

        assert!(nudges
            .iter()
            .any(|nudge| nudge.category == NudgeCategory::Pace));
    }

    #[test]
    fn emits_talk_time_nudge_for_long_monologue() {
        let mut pipeline = LiveNudgePipeline::new_with_limits(30_000, Duration::ZERO, 120);

        let nudges = pipeline.process_segment(&stream_event(
            1,
            "This is a long uninterrupted explanation.",
            0,
            46_000,
        ));

        assert!(nudges
            .iter()
            .any(|nudge| nudge.category == NudgeCategory::TalkTime));
    }

    #[test]
    fn throttles_duplicate_category_nudges() {
        let mut pipeline = LiveNudgePipeline::new_with_limits(30_000, Duration::ZERO, 120);

        let first = pipeline.process_segment(&stream_event(1, "um first", 0, 1_000));
        let second = pipeline.process_segment(&stream_event(2, "uh second", 2_000, 3_000));
        let third = pipeline.process_segment(&stream_event(3, "like third", 32_000, 33_000));

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
        assert_eq!(third.len(), 1);
    }

    #[test]
    fn bounds_recent_segment_history() {
        let mut pipeline = LiveNudgePipeline::new_with_limits(30_000, Duration::ZERO, 2);

        pipeline.process_segment(&stream_event(1, "one two three four five", 0, 1_000));
        pipeline.process_segment(&stream_event(2, "six seven eight nine ten", 1_000, 2_000));
        pipeline.process_segment(&stream_event(
            3,
            "eleven twelve thirteen fourteen fifteen",
            2_000,
            3_000,
        ));

        assert_eq!(pipeline.segments.len(), 2);
        assert_eq!(pipeline.segments[0].text, "six seven eight nine ten");
    }

    #[test]
    fn throttles_wall_clock_bursts_across_categories() {
        let mut pipeline = LiveNudgePipeline::new_with_limits(30_000, Duration::from_secs(60), 120);

        let nudges = pipeline.process_segment(&stream_event(
            1,
            "Um, I think we should pause here.",
            0,
            2_000,
        ));

        assert_eq!(nudges.len(), 1);
        assert_eq!(nudges[0].category, NudgeCategory::FillerWords);
    }

    #[test]
    fn nudge_sink_failure_does_not_fail_transcript_segment_emit() {
        let mut sink = NudgeTranscriptEventSink::new(
            AcceptingTranscriptSink,
            FailingNudgeSink,
            LiveNudgePipeline::new_with_limits(30_000, Duration::ZERO, 120),
        );

        let result = sink.emit_segment(stream_event(1, "um this should still succeed", 0, 1_000));

        assert!(result.is_ok());
    }
}
