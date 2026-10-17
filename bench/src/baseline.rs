//! `bench baseline`: appends a run's summary to the tracked `baseline.tsv`.
//! Only runs that met the protocol qualify, and every row carries the stamp.

use crate::stamp::{self, Stamp};
use crate::tsv::{self, Table};
use std::collections::BTreeSet;
use std::path::Path;

pub const BASELINE_HEADER: &[&str] = &[
    "date_utc",
    "milestone",
    "note",
    "run_id",
    "app",
    "app_version",
    "app_tree",
    "workload",
    "phase",
    "metric",
    "unit",
    "n",
    "mean",
    "ci95_low",
    "ci95_high",
    "median",
    "min",
    "max",
    "bench_commit",
    "bench_tree",
    "machine",
    "macos",
    "display",
    "power",
    "zed_version",
];

pub const MIN_TRIALS: usize = 10;

pub fn stamp_from_table(table: &Table) -> Result<Stamp, String> {
    let key = table.column("key")?;
    let value = table.column("value")?;
    let mut stamp = Stamp::default();
    for row in &table.rows {
        let name = row.get(key).ok_or("run.tsv row without a key")?;
        let text = row.get(value).ok_or("run.tsv row without a value")?;
        stamp.push(name, text.clone());
    }
    Ok(stamp)
}

/// Measured trials (not warm-ups) that stayed valid, from `trials.tsv`.
pub fn valid_trials(trials: &Table) -> Result<usize, String> {
    let mut count = 0;
    for row in &trials.rows {
        if trials.get(row, "warmup")? == "0" && trials.get(row, "valid")? == "1" {
            count += 1;
        }
    }
    Ok(count)
}

/// Reasons the run cannot become baseline rows; empty means it qualifies.
pub fn protocol_violations(
    stamp: &Stamp,
    valid: usize,
    summary: &Table,
) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    let power = stamp.require("power")?;
    if !stamp::is_ac_full_power(power) {
        violations.push(format!(
            "power was {power:?}; the protocol needs AC power without Low Power Mode"
        ));
    }
    let tree = stamp.require("bench_tree")?;
    if tree != "clean" {
        violations.push(format!("the bench checkout was {tree}"));
    }
    if stamp.require("app")? == "alpine" {
        let alpine_tree = stamp.get("alpine_tree").unwrap_or("unknown");
        if alpine_tree != "clean" {
            violations.push(format!(
                "the Alpine bundle was built from a {alpine_tree} tree"
            ));
        }
    }
    if valid < MIN_TRIALS {
        violations.push(format!(
            "the run has {valid} valid measured trials; the protocol needs {MIN_TRIALS}"
        ));
    }
    if summary.rows.is_empty() {
        violations.push("the run has no summary rows".to_owned());
    }
    violations.extend(key_summary_violations(stamp, Some(valid), summary)?);
    Ok(violations)
}

const KEY_REQUIRED: &[(&str, &str)] = &[
    ("keys_sent", "count"),
    ("key_semantic_qualified", "bool"),
    ("key_to_screen_p50", "ms"),
    ("key_to_screen_p95", "ms"),
    ("key_to_screen_max", "ms"),
];

fn key_phase(stamp: &Stamp) -> Option<&str> {
    stamp
        .get("workload")
        .filter(|name| matches!(*name, "typing" | "caret"))
}

fn key_value(value: &str, unit: &str) -> bool {
    value.parse::<f64>().is_ok_and(|value| {
        value.is_finite()
            && match unit {
                "bool" => value.total_cmp(&1.0).is_eq(),
                "count" => value > 0.0 && value.fract() == 0.0,
                _ => value >= 0.0,
            }
    })
}

