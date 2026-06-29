use serde::Serialize;

use crate::{
    domain::Score,
    rules::{MetricsSummary, RuleMetric},
};

const FILLER_WEIGHT: f64 = 0.20;
const PACE_WEIGHT: f64 = 0.20;
const CLARITY_WEIGHT: f64 = 0.20;
const TALK_TIME_WEIGHT: f64 = 0.15;
const ANALYSIS_WEIGHT: f64 = 0.25;

/// Availability-aware deterministic inputs for scoring.
///
/// Every metric is optional so callers can score partial reports without
/// coupling this pure module to SQLite, network analyzers, or transcript I/O.
/// Percent/rate inputs use ratios, so `0.05` means 5% filler words.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScoringInput {
    pub filler_word_rate: Option<f64>,
    pub words_per_minute: Option<f64>,
    pub hedging_phrase_count: Option<u32>,
    pub word_count: Option<u32>,
    pub duration_ms: Option<u64>,
    pub user_talk_time_ms: Option<u64>,
    pub analyzer_overall_score: Option<Score>,
}

impl ScoringInput {
    /// Builds scoring input from the deterministic metrics emitted by
    /// `rules::metrics_for_persistence`, optionally including analyzer quality.
    ///
    /// Unknown metric names are ignored, making this stable as new rule metrics
    /// are added. Invalid numeric values are treated as unavailable, except
    /// positive infinity in rate-like metrics, which is scored as maximally poor.
    pub fn from_rule_metrics(
        metrics: &[RuleMetric],
        analyzer_overall_score: Option<Score>,
    ) -> Self {
        let mut input = Self {
            analyzer_overall_score,
            ..Self::default()
        };

        for metric in metrics {
            match metric.name.as_str() {
                "filler_word_rate" => input.filler_word_rate = Some(metric.value),
                "words_per_minute" => input.words_per_minute = Some(metric.value),
                "hedging_phrase_count" => input.hedging_phrase_count = f64_to_u32(metric.value),
                "word_count" => input.word_count = f64_to_u32(metric.value),
                "duration_ms" => input.duration_ms = f64_to_u64(metric.value),
                "user_talk_time_ms" => input.user_talk_time_ms = f64_to_u64(metric.value),
                _ => {}
            }
        }

        input
    }
}

impl From<&MetricsSummary> for ScoringInput {
    fn from(summary: &MetricsSummary) -> Self {
        Self {
            filler_word_rate: Some(summary.filler_word_rate),
            words_per_minute: Some(summary.words_per_minute),
            hedging_phrase_count: Some(summary.hedging_phrase_count),
            word_count: Some(summary.word_count),
            duration_ms: Some(summary.duration_ms),
            user_talk_time_ms: Some(summary.user_talk_time_ms),
            analyzer_overall_score: None,
        }
    }
}

/// One score dimension with explicit availability.
///
/// `score` is always bounded to 0-100 when present. When unavailable, callers
/// can show `unavailable_reason` rather than guessing why a dimension is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreDimension {
    pub score: Option<Score>,
    pub unavailable_reason: Option<String>,
}

impl ScoreDimension {
    /// Returns true when this dimension has a bounded score.
    pub fn is_available(&self) -> bool {
        self.score.is_some()
    }

    /// Returns true when this dimension could not be scored from available inputs.
    pub fn is_unavailable(&self) -> bool {
        self.score.is_none()
    }

    fn available(value: f64) -> Self {
        Self {
            score: Some(score_from_f64(value)),
            unavailable_reason: None,
        }
    }

    fn unavailable(reason: &str) -> Self {
        Self {
            score: None,
            unavailable_reason: Some(reason.to_string()),
        }
    }
}

