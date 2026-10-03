//! `bench zed-isolation`: launches Zed exactly as a trial does and checks
//! that nothing under the owner's real Zed state changed. Trials use the
//! same snapshot and diff on every app's real state.

use crate::apps::{self, App, LaunchInputs};
use crate::fixtures;
use crate::helpers::Helpers;
use crate::paths::{self, BenchPaths};
use crate::procs::{self, LaunchedApp};
use crate::stamp;
use crate::workload::Fixture;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: char,
    pub size: u64,
    pub modified_ns: u128,
}

pub type Snapshot = BTreeMap<PathBuf, Entry>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(PathBuf),
    Removed(PathBuf),
    Modified(PathBuf),
}

fn entry(path: &Path) -> Option<Entry> {
    let meta = fs::symlink_metadata(path).ok()?;
    let kind = if meta.file_type().is_symlink() {
        'l'
    } else if meta.is_dir() {
        'd'
    } else {
        'f'
    };
    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos());
    Some(Entry {
        kind,
        size: if kind == 'd' { 0 } else { meta.len() },
        modified_ns,
    })
}

/// Size, type and mtime of every path under the roots; symlinks are not
/// followed and a missing root contributes nothing.
pub fn snapshot(roots: &[PathBuf]) -> Snapshot {
    let mut found = Snapshot::new();
    let mut pending: Vec<PathBuf> = roots.to_vec();
    while let Some(path) = pending.pop() {
        let Some(item) = entry(&path) else { continue };
        if item.kind == 'd'
            && let Ok(children) = fs::read_dir(&path)
        {
            pending.extend(children.filter_map(Result::ok).map(|child| child.path()));
        }
        found.insert(path, item);
    }
    found
}

/// Directory mtimes are ignored: they move whenever a child changes, and
/// the child itself is reported.
pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<Change> {
    let mut changes = Vec::new();
    for (path, old) in before {
        match after.get(path) {
            None => changes.push(Change::Removed(path.clone())),
            Some(new) if old.kind == 'd' && new.kind == 'd' => {}
            Some(new) if new != old => changes.push(Change::Modified(path.clone())),
            Some(_) => {}
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            changes.push(Change::Added(path.clone()));
        }
    }
    changes
}

/// Paths from `lsof -Fn` output that fall under any of the roots.
pub fn open_under(lsof: &str, roots: &[PathBuf]) -> Vec<String> {
    lsof.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .filter(|path| roots.iter().any(|root| Path::new(path).starts_with(root)))
        .map(ToOwned::to_owned)
        .collect()
}

pub fn describe(change: &Change) -> String {
    match change {
        Change::Added(path) => format!("added {}", path.display()),
        Change::Removed(path) => format!("removed {}", path.display()),
        Change::Modified(path) => format!("modified {}", path.display()),
    }
}

pub struct Probe {
    pub clean: bool,
    pub report: Vec<String>,
}

/// Prepares the probe's disposable home exactly as a trial would and
/// launches Zed, with the idle fixture only when asked.
fn launch(
    paths: &BenchPaths,
    probe_root: &Path,
    open_fixture: bool,
) -> Result<(LaunchedApp, PathBuf), String> {
    let home = probe_root.join("home");
    let tmp = probe_root.join("tmp");
    for dir in [&home, &tmp] {
        fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    }
    let documents = if open_fixture {
        vec![fixtures::ensure(&paths.fixtures_dir(), Fixture::Idle)?.path]
    } else {
        Vec::new()
    };
    let identity = paths::identity();
    let spec = apps::launch_spec(&LaunchInputs {
        app: App::Zed,
        home: &home,
        tmpdir: &tmp,
        documents: &documents,
        identity: &identity,
        rust_analyzer: None,
        alpine_bundle: &paths.alpine_bundle,
        zed_app: &paths.zed_app,
        reference_binary: Path::new("/nonexistent"),
    })?;
    for (path, content) in &spec.seed_files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        fs::write(path, content).map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    let log = |name: &str| {
        File::create(probe_root.join(name)).map_err(|error| format!("{name}: {error}"))
    };
    let logs = (log("stdout.log")?, log("stderr.log")?);
    let app = LaunchedApp::spawn(&spec, &home, logs, &probe_root.to_string_lossy())?;
    Ok((app, home))
}

