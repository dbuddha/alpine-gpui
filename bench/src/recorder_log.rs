//! Alpine's in-app recorder as a trial sees it: the palette keys that save
//! it, the files it writes under the app's HOME, and the cold first frame
//! its frames file reports.

use crate::tsv::Table;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// `kVK_ANSI_P` and `kVK_Return`.
const KEY_P: &str = "35";
const KEY_RETURN: &str = "36";

/// Lowercase, as bench-input types only a-z, digits, space, comma and period;
/// the palette matches case-insensitively, and no other command contains it.
pub const PALETTE_QUERY: &str = "save performance log";

/// Where Window > Save Performance Log writes, under the app's HOME.
pub fn log_dir(home: &Path) -> PathBuf {
    home.join("Library/Logs/Alpine Editor")
}

/// The bench-input scripts in order: Cmd+Shift+P opens the palette, the query
/// selects the command and Return runs it. Each posts to `pid` only.
pub fn save_steps(pid: &str) -> [Vec<String>; 3] {
    let owned = |args: &[&str]| args.iter().map(ToString::to_string).collect();
    [
        owned(&[
            "chord",
            "--pid",
            pid,
            "--keycode",
            KEY_P,
            "--modifiers",
            "command,shift",
        ]),
        owned(&[
            "type",
            "--pid",
            pid,
            "--text",
            PALETTE_QUERY,
            "--interval-ms",
            "60",
        ]),
        owned(&[
            "keys",
            "--pid",
            pid,
            "--keycode",
            KEY_RETURN,
            "--count",
            "1",
        ]),
    ]
}

/// The frames and samples files, once `perf-<s>-samples.tsv` exists: the
/// recorder writes and closes the frames file before it creates that one.
pub fn saved_files(dir: &Path) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {}: {error}", dir.display())),
    };
    let mut stamps = Vec::new();
    for entry in entries {
        let name = entry
            .map_err(|error| format!("read {}: {error}", dir.display()))?
            .file_name();
        let stamp = name
            .to_str()
            .and_then(|name| name.strip_prefix("perf-"))
            .and_then(|rest| rest.strip_suffix("-samples.tsv"));
        stamps.extend(stamp.map(ToOwned::to_owned));
    }
    match stamps.as_slice() {
        [] => Ok(None),
        [stamp] => {
            let frames = dir.join(format!("perf-{stamp}-frames.tsv"));
            if !frames.is_file() {
                return Err(format!("{} is missing", frames.display()));
            }
            Ok(Some((
                frames,
                dir.join(format!("perf-{stamp}-samples.tsv")),
            )))
        }
        _ => Err(format!(
            "{} holds {} performance logs; a fresh home holds one",
            dir.display(),
            stamps.len()
        )),
    }
}

/// The frames.tsv fields a trial reads; an empty field is absent, never 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecorderFrame {
    pub event: Option<u64>,
    pub build_begin_ns: Option<u64>,
    pub build_end_ns: Option<u64>,
}