/// Deterministic scorecard for meeting coaching.
///
/// Formulas:
/// - Filler uses filler word rate: 0% => 100, 5% => 70, 10% => 35, then
///   linearly decays toward 0.
/// - Pace is ideal from 120-170 WPM; each WPM outside the band subtracts one
///   point, bounded at 0.
/// - Clarity uses hedging phrases per 100 words: 0 => 100, 5 => 70, 10 => 40,
///   then decays toward 0 by 20 hedges per 100 words.
/// - Talk-time is availability-aware and gentle for a single-speaker mic:
///   `70 + active_talk_coverage * 30`, with coverage capped to 0-100%.
/// - Analysis is the analyzer-provided overall score when available.
/// - Overall is a weighted average of available dimensions only, renormalized
///   with weights: filler .20, pace .20, clarity .20, talk-time .15, analysis .25.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scorecard {
    pub filler: ScoreDimension,
    pub pace: ScoreDimension,
    pub clarity: ScoreDimension,
    pub talk_time: ScoreDimension,
    pub analysis: ScoreDimension,
    pub overall: ScoreDimension,
}

/// Calculates a pure, deterministic, bounded scorecard from available metrics.
pub fn calculate_scorecard(input: &ScoringInput) -> Scorecard {
    let filler = score_filler(input.filler_word_rate);
    let pace = score_pace(input.words_per_minute);
    let clarity = score_clarity(input.hedging_phrase_count, input.word_count);
    let talk_time = score_talk_time(input.duration_ms, input.user_talk_time_ms);
    let analysis = input
        .analyzer_overall_score
        .map(|score| ScoreDimension {
            score: Some(score),
            unavailable_reason: None,
        })
        .unwrap_or_else(|| ScoreDimension::unavailable("analyzer score unavailable"));
    let overall = score_overall(&[
        (&filler, FILLER_WEIGHT),
        (&pace, PACE_WEIGHT),
        (&clarity, CLARITY_WEIGHT),
        (&talk_time, TALK_TIME_WEIGHT),
        (&analysis, ANALYSIS_WEIGHT),
    ]);

    Scorecard {
        filler,
        pace,
        clarity,
        talk_time,
        analysis,
        overall,
    }
}

fn score_filler(filler_word_rate: Option<f64>) -> ScoreDimension {
    let Some(rate) = non_negative_or_infinite(filler_word_rate) else {
        return ScoreDimension::unavailable("filler word rate unavailable");
    };

    if rate <= 0.05 {
        return ScoreDimension::available(100.0 - (rate / 0.05 * 30.0));
    }

    if rate <= 0.10 {
        return ScoreDimension::available(70.0 - ((rate - 0.05) / 0.05 * 35.0));
    }

    ScoreDimension::available(35.0 - ((rate - 0.10) / 0.10 * 35.0))
}

fn score_pace(words_per_minute: Option<f64>) -> ScoreDimension {
    let Some(wpm) = finite_non_negative(words_per_minute) else {
        return ScoreDimension::unavailable("words per minute unavailable");
    };

    if (120.0..=170.0).contains(&wpm) {
        return ScoreDimension::available(100.0);
    }

    if wpm < 120.0 {
        return ScoreDimension::available(100.0 - (120.0 - wpm));
    }

    ScoreDimension::available(100.0 - (wpm - 170.0))
}

fn score_clarity(hedging_phrase_count: Option<u32>, word_count: Option<u32>) -> ScoreDimension {
    let (Some(hedges), Some(words)) = (hedging_phrase_count, word_count) else {
        return ScoreDimension::unavailable("hedging phrase count or word count unavailable");
    };
    if words == 0 {
        return ScoreDimension::unavailable("word count unavailable");
    }

    let hedges_per_100_words = f64::from(hedges) / f64::from(words) * 100.0;
    if hedges_per_100_words <= 5.0 {
        return ScoreDimension::available(100.0 - (hedges_per_100_words / 5.0 * 30.0));
    }

    if hedges_per_100_words <= 10.0 {
        return ScoreDimension::available(70.0 - ((hedges_per_100_words - 5.0) / 5.0 * 30.0));
    }

    ScoreDimension::available(40.0 - ((hedges_per_100_words - 10.0) / 10.0 * 40.0))
}