fn key_summary_violations(
    stamp: &Stamp,
    valid: Option<usize>,
    summary: &Table,
) -> Result<Vec<String>, String> {
    let Some(phase) = key_phase(stamp) else {
        return Ok(Vec::new());
    };
    let mut violations = Vec::new();
    for (name, unit) in KEY_REQUIRED {
        let mut matching = Vec::new();
        for row in &summary.rows {
            if summary.get(row, "phase")? == phase && summary.get(row, "metric")? == *name {
                matching.push(row);
            }
        }
        let qualified = if let [row] = matching.as_slice() {
            summary
                .get(row, "n")?
                .parse::<usize>()
                .is_ok_and(|n| n >= MIN_TRIALS && valid.is_none_or(|valid| n == valid))
                && summary.get(row, "unit")? == *unit
                && ["min", "max", "mean", "median"].iter().all(|field| {
                    summary
                        .get(row, field)
                        .is_ok_and(|value| key_value(value, unit))
                })
        } else {
            false
        };
        if !qualified {
            violations.push(format!("{phase} needs complete {name} ({unit}) in every measured trial; hash-only or legacy timing is unqualified"));
        }
    }
    Ok(violations)
}

fn key_trial_violations(
    stamp: &Stamp,
    trials: &Table,
    metrics: &Table,
) -> Result<Vec<String>, String> {
    let Some(phase) = key_phase(stamp) else {
        return Ok(Vec::new());
    };
    let run_id = stamp.require("run_id")?;
    let app = stamp.require("app")?;
    let mut violations = Vec::new();
    let mut seen = BTreeSet::new();
    for trial in &trials.rows {
        if trials.get(trial, "warmup")? != "0" || trials.get(trial, "valid")? != "1" {
            continue;
        }
        let number = trials.get(trial, "trial")?;
        if !seen.insert(number) {
            return Err(format!("duplicate measured trial {number}"));
        }
        for (name, unit) in KEY_REQUIRED {
            let mut values = Vec::new();
            for row in &metrics.rows {
                if metrics.get(row, "trial")? == number
                    && metrics.get(row, "phase")? == phase
                    && metrics.get(row, "metric")? == *name
                    && metrics.get(row, "warmup")? == "0"
                    && metrics.get(row, "valid")? == "1"
                    && metrics.get(row, "run_id")? == run_id
                    && metrics.get(row, "app")? == app
                    && metrics.get(row, "workload")? == phase
                    && metrics.get(row, "unit")? == *unit
                {
                    values.push(metrics.get(row, "value")?);
                }
            }
            if !matches!(values.as_slice(), [value] if key_value(value, unit)) {
                violations.push(format!("trial {number} {phase}: missing, incomplete or unqualified {name}; later trials cannot replace it"));
            }
        }
    }
    Ok(violations)
}

fn app_identity<'a>(stamp: &'a Stamp, app: &str) -> (&'a str, &'a str) {
    match app {
        "alpine" => (
            stamp.get("alpine_version").unwrap_or("unknown"),
            stamp.get("alpine_tree").unwrap_or("unknown"),
        ),
        "zed" => (stamp.get("zed_version").unwrap_or("unknown"), "release"),
        _ => (
            stamp.get("appkit_version").unwrap_or("unknown"),
            stamp.get("bench_tree").unwrap_or("unknown"),
        ),
    }
}

