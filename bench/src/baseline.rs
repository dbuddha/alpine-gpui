//! `bench baseline`: appends a run's summary to the tracked `baseline.tsv`.
//! Only runs that met the protocol qualify, and every row carries the stamp.

use crate::stamp::{self, Stamp};
use crate::tsv::{self, Table};
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

/// Reasons the run cannot become a baseline row; empty means it qualifies.
pub fn protocol_violations(stamp: &Stamp, summary: &Table) -> Result<Vec<String>, String> {
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
    if summary.rows.is_empty() {
        violations.push("the run has no summary rows".to_owned());
    }
    let mut short = 0;
    for row in &summary.rows {
        let n: usize = summary
            .get(row, "n")?
            .parse()
            .map_err(|_| "summary n is not a count")?;
        if n < MIN_TRIALS {
            short += 1;
        }
    }
    if short > 0 {
        violations.push(format!(
            "{short} summary rows have fewer than {MIN_TRIALS} valid trials"
        ));
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

pub fn baseline_rows(
    stamp: &Stamp,
    summary: &Table,
    milestone: &str,
    note: &str,
    date: &str,
) -> Result<Vec<Vec<String>>, String> {
    if milestone.trim().is_empty() {
        return Err("--milestone must name a milestone, for example M1".to_owned());
    }
    let app = stamp.require("app")?;
    let (app_version, app_tree) = app_identity(stamp, app);
    let mut rows = Vec::with_capacity(summary.rows.len());
    for row in &summary.rows {
        let value = |name: &str| summary.get(row, name).map(str::to_owned);
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
    Ok(rows)
}

pub fn append(
    run_dir: &Path,
    baseline: &Path,
    milestone: &str,
    note: &str,
) -> Result<usize, String> {
    let stamp = stamp_from_table(&tsv::read(&run_dir.join("run.tsv"))?)?;
    let summary = tsv::read(&run_dir.join("summary.tsv"))?;
    let violations = protocol_violations(&stamp, &summary)?;
    if !violations.is_empty() {
        return Err(format!(
            "{} does not meet the protocol:\n  {}",
            run_dir.display(),
            violations.join("\n  ")
        ));
    }
    let (date, _) = stamp::utc(stamp::now_seconds());
    let rows = baseline_rows(&stamp, &summary, milestone, note, &date)?;
    tsv::append(baseline, BASELINE_HEADER, &rows)?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::{BASELINE_HEADER, baseline_rows, protocol_violations, stamp_from_table};
    use crate::stamp::Stamp;
    use crate::trial::SUMMARY_HEADER;
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

    #[test]
    fn a_protocol_run_qualifies() -> Result<(), String> {
        let violations = protocol_violations(
            &stamp("alpine", "ac lowpower=0 battery=100%", "clean"),
            &summary(10)?,
        )?;
        assert!(violations.is_empty(), "{violations:?}");
        Ok(())
    }

    #[test]
    fn short_battery_or_dirty_runs_are_refused() -> Result<(), String> {
        let violations = protocol_violations(
            &stamp("alpine", "battery lowpower=0 battery=80%", "dirty"),
            &summary(1)?,
        )?;
        assert_eq!(violations.len(), 3, "{violations:?}");
        let empty = Table::new(SUMMARY_HEADER);
        assert!(
            !protocol_violations(&stamp("zed", "ac lowpower=0 battery=100%", "clean"), &empty)?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn rows_carry_the_stamp_and_the_measured_app_version() -> Result<(), String> {
        let rows = baseline_rows(
            &stamp("zed", "ac lowpower=0 battery=100%", "clean"),
            &summary(10)?,
            "M1",
            "first",
            "2026-10-02T18:30:05Z",
        )?;
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