/// Parses frames.tsv by column name, so added columns do not move these.
pub fn parse_frames(text: &str) -> Result<Vec<RecorderFrame>, String> {
    let table = Table::parse(text).map_err(|error| format!("frames.tsv: {error}"))?;
    let event = table.column("event")?;
    let begin = table.column("build_begin_ns")?;
    let end = table.column("build_end_ns")?;
    let mut frames = Vec::with_capacity(table.rows.len());
    for (index, row) in table.rows.iter().enumerate() {
        let field = |column: usize| -> Result<Option<u64>, String> {
            match row.get(column).map_or("", String::as_str) {
                "" => Ok(None),
                text => text
                    .parse()
                    .map(Some)
                    .map_err(|_| format!("frames.tsv row {}: {text:?} is not a count", index + 1)),
            }
        };
        frames.push(RecorderFrame {
            event: field(event)?,
            build_begin_ns: field(begin)?,
            build_end_ns: field(end)?,
        });
    }
    Ok(frames)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstBuild {
    /// The startup build, recorded as event 0, took `build_ns`.
    Startup { build_ns: u64 },
    /// Why no row can stand for the cold first frame.
    Refused(String),
}

/// The first row with both build columns must be the startup build, whose
/// `event` is empty; any other is a warm frame and is refused, never used.
pub fn first_build(frames: &[RecorderFrame]) -> FirstBuild {
    let built = frames.iter().enumerate().find_map(|(index, frame)| {
        let begin = frame.build_begin_ns?;
        Some((index, frame.event, begin, frame.build_end_ns?))
    });
    let Some((index, event, begin, end)) = built else {
        return FirstBuild::Refused(
            "no frames.tsv row has build_begin_ns and build_end_ns".to_owned(),
        );
    };
    if let Some(event) = event {
        return FirstBuild::Refused(format!(
            "the first row with build columns (row {}) is event {event}'s warm frame, \
             not the startup build",
            index + 1
        ));
    }
    match end.checked_sub(begin) {
        Some(build_ns) => FirstBuild::Startup { build_ns },
        None => FirstBuild::Refused("the startup build ends before it begins".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FirstBuild, PALETTE_QUERY, RecorderFrame, first_build, log_dir, parse_frames, save_steps,
        saved_files,
    };
    use std::path::Path;

    /// The recorder's header as Alpine writes it.
    const HEADER: &str = "event\trevision\toutcome\tevent_ns\tdispatch_ns\tmutation_ns\
        \tbuild_begin_ns\tlayout_begin_ns\tlayout_end_ns\tatlas_begin_ns\tatlas_end_ns\
        \tbuild_end_ns\thandler_end_ns\tsubmit_begin_ns\tsubmit_end_ns\tgpu_observed_ns\
        \ttarget_ns\ttarget_present_ns\tpresented_ns\trecorded_ns";

    /// A row of 20 fields: `event`, the build columns, and filler elsewhere.
    fn row(event: &str, begin: &str, end: &str) -> String {
        let mut fields = vec![""; 20];
        fields[0] = event;
        fields[1] = "1";
        fields[2] = "presented";
        fields[3] = "100000000005";
        fields[6] = begin;
        fields[11] = end;
        fields[13] = "41000000";
        fields[19] = "52000000";
        fields.join("\t")
    }

    fn table(rows: &[String]) -> String {
        format!("{HEADER}\n{}\n", rows.join("\n"))
    }

    #[test]
    fn rows_parse_by_column_name_with_empty_fields_absent() -> Result<(), String> {
        let text = table(&[
            row("", "120", "41000120"),
            row("3", "", ""),
            row("", "", ""),
        ]);
        assert_eq!(
            parse_frames(&text)?,
            [
                RecorderFrame {
                    event: None,
                    build_begin_ns: Some(120),
                    build_end_ns: Some(41_000_120),
                },
                RecorderFrame {
                    event: Some(3),
                    build_begin_ns: None,
                    build_end_ns: None,
                },
                RecorderFrame {
                    event: None,
                    build_begin_ns: None,
                    build_end_ns: None,
                },
            ]
        );
        assert_eq!(parse_frames(&format!("{HEADER}\n"))?, []);
        Ok(())
    }

    #[test]
    fn malformed_tables_are_errors() {
        for bad in [
            String::new(),
            table(&[row("x", "1", "2")]),
            table(&[row("", "-1", "2")]),
            table(&["\t1\tpresented".to_owned()]),
            "event\trevision\n\t1\n".to_owned(),
        ] {
            assert!(parse_frames(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_startup_row_gives_the_first_frame_build_time() -> Result<(), String> {
        // Accessibility frames carry no event and no build; they are skipped.
        let text = table(&[
            row("", "", ""),
            row("", "250", "38000250"),
            row("4", "10", "900010"),
        ]);
        assert_eq!(
            first_build(&parse_frames(&text)?),
            FirstBuild::Startup {
                build_ns: 38_000_000
            }
        );
        Ok(())
    }

    #[test]
    fn a_warm_or_missing_startup_row_is_refused() -> Result<(), String> {
        let warm = table(&[row("", "", ""), row("2", "10", "900010")]);
        let FirstBuild::Refused(reason) = first_build(&parse_frames(&warm)?) else {
            return Err("an event's frame was accepted".to_owned());
        };
        assert!(
            reason.contains("event 2") && reason.contains("row 2"),
            "{reason}"
        );
        for refused in [
            table(&[row("", "", "")]),
            table(&[row("", "10", ""), row("", "", "20")]),
            table(&[row("", "30", "20")]),
            format!("{HEADER}\n"),
        ] {
            let frames = parse_frames(&refused)?;
            assert!(
                matches!(first_build(&frames), FirstBuild::Refused(_)),
                "{refused}"
            );
        }
        Ok(())
    }

    #[test]
    fn save_steps_target_the_pid_and_pick_the_command() {
        let steps = save_steps("4242");
        for step in &steps {
            let targets: Vec<&[String]> =
                step.windows(2).filter(|pair| pair[0] == "--pid").collect();
            assert_eq!(targets.len(), 1, "{step:?}");
            assert_eq!(targets[0][1], "4242");
        }
        assert_eq!(steps[0][0], "chord");
        assert!(steps[0].windows(2).any(|pair| pair == ["--keycode", "35"]));
        assert!(
            steps[0]
                .windows(2)
                .any(|pair| pair == ["--modifiers", "command,shift"])
        );
        assert!(
            steps[1]
                .windows(2)
                .any(|pair| pair == ["--text", PALETTE_QUERY])
        );
        assert!(steps[2].windows(2).any(|pair| pair == ["--keycode", "36"]));
        // bench-input types only these characters.
        assert!(
            PALETTE_QUERY
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == ' ')
        );
    }

    #[test]
    fn the_samples_file_marks_a_complete_save() -> Result<(), String> {
        let home = crate::test_dir("recorder-log")?;
        let dir = log_dir(&home);
        assert!(dir.ends_with("Library/Logs/Alpine Editor"));
        assert_eq!(saved_files(&dir)?, None);
        let write = |name: &str| {
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            std::fs::write(dir.join(name), HEADER).map_err(|error| error.to_string())
        };
        write("perf-1759500000-frames.tsv")?;
        assert_eq!(saved_files(&dir)?, None, "frames alone may be partial");
        write("perf-1759500000-samples.tsv")?;
        let (frames, samples) = saved_files(&dir)?.ok_or("no saved pair")?;
        assert!(frames.ends_with("perf-1759500000-frames.tsv"));
        assert!(samples.ends_with("perf-1759500000-samples.tsv"));
        write("perf-1759500009-samples.tsv")?;
        assert!(saved_files(&dir).is_err(), "two logs in a fresh home");
        std::fs::remove_dir_all(&home).map_err(|error| error.to_string())?;
        assert!(saved_files(Path::new("/nonexistent/alpine")).is_ok_and(|found| found.is_none()));
        Ok(())
    }
}