/// Baseline rows for every summary row with at least `MIN_TRIALS` values,
/// and a note for each row skipped because a metric was missing in a trial.
pub fn baseline_rows(
    stamp: &Stamp,
    summary: &Table,
    milestone: &str,
    note: &str,
    date: &str,
) -> Result<(Vec<Vec<String>>, Vec<String>), String> {
    if milestone.trim().is_empty() {
        return Err("--milestone must name a milestone, for example M1".to_owned());
    }
    let required = key_summary_violations(stamp, None, summary)?;
    if !required.is_empty() {
        return Err(required.join("\n"));
    }
    let app = stamp.require("app")?;
    let (app_version, app_tree) = app_identity(stamp, app);
    let mut rows = Vec::with_capacity(summary.rows.len());
    let mut skipped = Vec::new();
    for row in &summary.rows {
        let value = |name: &str| summary.get(row, name).map(str::to_owned);
        let n: usize = summary
            .get(row, "n")?
            .parse()
            .map_err(|_| "summary n is not a count")?;
        if n < MIN_TRIALS {
            skipped.push(format!("{} {} (n={n})", value("phase")?, value("metric")?));
            continue;
        }
        rows.push(vec![
            date.to_owned(),
            milestone.to_owned(),
            note.to_owned(),
            stamp.require("run_id")?.to_owned(),
            app.to_owned(),
            app_version.to_owned(),
            app_tree.to_owned(),
            stamp.require("workload")?.to_owned(),
            value("phase")?,
            value("metric")?,
            value("unit")?,
            value("n")?,
            value("mean")?,
            value("ci95_low")?,
            value("ci95_high")?,
            value("median")?,
            value("min")?,
            value("max")?,
            stamp.require("bench_commit")?.to_owned(),
            stamp.require("bench_tree")?.to_owned(),
            stamp.require("machine")?.to_owned(),
            stamp.require("macos")?.to_owned(),
            stamp.require("display")?.to_owned(),
            stamp.require("power")?.to_owned(),
            stamp.require("zed_version")?.to_owned(),
        ]);
    }
    Ok((rows, skipped))
}