/// Lets the probe run, then records how it ended and which files it held.
fn observe(
    paths: &BenchPaths,
    app: &mut LaunchedApp,
    seconds: u64,
    roots: &[PathBuf],
    report: &mut Vec<String>,
) -> Vec<String> {
    let pid = app.pid();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) && !app.exited() {
        thread::sleep(Duration::from_millis(100));
    }
    if app.exited() {
        let lifetime = started.elapsed().as_millis();
        report.push(format!("probe zed (pid {pid}) exited after {lifetime} ms"));
        return Vec::new();
    }
    let lsof = procs::output(
        Path::new("/usr/sbin/lsof"),
        &["-n", "-P", "-Fn", "-p", &pid.to_string()],
    )
    .unwrap_or_default();
    let window = Helpers::locate(&paths.helpers_dir)
        .and_then(|helpers| helpers.window_list())
        .map(|list| list.windows.iter().any(|window| window.pid == pid));
    report.push(format!(
        "probe zed (pid {pid}) still running after {seconds} s; window on screen: {}",
        window.map_or_else(
            |error| format!("unknown ({error})"),
            |seen| seen.to_string()
        )
    ));
    open_under(&lsof, roots)
}

fn home_summary(home: &Path) -> String {
    let created = snapshot(&[home.to_path_buf()]);
    let mut examples: Vec<String> = created
        .keys()
        .filter(|path| path.components().count() <= home.components().count() + 4)
        .map(|path| {
            path.strip_prefix(home)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .filter(|path| !path.is_empty())
        .collect();
    examples.truncate(12);
    format!(
        "disposable home {} gained {} entries: {}",
        home.display(),
        created.len().saturating_sub(1),
        examples.join(", ")
    )
}

/// A windowless probe needs another Zed holding the single-instance port; a
/// windowed one needs no other Zed, whose own writes would mix into the diff.
pub fn probe_precondition(
    other_zed: bool,
    allow_window: bool,
    open_fixture: bool,
) -> Result<(), String> {
    if open_fixture && !allow_window {
        return Err(
            "--open-fixture needs --allow-window: without a window Zed exits before opening it"
                .to_owned(),
        );
    }
    match (allow_window, other_zed) {
        (false, false) => Err(
            "no other Zed is running, so this probe would open a Zed window; rerun with \
             --allow-window while the owner is present"
                .to_owned(),
        ),
        (true, true) => Err(
            "another Zed is running: a windowed probe needs the owner's Zed quit, or its \
             writes would mix into the comparison and the probe would stop at Zed's \
             single-instance check"
                .to_owned(),
        ),
        _ => Ok(()),
    }
}

pub fn run(
    paths: &BenchPaths,
    seconds: u64,
    allow_window: bool,
    open_fixture: bool,
) -> Result<Probe, String> {
    let zed_binary = paths.zed_app.join("Contents/MacOS/zed");
    let zed_text = zed_binary.to_string_lossy().into_owned();
    let home_root_text = paths.home_root.to_string_lossy().into_owned();
    let foreign = procs::foreign_zed(&procs::ps()?, &zed_text, &home_root_text);
    probe_precondition(!foreign.is_empty(), allow_window, open_fixture)?;
    let roots = apps::real_state_paths(App::Zed, &paths.real_home, &paths.zed_app);
    let before = snapshot(&roots);
    let version_before = stamp::zed_version(&paths.zed_app);
    let (_, tag) = stamp::utc(stamp::now_seconds());
    let probe_root = paths.home_root.join(format!("{tag}-zed-isolation"));
    let mut report = vec![format!("zed: {version_before} at {}", zed_binary.display())];
    let (mut app, home) = launch(paths, &probe_root, open_fixture)?;
    let leaked_open = observe(paths, &mut app, seconds, &roots, &mut report);
    let stopped = app.stop(&[], Duration::from_secs(5));
    if !stopped.killed.is_empty() {
        report.push(format!(
            "KILLed {:?} after the TERM grace period",
            stopped.killed
        ));
    }
    thread::sleep(Duration::from_secs(1));
    let changes = diff(&before, &snapshot(&roots));
    let version_after = stamp::zed_version(&paths.zed_app);
    let stdout = fs::read_to_string(probe_root.join("stdout.log")).unwrap_or_default();
    if stdout.contains("zed is already running") {
        report.push(
            "probe zed printed \"zed is already running\": the owner's Zed holds the \
             single-instance port, so the probe stopped before opening a window"
                .to_owned(),
        );
    }
    report.push(home_summary(&home));
    report.push(format!(
        "open files under real Zed paths: {}",
        if leaked_open.is_empty() {
            "none".to_owned()
        } else {
            leaked_open.join(", ")
        }
    ));
    report.push(format!(
        "Zed.app before {version_before}, after {version_after}"
    ));
    if changes.is_empty() {
        report.push(format!(
            "real Zed paths unchanged ({} entries compared)",
            before.len()
        ));
    } else {
        let owner = if foreign.is_empty() {
            "no other Zed was running".to_owned()
        } else {
            let pids: Vec<String> = foreign.iter().map(|row| row.pid.to_string()).collect();
            format!(
                "the owner's Zed (pid {}) was running and may have made them",
                pids.join(", ")
            )
        };
        report.push(format!("real Zed paths changed ({owner}):"));
        report.extend(
            changes
                .iter()
                .map(|change| format!("  {}", describe(change))),
        );
    }
    let clean = changes.is_empty() && leaked_open.is_empty() && version_before == version_after;
    report.push(format!("probe files kept at {}", probe_root.display()));
    Ok(Probe { clean, report })
}

#[cfg(test)]
mod tests {
    use super::{Change, Entry, Snapshot, diff, open_under, probe_precondition, snapshot};
    use std::path::{Path, PathBuf};

    fn file(size: u64, modified_ns: u128) -> Entry {
        Entry {
            kind: 'f',
            size,
            modified_ns,
        }
    }

    #[test]
    fn diff_reports_files_but_not_directory_mtimes() {
        let mut before = Snapshot::new();
        before.insert(
            PathBuf::from("/z"),
            Entry {
                kind: 'd',
                size: 0,
                modified_ns: 1,
            },
        );
        before.insert(PathBuf::from("/z/a"), file(1, 1));
        before.insert(PathBuf::from("/z/b"), file(1, 1));
        let mut after = before.clone();
        after.insert(
            PathBuf::from("/z"),
            Entry {
                kind: 'd',
                size: 0,
                modified_ns: 9,
            },
        );
        after.insert(PathBuf::from("/z/a"), file(2, 5));
        after.remove(Path::new("/z/b"));
        after.insert(PathBuf::from("/z/c"), file(1, 1));
        assert_eq!(
            diff(&before, &after),
            vec![
                Change::Modified(PathBuf::from("/z/a")),
                Change::Removed(PathBuf::from("/z/b")),
                Change::Added(PathBuf::from("/z/c")),
            ]
        );
        assert!(diff(&before, &before).is_empty());
    }

    #[test]
    fn snapshots_walk_directories_and_skip_missing_roots() -> Result<(), String> {
        let dir = crate::test_dir("snapshot")?;
        std::fs::create_dir_all(dir.join("nested")).map_err(|error| error.to_string())?;
        std::fs::write(dir.join("nested/file"), "x").map_err(|error| error.to_string())?;
        let found = snapshot(&[dir.clone(), dir.join("absent")]);
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.get(&dir.join("nested/file")).map(|entry| entry.size),
            Some(1)
        );
        std::fs::remove_dir_all(&dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn lsof_paths_are_matched_by_component_prefix() {
        let roots = vec![PathBuf::from("/Users/me/Library/Application Support/Zed")];
        let lsof = "p123\nfcwd\nn/tmp/alpine-bench/x/home\nn/Users/me/Library/Application Support/Zed/db/0-stable/db.sqlite\nn/Users/me/Library/Application Support/Zedfoo\n";
        assert_eq!(
            open_under(lsof, &roots),
            vec!["/Users/me/Library/Application Support/Zed/db/0-stable/db.sqlite".to_owned()]
        );
    }

    #[test]
    fn probes_need_the_right_zed_state_and_flags() {
        // Windowless: another Zed must hold the single-instance port.
        assert!(probe_precondition(true, false, false).is_ok());
        assert!(probe_precondition(false, false, false).is_err());
        // Windowed: no other Zed may run, with or without a document.
        assert!(probe_precondition(false, true, false).is_ok());
        assert!(probe_precondition(false, true, true).is_ok());
        assert!(probe_precondition(true, true, false).is_err());
        assert!(probe_precondition(true, true, true).is_err());
        // A document only makes sense with a window.
        assert!(probe_precondition(true, false, true).is_err());
        assert!(probe_precondition(false, false, true).is_err());
    }
}
