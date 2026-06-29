use serde::Serialize;

const FILLER_WORDS: &[&str] = &["um", "uh", "like"];
const HEDGING_PHRASES: &[&[&str]] = &[
    &["i", "think"],
    &["kind", "of"],
    &["sort", "of"],
    &["maybe"],
    &["probably"],
];

/// Transcript segment input for the deterministic rule engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleTranscriptSegment {
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

/// Pure deterministic metrics computed from transcript text and segment offsets.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSummary {
    pub filler_word_count: u32,
    pub filler_word_rate: f64,
    pub hedging_phrase_count: u32,
    pub word_count: u32,
    pub duration_ms: u64,
    pub words_per_minute: f64,
    pub user_talk_time_ms: u64,
    pub longest_monologue_ms: u64,
}

/// A metric value ready to map into persistence.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleMetric {
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
}

/// Calculates deterministic transcript metrics without I/O.
///
/// This MVP intentionally uses the Strategy pattern in simple functions: tokenization,
/// phrase counting, and timing are independently testable strategies behind one small
/// public interface. Phrase matching is O(tokens * phrases * phrase_length), which is
/// acceptable for local meeting transcripts and the small static dictionaries here.
pub fn calculate_metrics(segments: &[RuleTranscriptSegment]) -> MetricsSummary {
    let tokens = segments
        .iter()
        .flat_map(|segment| tokenize(&segment.text))
        .collect::<Vec<_>>();
    let word_count = u32::try_from(tokens.len()).unwrap_or(u32::MAX);
    let filler_word_count = count_filler_words(&tokens);
    let hedging_phrase_count = count_phrases(&tokens, HEDGING_PHRASES);
    let duration_ms = transcript_duration_ms(segments);
    let (user_talk_time_ms, longest_monologue_ms) = talk_time_metrics(segments);

    MetricsSummary {
        filler_word_count,
        filler_word_rate: ratio(f64::from(filler_word_count), f64::from(word_count)),
        hedging_phrase_count,
        word_count,
        duration_ms,
        words_per_minute: words_per_minute(word_count, duration_ms),
        user_talk_time_ms,
        longest_monologue_ms,
    }
}

/// Returns the stable metric rows represented by a summary.
pub fn metrics_for_persistence(summary: &MetricsSummary) -> Vec<RuleMetric> {
    vec![
        metric("word_count", f64::from(summary.word_count), Some("count")),
        metric("duration_ms", summary.duration_ms as f64, Some("ms")),
        metric("words_per_minute", summary.words_per_minute, Some("wpm")),
        metric(
            "filler_word_count",
            f64::from(summary.filler_word_count),
            Some("count"),
        ),
        metric("filler_word_rate", summary.filler_word_rate, Some("ratio")),
        metric(
            "hedging_phrase_count",
            f64::from(summary.hedging_phrase_count),
            Some("count"),
        ),
        metric(
            "user_talk_time_ms",
            summary.user_talk_time_ms as f64,
            Some("ms"),
        ),
        metric(
            "longest_monologue_ms",
            summary.longest_monologue_ms as f64,
            Some("ms"),
        ),
    ]
}

pub fn deterministic_metric_names() -> Vec<String> {
    metrics_for_persistence(&MetricsSummary {
        filler_word_count: 0,
        filler_word_rate: 0.0,
        hedging_phrase_count: 0,
        word_count: 0,
        duration_ms: 0,
        words_per_minute: 0.0,
        user_talk_time_ms: 0,
        longest_monologue_ms: 0,
    })
    .into_iter()
    .map(|metric| metric.name)
    .collect()
}

