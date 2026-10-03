//! Order statistics within a trial and Student t confidence intervals across
//! trials. Percentiles use the nearest-rank rule, so every reported value was
//! actually observed.

/// Lossy only above 2^53, far beyond any byte, tick or sample count here.
#[allow(
    clippy::cast_precision_loss,
    reason = "counts, bytes and nanoseconds stay below 2^53"
)]
pub fn to_f64(value: u64) -> f64 {
    value as f64
}

#[allow(clippy::cast_precision_loss, reason = "sample counts stay below 2^53")]
fn count(value: usize) -> f64 {
    value as f64
}

/// Nearest-rank percentile: the smallest value with at least `quantile` of
/// the samples at or below it. `None` for no samples or a NaN.
pub fn nearest_rank(values: &[f64], quantile: f64) -> Option<f64> {
    if values.is_empty() || !(0.0..=1.0).contains(&quantile) || values.iter().any(|v| v.is_nan()) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = rank_for(quantile, sorted.len());
    sorted.get(rank - 1).copied()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the product is clamped into 1..=len before the cast matters"
)]
fn rank_for(quantile: f64, len: usize) -> usize {
    let raw = (quantile * count(len)).ceil();
    (raw as usize).clamp(1, len)
}

pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / count(values.len()))
}

/// Sample standard deviation (n - 1 denominator); needs two values.
pub fn stddev(values: &[f64]) -> Option<f64> {
    let average = mean(values)?;
    if values.len() < 2 {
        return None;
    }
    let squares: f64 = values.iter().map(|value| (value - average).powi(2)).sum();
    Some((squares / count(values.len() - 1)).sqrt())
}

pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() || values.iter().any(|v| v.is_nan()) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted.get(middle).copied()
    } else {
        Some((sorted.get(middle - 1)? + sorted.get(middle)?) / 2.0)
    }
}

/// Two-sided 95% critical values of Student's t. Degrees of freedom between
/// table rows take the smaller row's value, which widens the interval.
pub fn t_critical_95(degrees_of_freedom: usize) -> Option<f64> {
    const TABLE: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    match degrees_of_freedom {
        0 => None,
        1..=30 => TABLE.get(degrees_of_freedom - 1).copied(),
        31..=39 => Some(2.042),
        40..=59 => Some(2.021),
        60..=119 => Some(2.000),
        _ => Some(1.980),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub mean: f64,
    pub stddev: Option<f64>,
    pub ci95: Option<(f64, f64)>,
    pub median: f64,
    pub min: f64,
    pub max: f64,
}

/// Summarizes one metric across trials: the 95% interval is for the mean of
/// the per-trial values and needs at least two trials.
pub fn summarize(values: &[f64]) -> Option<Summary> {
    let average = mean(values)?;
    let middle = median(values)?;
    let deviation = stddev(values);
    let ci95 = deviation.and_then(|sd| {
        let critical = t_critical_95(values.len() - 1)?;
        let half = critical * sd / count(values.len()).sqrt();
        Some((average - half, average + half))
    });
    Some(Summary {
        n: values.len(),
        mean: average,
        stddev: deviation,
        ci95,
        median: middle,
        min: values.iter().copied().fold(f64::INFINITY, f64::min),
        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    })
}

/// Formats a value with enough digits for milliseconds and bytes alike.
pub fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_value, mean, median, nearest_rank, stddev, summarize, t_critical_95};

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    #[test]
    fn nearest_rank_returns_observed_values() {
        let values: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(nearest_rank(&values, 0.5), Some(10.0));
        assert_eq!(nearest_rank(&values, 0.95), Some(19.0));
        assert_eq!(nearest_rank(&values, 1.0), Some(20.0));
        assert_eq!(nearest_rank(&values, 0.0), Some(1.0));
        assert_eq!(nearest_rank(&[7.0], 0.95), Some(7.0));
        assert_eq!(nearest_rank(&[3.0, 1.0, 2.0], 0.5), Some(2.0));
    }

    #[test]
    fn nearest_rank_rejects_empty_nan_and_bad_quantiles() {
        assert_eq!(nearest_rank(&[], 0.5), None);
        assert_eq!(nearest_rank(&[1.0, f64::NAN], 0.5), None);
        assert_eq!(nearest_rank(&[1.0], 1.5), None);
    }

    #[test]
    fn moments_match_hand_computed_values() -> Result<(), String> {
        let values = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert!(close(mean(&values).ok_or("mean")?, 5.0));
        assert!(close(stddev(&values).ok_or("sd")?, (32.0_f64 / 7.0).sqrt()));
        assert!(close(median(&values).ok_or("median")?, 4.5));
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(stddev(&[1.0]), None);
        Ok(())
    }

    #[test]
    fn t_table_covers_small_and_large_samples() {
        assert_eq!(t_critical_95(0), None);
        assert_eq!(t_critical_95(1), Some(12.706));
        assert_eq!(t_critical_95(9), Some(2.262));
        assert_eq!(t_critical_95(30), Some(2.042));
        assert_eq!(t_critical_95(35), Some(2.042));
        assert_eq!(t_critical_95(500), Some(1.980));
    }

    #[test]
    fn ten_trials_give_a_t_interval_around_the_mean() -> Result<(), String> {
        let values: Vec<f64> = (1..=10).map(f64::from).collect();
        let summary = summarize(&values).ok_or("summary")?;
        let sd = stddev(&values).ok_or("sd")?;
        let half = 2.262 * sd / 10.0_f64.sqrt();
        let (low, high) = summary.ci95.ok_or("ci")?;
        assert_eq!(summary.n, 10);
        assert!(close(summary.mean, 5.5));
        assert!(close(low, 5.5 - half) && close(high, 5.5 + half));
        assert!(close(summary.median, 5.5));
        assert!(close(summary.min, 1.0) && close(summary.max, 10.0));
        Ok(())
    }

    #[test]
    fn one_trial_has_no_interval() -> Result<(), String> {
        let summary = summarize(&[42.0]).ok_or("summary")?;
        assert_eq!(summary.ci95, None);
        assert_eq!(summary.stddev, None);
        assert!(close(summary.mean, 42.0));
        assert_eq!(summarize(&[]), None);
        Ok(())
    }

    #[test]
    fn values_format_without_noise() {
        assert_eq!(format_value(1_048_576.0), "1048576");
        assert_eq!(format_value(8.333_333), "8.333");
    }
}
