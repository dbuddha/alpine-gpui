//! Running tools and finding or stopping processes, with `ps` and `kill`
//! only. A process is stopped only if it descends from a launched app or its
//! command line names the trial's disposable home.

use crate::apps::LaunchSpec;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Runs a tool to completion and returns stdout, or stderr on failure.
pub fn output(program: &Path, args: &[&str]) -> Result<String, String> {
    let result = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{}: {error}", program.display()))?;
    if !result.status.success() {
        return Err(format!(
            "{} {} failed ({}): {}",
            program.display(),
            args.join(" "),
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    String::from_utf8(result.stdout).map_err(|error| format!("{}: {error}", program.display()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsRow {
    pub pid: i32,
    pub ppid: i32,
    pub command: String,
}

pub fn parse_ps(text: &str) -> Vec<PsRow> {
    text.lines()
        .filter_map(|line| {
            let (process, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let rest = rest.trim_start();
            let (parent, command) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            Some(PsRow {
                pid: process.parse().ok()?,
                ppid: parent.parse().ok()?,
                command: command.trim().to_owned(),
            })
        })
        .collect()
}

pub fn ps() -> Result<Vec<PsRow>, String> {
    output(Path::new("/bin/ps"), &["-A", "-o", "pid=,ppid=,command="]).map(|text| parse_ps(&text))
}

/// Every descendant of `root` in a `ps` snapshot, nearest first.
pub fn descendants(rows: &[PsRow], root: i32) -> Vec<i32> {
    let mut children: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for row in rows {
        children.entry(row.ppid).or_default().push(row.pid);
    }
    let mut found = Vec::new();
    let mut seen = BTreeSet::from([root]);
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        for child in children.get(&parent).into_iter().flatten() {
            if seen.insert(*child) {
                found.push(*child);
                queue.insert(0, *child);
            }
        }
    }
    found
}

/// Orphans (reparented to launchd) whose command line names `marker`, a
/// trial's disposable home: a helper the app started that outlived it.
pub fn marked_orphans(rows: &[PsRow], marker: &str) -> Vec<i32> {
    if marker.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .filter(|row| row.ppid == 1 && row.command.contains(marker))
        .map(|row| row.pid)
        .collect()
}

/// Processes running `binary`, the exact path or the path then arguments.
pub fn running(rows: &[PsRow], binary: &str) -> Vec<PsRow> {
    rows.iter()
        .filter(|row| {
            row.command == binary
                || row
                    .command
                    .strip_prefix(binary)
                    .is_some_and(|rest| rest.starts_with(' '))
        })
        .cloned()
        .collect()
}

/// Zed main processes that are not ours: no `--crash-handler`, and the
/// command line does not name `ours` (a bench home).
pub fn foreign_zed(rows: &[PsRow], zed_binary: &str, ours: &str) -> Vec<PsRow> {
    rows.iter()
        .filter(|row| {
            row.command.starts_with(zed_binary)
                && !row.command.contains("--crash-handler")
                && (ours.is_empty() || !row.command.contains(ours))
        })
        .cloned()
        .collect()
}

pub fn signal(name: &str, pids: &[i32]) {
    if pids.is_empty() {
        return;
    }
    let mut args = vec![format!("-{name}")];
    args.extend(pids.iter().map(ToString::to_string));
    // Exit status is ignored: some processes may already be gone.
    let _ = Command::new("/bin/kill")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// An app process the bench started, stopped with everything it started
/// when the trial ends or the bench unwinds.
pub struct LaunchedApp {
    child: Child,
    pid: i32,
    marker: String,
    stopped: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StopReport {
    pub signalled: usize,
    pub killed: Vec<i32>,
}

impl LaunchedApp {
    /// `marker` is the trial's disposable home: any process whose command
    /// line names it belongs to this trial.
    pub fn spawn(
        spec: &LaunchSpec,
        cwd: &Path,
        logs: (File, File),
        marker: &str,
    ) -> Result<Self, String> {
        let child = Command::new(&spec.program)
            .args(&spec.args)
            .env_clear()
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(logs.0)
            .stderr(logs.1)
            .spawn()
            .map_err(|error| format!("launch {}: {error}", spec.program.display()))?;
        let pid = i32::try_from(child.id()).map_err(|_| "child pid does not fit i32")?;
        Ok(Self {
            child,
            pid,
            marker: marker.to_owned(),
            stopped: false,
        })
    }

    pub fn pid(&self) -> i32 {
        self.pid
    }

    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    /// TERM to the app, its descendants and marked orphans, then KILL after
    /// `grace`. A pid is signalled only while its command line is unchanged,
    /// so a recycled pid is never hit.
    pub fn stop(&mut self, known: &[i32], grace: Duration) -> StopReport {
        let rows = ps().unwrap_or_default();
        let mut targets: BTreeSet<i32> = known.iter().copied().collect();
        targets.extend(descendants(&rows, self.pid));
        targets.extend(marked_orphans(&rows, &self.marker));
        targets.remove(&self.pid);
        let commands: BTreeMap<i32, String> = rows
            .into_iter()
            .filter(|row| targets.contains(&row.pid))
            .map(|row| (row.pid, row.command))
            .collect();
        let others: Vec<i32> = commands.keys().copied().collect();
        if !self.exited() {
            signal("TERM", &[self.pid]);
        }
        signal("TERM", &others);
        let deadline = Instant::now() + grace;
        let mut killed = Vec::new();
        loop {
            let root_done = self.exited();
            let alive: Vec<i32> = ps()
                .unwrap_or_default()
                .into_iter()
                .filter(|row| commands.get(&row.pid) == Some(&row.command))
                .map(|row| row.pid)
                .collect();
            if root_done && alive.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                if !root_done {
                    let _ = self.child.kill();
                    killed.push(self.pid);
                }
                signal("KILL", &alive);
                killed.extend(alive);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.wait();
        self.stopped = true;
        StopReport {
            signalled: others.len() + 1,
            killed,
        }
    }
}

impl Drop for LaunchedApp {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.stop(&[], Duration::from_secs(3));
        }
    }
}

/// `phys_footprint: 884952 B` from `footprint --noCategories -f bytes`.
pub fn parse_footprint(text: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("phys_footprint:")?;
        rest.trim().strip_suffix(" B")?.trim().parse().ok()
    })
}

/// Apple's `footprint` as a cross-check of the sampler at trial end.
pub fn footprint_bytes(pid: i32) -> Result<u64, String> {
    let text = output(
        Path::new("/usr/bin/footprint"),
        &["--noCategories", "-f", "bytes", "-p", &pid.to_string()],
    )?;
    parse_footprint(&text).ok_or_else(|| "footprint printed no phys_footprint".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        PsRow, descendants, foreign_zed, marked_orphans, parse_footprint, parse_ps, running,
    };

    #[test]
    fn footprint_output_yields_phys_footprint() {
        let text = "======\nsleep [5313]: 64-bit    Footprint: 868568 B (16384 bytes per page)\n======\n\nAuxiliary data:\n    phys_footprint: 884952 B\n    phys_footprint_peak: 901336 B\n";
        assert_eq!(parse_footprint(text), Some(884_952));
        assert_eq!(parse_footprint("nothing here"), None);
    }

    const SNAPSHOT: &str = "    1     0 /sbin/launchd
  500     1 /Applications/Zed.app/Contents/MacOS/zed
  501   500 /Applications/Zed.app/Contents/MacOS/zed --crash-handler /Users/me/Library/Caches/Zed/x
  600     1 /Users/me/Applications/Alpine Editor.app/Contents/MacOS/alpine-editor /tmp/a.txt
  601   600 /Users/me/.rustup/toolchains/x/bin/rust-analyzer
  602   601 /Users/me/.rustup/toolchains/x/bin/rust-analyzer-proc-macro-srv
  603   600 cargo check --workspace
  700     1 /Applications/Zed.app/Contents/MacOS/zed --user-data-dir /tmp/alpine-bench/r1-01/home/zed
";

    #[test]
    fn ps_rows_keep_commands_with_spaces() {
        let rows = parse_ps(SNAPSHOT);
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[3].pid, 600);
        assert_eq!(rows[3].ppid, 1);
        assert!(rows[3].command.ends_with("alpine-editor /tmp/a.txt"));
        assert!(parse_ps("garbage\n  12\n").is_empty());
    }

    #[test]
    fn descendants_cover_grandchildren_only_below_the_root() {
        let rows = parse_ps(SNAPSHOT);
        assert_eq!(descendants(&rows, 600), vec![601, 603, 602]);
        assert_eq!(descendants(&rows, 602), Vec::<i32>::new());
        assert!(!descendants(&rows, 1).is_empty());
    }

    #[test]
    fn only_orphans_carrying_the_marker_are_targets() {
        let mut rows = parse_ps(SNAPSHOT);
        assert_eq!(
            marked_orphans(&rows, "/tmp/alpine-bench/r1-01/home"),
            vec![700]
        );
        assert!(marked_orphans(&rows, "").is_empty());
        // A process that merely mentions the home but has a live parent,
        // such as the owner's shell running `ls` on it, is never a target.
        rows.push(PsRow {
            pid: 800,
            ppid: 501,
            command: "ls /tmp/alpine-bench/r1-01/home".to_owned(),
        });
        assert_eq!(
            marked_orphans(&rows, "/tmp/alpine-bench/r1-01/home"),
            vec![700]
        );
    }

    #[test]
    fn running_matches_the_binary_path_exactly() {
        let rows = parse_ps(SNAPSHOT);
        let alpine = "/Users/me/Applications/Alpine Editor.app/Contents/MacOS/alpine-editor";
        let found: Vec<i32> = running(&rows, alpine).iter().map(|row| row.pid).collect();
        assert_eq!(found, vec![600]);
        let zed = "/Applications/Zed.app/Contents/MacOS/zed";
        assert_eq!(running(&rows, zed).len(), 3);
        assert!(running(&rows, "/Applications/Zed.app/Contents/MacOS/ze").is_empty());
    }

    #[test]
    fn foreign_zed_skips_crash_handlers_and_bench_instances() {
        let rows = parse_ps(SNAPSHOT);
        let zed = "/Applications/Zed.app/Contents/MacOS/zed";
        let foreign = foreign_zed(&rows, zed, "/tmp/alpine-bench");
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0].pid, 500);
        assert_eq!(foreign_zed(&rows, zed, "").len(), 2);
    }
}