fn score_talk_time(duration_ms: Option<u64>, user_talk_time_ms: Option<u64>) -> ScoreDimension {
    let (Some(duration), Some(talk_time)) = (duration_ms, user_talk_time_ms) else {
        return ScoreDimension::unavailable("duration or talk time unavailable");
    };
    if duration == 0 {
        return ScoreDimension::unavailable("duration unavailable");
    }

    let coverage = (talk_time as f64 / duration as f64).clamp(0.0, 1.0);
    ScoreDimension::available(70.0 + (coverage * 30.0))
}

fn score_overall(dimensions: &[(&ScoreDimension, f64)]) -> ScoreDimension {
    let (weighted_sum, weight_sum) = dimensions.iter().fold(
        (0.0, 0.0),
        |(score_total, weight_total), (dimension, weight)| match dimension.score {
            Some(score) => (
                score_total + (f64::from(score.value()) * *weight),
                weight_total + *weight,
            ),
            None => (score_total, weight_total),
        },
    );

    if weight_sum == 0.0 {
        return ScoreDimension::unavailable("no score dimensions available");
    }

    ScoreDimension::available(weighted_sum / weight_sum)
}

fn score_from_f64(value: f64) -> Score {
    let bounded = if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else if value.is_sign_positive() {
        100.0
    } else {
        0.0
    };

    Score::new(bounded.round() as u8).expect("score is clamped to 0-100")
}

fn finite_non_negative(value: Option<f64>) -> Option<f64> {
    value.filter(|number| number.is_finite() && *number >= 0.0)
}

fn non_negative_or_infinite(value: Option<f64>) -> Option<f64> {
    match value {
        Some(number) if number.is_finite() && number >= 0.0 => Some(number),
        Some(number) if number.is_infinite() && number.is_sign_positive() => Some(f64::MAX),
        _ => None,
    }
}

fn f64_to_u32(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return None;
    }

    Some(value.round() as u32)
}