fn metric(name: &str, value: f64, unit: Option<&str>) -> RuleMetric {
    RuleMetric {
        name: name.to_string(),
        value,
        unit: unit.map(str::to_string),
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for character in text.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
            continue;
        }

        if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn count_filler_words(tokens: &[String]) -> u32 {
    let count = tokens
        .iter()
        .filter(|token| FILLER_WORDS.contains(&token.as_str()))
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn count_phrases(tokens: &[String], phrases: &[&[&str]]) -> u32 {
    let count = (0..tokens.len())
        .map(|start_index| {
            phrases
                .iter()
                .filter(|phrase| phrase_matches(tokens, start_index, phrase))
                .count()
        })
        .sum::<usize>();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn phrase_matches(tokens: &[String], start_index: usize, phrase: &[&str]) -> bool {
    if start_index + phrase.len() > tokens.len() {
        return false;
    }

    phrase
        .iter()
        .enumerate()
        .all(|(offset, phrase_token)| tokens[start_index + offset] == *phrase_token)
}

fn transcript_duration_ms(segments: &[RuleTranscriptSegment]) -> u64 {
    let Some(first_start) = segments.iter().map(|segment| segment.started_at_ms).min() else {
        return 0;
    };
    let last_end = segments
        .iter()
        .map(|segment| segment.ended_at_ms)
        .max()
        .unwrap_or(first_start);
    last_end.saturating_sub(first_start)
}

fn talk_time_metrics(segments: &[RuleTranscriptSegment]) -> (u64, u64) {
    let mut intervals = segments
        .iter()
        .filter(|segment| segment.ended_at_ms > segment.started_at_ms)
        .map(|segment| (segment.started_at_ms, segment.ended_at_ms))
        .collect::<Vec<_>>();
    intervals.sort_unstable_by_key(|(start, end)| (*start, *end));

    let mut merged_total = 0_u64;
    let mut longest = 0_u64;
    let mut current: Option<(u64, u64)> = None;

    for (start, end) in intervals {
        current = match current {
            None => Some((start, end)),
            Some((current_start, current_end)) if start <= current_end => {
                Some((current_start, current_end.max(end)))
            }
            Some((current_start, current_end)) => {
                let duration = current_end.saturating_sub(current_start);
                merged_total = merged_total.saturating_add(duration);
                longest = longest.max(duration);
                Some((start, end))
            }
        };
    }

    if let Some((start, end)) = current {
        let duration = end.saturating_sub(start);
        merged_total = merged_total.saturating_add(duration);
        longest = longest.max(duration);
    }

    (merged_total, longest)
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        return 0.0;
    }

    numerator / denominator
}

fn words_per_minute(word_count: u32, duration_ms: u64) -> f64 {
    if duration_ms == 0 {
        return 0.0;
    }

    f64::from(word_count) / (duration_ms as f64 / 60_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_phrases_across_case_punctuation_and_repeated_words() {
        let segments = vec![RuleTranscriptSegment {
            text: "Um... UM, unlike like; I THINK! kind-of kind of.".to_string(),
            started_at_ms: 0,
            ended_at_ms: 10_000,
        }];

        let summary = calculate_metrics(&segments);

        assert_eq!(summary.word_count, 10);
        assert_eq!(summary.filler_word_count, 3);
        assert_eq!(summary.hedging_phrase_count, 3);
    }

    #[test]
    fn golden_transcript_produces_expected_metrics() {
        let segments = vec![
            RuleTranscriptSegment {
                text: "Um, I think we should, kind of, start now.".to_string(),
                started_at_ms: 0,
                ended_at_ms: 30_000,
            },
            RuleTranscriptSegment {
                text: "Like, maybe we move fast. Probably, uh, yes.".to_string(),
                started_at_ms: 30_000,
                ended_at_ms: 60_000,
            },
        ];

        let summary = calculate_metrics(&segments);

        assert_eq!(summary.word_count, 17);
        assert_eq!(summary.filler_word_count, 3);
        assert_eq!(summary.hedging_phrase_count, 4);
        assert_eq!(summary.duration_ms, 60_000);
        assert_eq!(summary.user_talk_time_ms, 60_000);
        assert_eq!(summary.longest_monologue_ms, 60_000);
        assert_f64_eq(summary.words_per_minute, 17.0);
        assert_f64_eq(summary.filler_word_rate, 3.0 / 17.0);
    }

    #[test]
    fn empty_transcript_returns_zero_metrics() {
        let summary = calculate_metrics(&[]);

        assert_eq!(summary.word_count, 0);
        assert_eq!(summary.filler_word_count, 0);
        assert_eq!(summary.hedging_phrase_count, 0);
        assert_eq!(summary.duration_ms, 0);
        assert_eq!(summary.user_talk_time_ms, 0);
        assert_eq!(summary.longest_monologue_ms, 0);
        assert_f64_eq(summary.words_per_minute, 0.0);
        assert_f64_eq(summary.filler_word_rate, 0.0);
    }

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < f64::EPSILON,
            "expected {expected}, got {actual}"
        );
    }
}