pub fn append(
    run_dir: &Path,
    baseline: &Path,
    milestone: &str,
    note: &str,
) -> Result<usize, String> {
    let stamp = stamp_from_table(&tsv::read(&run_dir.join("run.tsv"))?)?;
    let summary = tsv::read(&run_dir.join("summary.tsv"))?;
    let trials = tsv::read(&run_dir.join("trials.tsv"))?;
    let valid = valid_trials(&trials)?;
    let mut violations = protocol_violations(&stamp, valid, &summary)?;
    if key_phase(&stamp).is_some() {
        violations.extend(key_trial_violations(
            &stamp,
            &trials,
            &tsv::read(&run_dir.join("metrics.tsv"))?,
        )?);
    }
    if !violations.is_empty() {
        return Err(format!(
            "{} does not meet the protocol:\n  {}",
            run_dir.display(),
            violations.join("\n  ")
        ));
    }
    let (date, _) = stamp::utc(stamp::now_seconds());
    let (rows, skipped) = baseline_rows(&stamp, &summary, milestone, note, &date)?;
    if !skipped.is_empty() {
        eprintln!(
            "bench: skipped rows with fewer than {MIN_TRIALS} values: {}",
            skipped.join(", ")
        );
    }
    tsv::append(baseline, BASELINE_HEADER, &rows)?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::{
        BASELINE_HEADER, KEY_REQUIRED, append, baseline_rows, key_trial_violations,
        protocol_violations, stamp_from_table, valid_trials,
    };
    use crate::stamp::Stamp;
    use crate::trial::{METRICS_HEADER, SUMMARY_HEADER, TRIALS_HEADER};
    use crate::tsv::Table;

    fn stamp(app: &str, power: &str, tree: &str) -> Stamp {
        let mut stamp = Stamp::default();
        for (key, value) in [
            ("run_id", "20261002T183005Z-idle-alpine"),
            ("workload", "idle"),
            ("app", app),
            ("bench_commit", "26597c013b6dbaf50a0fb6ee8aa650aadb579acf"),
            ("bench_tree", tree),
            ("machine", "Mac16,1 Apple M4 24GB"),
            ("macos", "26.6.2 (25G83)"),
            ("display", "builtin 1512x982pt 3024x1964px 120Hz max120fps"),
            ("power", power),
            ("zed_version", "1.22.0 (20260930.150108)"),
            ("alpine_version", "0.0.0+26597c013b6d"),
            ("alpine_tree", "clean"),
            ("appkit_version", "src-b3c01b7b20ba swift-6.3.2"),
        ] {
            stamp.push(key, value);
        }
        stamp
    }

    fn summary(n: usize) -> Result<Table, String> {
        let mut table = Table::new(SUMMARY_HEADER);
        table.push(
            [
                "20261002T183005Z-idle-alpine",
                "alpine",
                "idle",
                "idle",
                "cpu_time",
                "ms",
                &n.to_string(),
                "12.5",
                "1.2",
                "11.6",
                "13.4",
                "12.4",
                "10.9",
                "14.8",
            ]
            .iter()
            .map(ToString::to_string)
            .collect(),
        )?;
        Ok(table)
    }

    fn key_tables(phase: &str, n: usize) -> Result<(Stamp, Table, Table, Table), String> {
        let mut stamp = stamp("appkit", "ac lowpower=0 battery=100%", "clean");
        for (name, value) in &mut stamp.entries {
            if name == "workload" {
                *value = phase.to_owned();
            }
        }
        let run_id = stamp.require("run_id")?;
        let mut trials = Table::new(TRIALS_HEADER);
        let mut metrics = Table::new(METRICS_HEADER);
        let mut summaries = Table::new(SUMMARY_HEADER);
        for trial in 1..=n {
            let mut row = vec![
                run_id.to_owned(),
                trial.to_string(),
                "0".to_owned(),
                "1".to_owned(),
            ];
            row.resize(TRIALS_HEADER.len(), String::new());
            trials.push(row)?;
            for (name, unit) in KEY_REQUIRED {
                let value = match *unit {
                    "count" => "100",
                    "bool" => "1",
                    _ => "30",
                };
                metrics.push(
                    [
                        run_id,
                        "appkit",
                        phase,
                        &trial.to_string(),
                        "0",
                        "1",
                        phase,
                        name,
                        value,
                        unit,
                    ]
                    .map(str::to_owned)
                    .to_vec(),
                )?;
                if trial == 1 {
                    summaries.push(
                        [
                            run_id,
                            "appkit",
                            phase,
                            phase,
                            name,
                            unit,
                            &n.to_string(),
                            value,
                            "0",
                            value,
                            value,
                            value,
                            value,
                            value,
                        ]
                        .map(str::to_owned)
                        .to_vec(),
                    )?;
                }
            }
        }
        Ok((stamp, trials, metrics, summaries))
    }

    #[test]
    fn legacy_and_hash_only_typing_and_caret_cannot_be_baselines() -> Result<(), String> {
        for phase in ["typing", "caret"] {
            let (stamp, trials, mut metrics, mut summary) = key_tables(phase, 10)?;
            assert!(protocol_violations(&stamp, 10, &summary)?.is_empty());
            assert!(key_trial_violations(&stamp, &trials, &metrics)?.is_empty());
            summary
                .rows
                .retain(|row| row[4] != "key_semantic_qualified");
            assert!(baseline_rows(&stamp, &summary, "M1", "", "d").is_err());
            metrics
                .rows
                .retain(|row| row[7] != "key_semantic_qualified");
            assert!(
                key_trial_violations(&stamp, &trials, &metrics)?
                    .iter()
                    .any(|v| v.contains("key_semantic_qualified"))
            );
            let (_, _, mut metrics, mut summary) = key_tables(phase, 10)?;
            for row in &mut metrics.rows {
                if row[7] == "key_semantic_qualified" {
                    row[8] = "0".to_owned();
                }
            }
            for row in &mut summary.rows {
                if row[4] == "key_semantic_qualified" {
                    for index in [7, 9, 10, 11, 12, 13] {
                        row[index] = "0".to_owned();
                    }
                }
            }
            assert!(!key_trial_violations(&stamp, &trials, &metrics)?.is_empty());
            assert!(baseline_rows(&stamp, &summary, "M1", "", "d").is_err());
        }
        Ok(())
    }

    #[test]
    fn required_key_rows_cannot_be_skipped_duplicated_or_nonfinite() -> Result<(), String> {
        let (stamp, trials, metrics, summary) = key_tables("typing", 10)?;
        for (column, bad) in [(5, "bytes"), (6, "9"), (7, "NaN"), (12, "-1"), (13, "inf")] {
            let mut table = summary.clone();
            table.rows[2][column] = bad.to_owned();
            assert!(!protocol_violations(&stamp, 10, &table)?.is_empty());
        }
        let mut duplicate = summary.clone();
        duplicate.rows.push(duplicate.rows[2].clone());
        assert!(baseline_rows(&stamp, &duplicate, "M1", "", "d").is_err());
        for (column, bad) in [(8, "NaN"), (9, "bytes"), (5, "0"), (0, "other-run")] {
            let mut table = metrics.clone();
            table.rows[2][column] = bad.to_owned();
            assert!(!key_trial_violations(&stamp, &trials, &table)?.is_empty());
        }
        let mut duplicate = metrics;
        duplicate.rows.push(duplicate.rows[2].clone());
        assert!(!key_trial_violations(&stamp, &trials, &duplicate)?.is_empty());
        Ok(())
    }

    #[test]
    fn later_successes_cannot_replace_an_earlier_incomplete_trial() -> Result<(), String> {
        let (stamp, trials, mut metrics, summary) = key_tables("typing", 11)?;
        assert!(protocol_violations(&stamp, 11, &summary)?.is_empty());
        metrics.rows[1][8] = "0".to_owned();
        let violations = key_trial_violations(&stamp, &trials, &metrics)?;
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("trial 1 typing"));
        let dir = crate::test_dir("unqualified-baseline")?;
        let mut run = Table::new(&["key", "value"]);
        for (name, value) in &stamp.entries {
            run.push(vec![name.clone(), value.clone()])?;
        }
        for (name, table) in [
            ("run.tsv", &run),
            ("trials.tsv", &trials),
            ("metrics.tsv", &metrics),
            ("summary.tsv", &summary),
        ] {
            crate::tsv::write(&dir.join(name), table)?;
        }
        let baseline = dir.join("baseline.tsv");
        let error = append(&dir, &baseline, "M1", "")
            .err()
            .ok_or("accepted incomplete trial")?;
        assert!(error.contains("trial 1 typing"), "{error}");
        assert!(!baseline.exists());
        std::fs::remove_dir_all(dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn zero_input_cannot_qualify_even_with_a_claimed_latency() -> Result<(), String> {
        let (stamp, trials, mut metrics, mut summary) = key_tables("caret", 10)?;
        metrics.rows[0][8] = "0".to_owned();
        summary.rows[0][12] = "0".to_owned();
        assert!(!key_trial_violations(&stamp, &trials, &metrics)?.is_empty());
        assert!(!protocol_violations(&stamp, 10, &summary)?.is_empty());
        Ok(())
    }

    #[test]
    fn complete_semantic_evidence_can_report_late_responses_without_hash_qualification()
    -> Result<(), String> {
        let (stamp, trials, mut metrics, mut summary) = key_tables("typing", 10)?;
        for row in &mut metrics.rows {
            if row[9] == "ms" {
                row[8] = "400".to_owned();
            }
        }
        let flags: Vec<_> = metrics
            .rows
            .iter()
            .filter(|row| row[7] == "key_semantic_qualified")
            .map(|row| {
                let mut flag = row.clone();
                flag[7] = "key_hash_timing_complete".to_owned();
                flag[8] = "0".to_owned();
                flag
            })
            .collect();
        metrics.rows.extend(flags);
        for row in &mut summary.rows {
            if row[5] == "ms" {
                for index in [7, 9, 10, 11, 12, 13] {
                    row[index] = "400".to_owned();
                }
            }
        }
        assert!(protocol_violations(&stamp, 10, &summary)?.is_empty());
        assert!(key_trial_violations(&stamp, &trials, &metrics)?.is_empty());
        assert!(baseline_rows(&stamp, &summary, "M1", "", "d").is_ok());
        Ok(())
    }

    #[test]
    fn a_protocol_run_qualifies() -> Result<(), String> {
        let violations = protocol_violations(
            &stamp("alpine", "ac lowpower=0 battery=100%", "clean"),
            10,
            &summary(10)?,
        )?;
        assert!(violations.is_empty(), "{violations:?}");
        Ok(())
    }

    #[test]
    fn short_battery_or_dirty_runs_are_refused() -> Result<(), String> {
        let violations = protocol_violations(
            &stamp("alpine", "battery lowpower=0 battery=80%", "dirty"),
            1,
            &summary(1)?,
        )?;
        assert_eq!(violations.len(), 3, "{violations:?}");
        assert!(
            violations[2].contains("1 valid measured trials"),
            "{violations:?}"
        );
        let empty = Table::new(SUMMARY_HEADER);
        let stamp = stamp("zed", "ac lowpower=0 battery=100%", "clean");
        assert!(!protocol_violations(&stamp, 10, &empty)?.is_empty());
        Ok(())
    }

    #[test]
    fn valid_trials_count_measured_valid_rows_only() -> Result<(), String> {
        let mut trials = Table::new(TRIALS_HEADER);
        for (warmup, valid) in [("1", "1"), ("0", "1"), ("0", "0"), ("0", "1")] {
            let mut row = vec![
                "r".to_owned(),
                "1".to_owned(),
                warmup.to_owned(),
                valid.to_owned(),
            ];
            row.resize(TRIALS_HEADER.len(), String::new());
            trials.push(row)?;
        }
        assert_eq!(valid_trials(&trials)?, 2);
        Ok(())
    }

    #[test]
    fn rows_missing_values_are_skipped_not_fatal() -> Result<(), String> {
        let mut table = summary(10)?;
        let mut short = table.rows[0].clone();
        short[4] = "footprint_tool_end".to_owned();
        short[6] = "9".to_owned();
        table.push(short)?;
        let stamp = stamp("alpine", "ac lowpower=0 battery=100%", "clean");
        let (rows, skipped) = baseline_rows(&stamp, &table, "M1", "", "d")?;
        assert_eq!(rows.len(), 1);
        assert_eq!(skipped, vec!["idle footprint_tool_end (n=9)".to_owned()]);
        Ok(())
    }

    #[test]
    fn rows_carry_the_stamp_and_the_measured_app_version() -> Result<(), String> {
        let (rows, skipped) = baseline_rows(
            &stamp("zed", "ac lowpower=0 battery=100%", "clean"),
            &summary(10)?,
            "M1",
            "first",
            "2026-10-02T18:30:05Z",
        )?;
        assert!(skipped.is_empty());
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.len(), BASELINE_HEADER.len());
        let column = |name: &str| {
            BASELINE_HEADER
                .iter()
                .position(|field| *field == name)
                .unwrap_or(usize::MAX)
        };
        assert_eq!(row[column("milestone")], "M1");
        assert_eq!(row[column("app_version")], "1.22.0 (20260930.150108)");
        assert_eq!(row[column("app_tree")], "release");
        assert_eq!(row[column("zed_version")], "1.22.0 (20260930.150108)");
        assert_eq!(row[column("macos")], "26.6.2 (25G83)");
        assert_eq!(row[column("mean")], "12.5");
        assert!(baseline_rows(&stamp("zed", "ac", "clean"), &summary(10)?, " ", "", "d").is_err());
        Ok(())
    }

    #[test]
    fn the_tracked_baseline_has_the_current_header() -> Result<(), String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("baseline.tsv");
        let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        assert_eq!(
            text.lines().next(),
            Some(crate::tsv::join(BASELINE_HEADER).as_str())
        );
        Ok(())
    }

    #[test]
    fn run_tables_become_stamps() -> Result<(), String> {
        let mut table = Table::new(&["key", "value"]);
        table.push(vec![
            "power".to_owned(),
            "ac lowpower=0 battery=100%".to_owned(),
        ])?;
        let stamp = stamp_from_table(&table)?;
        assert_eq!(stamp.get("power"), Some("ac lowpower=0 battery=100%"));
        Ok(())
    }
}