fn f64_to_u64(value: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 || value >= u64::MAX as f64 {
        return None;
    }

    Some(value.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Score;
    use crate::rules::{metrics_for_persistence, MetricsSummary};

    #[test]
    fn ideal_metrics_produce_high_bounded_scores() {
        let scorecard = calculate_scorecard(&ScoringInput {
            filler_word_rate: Some(0.0),
            words_per_minute: Some(145.0),
            hedging_phrase_count: Some(0),
            word_count: Some(200),
            duration_ms: Some(90_000),
            user_talk_time_ms: Some(72_000),
            analyzer_overall_score: Some(Score::new(92).expect("score is valid")),
        });

        assert!(scorecard.filler.score.expect("filler available").value() >= 95);
        assert!(scorecard.pace.score.expect("pace available").value() >= 95);
        assert!(scorecard.clarity.score.expect("clarity available").value() >= 95);
        assert!(
            scorecard
                .talk_time
                .score
                .expect("talk time available")
                .value()
                >= 90
        );
        assert_eq!(
            scorecard
                .analysis
                .score
                .expect("analysis available")
                .value(),
            92
        );
        let overall = scorecard.overall.score.expect("overall available").value();
        assert!((90..=100).contains(&overall));
    }

    #[test]
    fn high_filler_and_hedging_reduce_relevant_dimensions() {
        let scorecard = calculate_scorecard(&ScoringInput {
            filler_word_rate: Some(0.12),
            words_per_minute: Some(145.0),
            hedging_phrase_count: Some(12),
            word_count: Some(100),
            duration_ms: Some(60_000),
            user_talk_time_ms: Some(50_000),
            analyzer_overall_score: None,
        });

        assert!(scorecard.filler.score.expect("filler available").value() <= 35);
        assert!(scorecard.clarity.score.expect("clarity available").value() <= 40);
        assert!(scorecard.pace.score.expect("pace available").value() >= 95);
    }

    #[test]
    fn pace_too_slow_or_too_fast_reduces_pace_score() {
        let slow = calculate_scorecard(&ScoringInput {
            words_per_minute: Some(70.0),
            ..ScoringInput::default()
        });
        let fast = calculate_scorecard(&ScoringInput {
            words_per_minute: Some(230.0),
            ..ScoringInput::default()
        });

        assert!(slow.pace.score.expect("slow pace available").value() < 70);
        assert!(fast.pace.score.expect("fast pace available").value() < 70);
    }

    #[test]
    fn missing_metrics_are_unavailable_but_overall_uses_available_dimensions() {
        let scorecard = calculate_scorecard(&ScoringInput {
            filler_word_rate: Some(0.0),
            words_per_minute: Some(145.0),
            ..ScoringInput::default()
        });

        assert!(scorecard.filler.is_available());
        assert!(scorecard.pace.is_available());
        assert!(scorecard.clarity.is_unavailable());
        assert!(scorecard.talk_time.is_unavailable());
        assert!(scorecard.analysis.is_unavailable());
        assert!(scorecard.overall.is_available());
        assert!(scorecard.overall.score.expect("overall available").value() >= 95);
    }

    #[test]
    fn all_missing_metrics_return_unavailable_overall() {
        let scorecard = calculate_scorecard(&ScoringInput::default());

        assert!(scorecard.filler.is_unavailable());
        assert!(scorecard.pace.is_unavailable());
        assert!(scorecard.clarity.is_unavailable());
        assert!(scorecard.talk_time.is_unavailable());
        assert!(scorecard.analysis.is_unavailable());
        assert!(scorecard.overall.is_unavailable());
        assert_eq!(
            scorecard.overall.unavailable_reason.as_deref(),
            Some("no score dimensions available")
        );
    }

    #[test]
    fn analyzer_score_is_included_when_provided_and_omitted_when_absent() {
        let with_analyzer = calculate_scorecard(&ScoringInput {
            analyzer_overall_score: Some(Score::new(64).expect("score is valid")),
            ..ScoringInput::default()
        });
        let without_analyzer = calculate_scorecard(&ScoringInput::default());

        assert_eq!(
            with_analyzer
                .analysis
                .score
                .expect("analysis available")
                .value(),
            64
        );
        assert!(with_analyzer.overall.is_available());
        assert!(without_analyzer.analysis.is_unavailable());
    }

    #[test]
    fn scores_are_bounded_for_extreme_values() {
        let scorecard = calculate_scorecard(&ScoringInput {
            filler_word_rate: Some(f64::INFINITY),
            words_per_minute: Some(10_000.0),
            hedging_phrase_count: Some(u32::MAX),
            word_count: Some(1),
            duration_ms: Some(1),
            user_talk_time_ms: Some(u64::MAX),
            analyzer_overall_score: Some(Score::new(100).expect("score is valid")),
        });

        for dimension in [
            &scorecard.filler,
            &scorecard.pace,
            &scorecard.clarity,
            &scorecard.talk_time,
            &scorecard.analysis,
            &scorecard.overall,
        ] {
            if let Some(score) = dimension.score {
                assert!(score.value() <= 100);
            }
        }
    }

    #[test]
    fn scorecard_can_be_built_from_persisted_rule_metrics() {
        let summary = MetricsSummary {
            filler_word_count: 0,
            filler_word_rate: 0.0,
            hedging_phrase_count: 0,
            word_count: 180,
            duration_ms: 75_000,
            words_per_minute: 144.0,
            user_talk_time_ms: 60_000,
            longest_monologue_ms: 20_000,
        };
        let metrics = metrics_for_persistence(&summary);

        let scorecard = calculate_scorecard(&ScoringInput::from_rule_metrics(
            &metrics,
            Some(Score::new(80).expect("score is valid")),
        ));

        assert!(scorecard.overall.is_available());
        assert_eq!(
            scorecard
                .analysis
                .score
                .expect("analysis available")
                .value(),
            80
        );
        assert!(scorecard.filler.is_available());
    }

    #[test]
    fn persisted_duration_at_float_u64_boundary_is_unavailable() {
        let input = ScoringInput::from_rule_metrics(
            &[RuleMetric {
                name: "duration_ms".to_string(),
                value: u64::MAX as f64,
                unit: Some("ms".to_string()),
            }],
            None,
        );

        assert_eq!(input.duration_ms, None);
        let scorecard = calculate_scorecard(&input);
        assert!(scorecard.talk_time.is_unavailable());
    }
}
