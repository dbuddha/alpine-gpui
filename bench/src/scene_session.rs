//! Approved-session-only native controls. Results never replace a physical gate.

use crate::{analysis, gate, helpers, physical_evidence, sharing_indicator::Proof};
use std::fmt::Write as _;
use std::{
    fs,
    io::{Read, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const CAP: usize = 2 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(20);
type Result<T> = std::result::Result<T, String>;
type Buffer = Arc<Mutex<(Vec<u8>, bool)>>;

fn collector(
    mut reader: UnixStream,
    cap: usize,
    stopped: Arc<AtomicBool>,
) -> Result<(Buffer, thread::JoinHandle<()>)> {
    reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|e| e.to_string())?;
    let data = Arc::new(Mutex::new((Vec::new(), false)));
    let copy = Arc::clone(&data);
    let handle = thread::spawn(move || {
        let mut chunk = [0; 4096];
        let mut drain = None;
        loop {
            if stopped.load(Ordering::Acquire) {
                let deadline =
                    drain.get_or_insert_with(|| Instant::now() + Duration::from_millis(100));
                if Instant::now() >= *deadline {
                    if let Ok(mut data) = copy.lock() {
                        data.1 = true;
                    }
                    return;
                }
            }
            let read = reader.read(&mut chunk);
            if read.as_ref().is_err_and(|e| {
                matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                )
            }) {
                continue;
            }
            let Ok(mut data) = copy.lock() else {
                return;
            };
            match read {
                Ok(0) => return,
                Ok(n) if n <= cap.saturating_sub(data.0.len()) => {
                    data.0.extend_from_slice(&chunk[..n]);
                }
                _ => {
                    data.1 = true;
                    return;
                }
            }
        }
    });
    Ok((data, handle))
}

fn kill_group(pid: u32, already_exited: bool) -> Result<()> {
    let (error, write) = UnixStream::pair().map_err(|e| e.to_string())?;
    error
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|e| e.to_string())?;
    let mut signal = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(std::os::fd::OwnedFd::from(write)))
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        if let Some(status) = signal.try_wait().map_err(|e| e.to_string())? {
            if status.success() {
                return Ok(());
            }
            let mut message = String::new();
            error
                .take(1024)
                .read_to_string(&mut message)
                .map_err(|e| e.to_string())?;
            if already_exited && message.contains("No such process") {
                return Ok(());
            }
            return Err(format!(
                "owned group {pid} cleanup failed: {status}: {message}"
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
    let _ = signal.kill();
    let _ = signal.wait();
    Err("owned group signal helper timed out".into())
}

struct Owned {
    child: Child,
    out: Buffer,
    err: Buffer,
    readers: Vec<thread::JoinHandle<()>>,
    evidence: Option<PathBuf>,
    stopped: Arc<AtomicBool>,
    group_closed: bool,
}
impl Owned {
    fn spawn(
        path: &Path,
        args: &[&str],
        cap: usize,
        evidence: Option<PathBuf>,
        home: Option<&Path>,
    ) -> Result<Self> {
        let (read_out, write_out) = UnixStream::pair().map_err(|e| e.to_string())?;
        let (read_err, write_err) = UnixStream::pair().map_err(|e| e.to_string())?;
        let mut command = Command::new(path);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(write_out)))
            .stderr(Stdio::from(std::os::fd::OwnedFd::from(write_err)))
            .process_group(0);
        if let Some(home) = home {
            command
                .env_clear()
                .env("HOME", home)
                .env("CFFIXED_USER_HOME", home)
                .env("TMPDIR", home.join("tmp"))
                .env("PATH", "/usr/bin:/bin")
                .env("MTL_DEBUG_LAYER", "1");
        }
        let child = command.spawn().map_err(|e| e.to_string())?;
        drop(command);
        let stopped = Arc::new(AtomicBool::new(false));
        let (out, a) = collector(read_out, cap, Arc::clone(&stopped))?;
        let (err, b) = collector(read_err, 65_536, Arc::clone(&stopped))?;
        Ok(Self {
            child,
            out,
            err,
            readers: vec![a, b],
            evidence,
            stopped,
            group_closed: false,
        })
    }

    fn text(buffer: &Buffer) -> Result<String> {
        let data = buffer.lock().map_err(|e| e.to_string())?;
        if data.1 {
            return Err("child output cap or read failure".into());
        }
        String::from_utf8(data.0.clone()).map_err(|e| e.to_string())
    }
    fn output(&self) -> Result<String> {
        Self::text(&self.err)?;
        Self::text(&self.out)
    }
    fn check(buffer: &Buffer) -> Result<()> {
        if buffer.lock().map_err(|e| e.to_string())?.1 {
            return Err("child output cap or read failure".into());
        }
        Ok(())
    }
    fn done(&mut self) -> Result<bool> {
        Self::check(&self.out)?;
        Self::check(&self.err)?;
        if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
            self.close_group(true)?;
            self.join();
            self.output()?;
            if !status.success() {
                return Err(format!(
                    "owned child exited {status}: {}",
                    Self::text(&self.err)?
                ));
            }
            return Ok(true);
        }
        Ok(false)
    }
    fn send(&mut self, text: &str) -> Result<()> {
        self.child
            .stdin
            .as_mut()
            .ok_or("stdin unavailable")?
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())
    }
    fn close_group(&mut self, exited: bool) -> Result<()> {
        if !self.group_closed {
            self.group_closed = true;
            kill_group(self.child.id(), exited)?;
        }
        Ok(())
    }
    fn join(&mut self) {
        self.stopped.store(true, Ordering::Release);
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        let exited = self.child.try_wait().ok().flatten().is_some();
        let cleanup = self.close_group(exited);
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.join();
        if let Some(path) = &self.evidence {
            if let Err(error) = cleanup {
                let _ = fs::write(path.with_extension("cleanup-error"), error);
            }
            for (suffix, buffer) in [("out", &self.out), ("err", &self.err)] {
                if let Ok(data) = buffer.lock() {
                    let _ = fs::write(path.with_extension(suffix), &data.0);
                }
            }
        }
    }
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or("session deadline expired".into())
}
fn sample_deadline(start: Instant, phase_deadline: Instant) -> Instant {
    phase_deadline.min(start + Duration::from_secs(1))
}
fn poll_delay(now: Instant, next_poll: Instant, phase_deadline: Instant) -> Result<Duration> {
    if now >= phase_deadline {
        return Err("session deadline expired".into());
    }
    Ok(next_poll
        .saturating_duration_since(now)
        .min(phase_deadline - now)
        .min(POLL))
}

fn bounded(path: &Path, args: &[&str], deadline: Instant) -> Result<String> {
    remaining(deadline)?;
    let mut process = Owned::spawn(path, args, 65_536, None, None)?;
    loop {
        remaining(deadline)?;
        if process.done()? {
            remaining(deadline)?;
            return process.output();
        }
        thread::sleep(POLL);
    }
}
fn capture_started(stderr: &str, pid: u32, window: u32) -> Result<bool> {
    let expected = format!("scene-stream-start\talpine-scene-diagnostic/v1\t{pid}\t{window}");
    let complete = stderr.rsplit_once('\n').map_or("", |(done, _)| done);
    let mut markers = complete
        .lines()
        .filter(|line| line.starts_with("scene-stream-start"));
    match (markers.next(), markers.next()) {
        (None, None) => Ok(false),
        (Some(line), None) if line == expected => Ok(true),
        _ => Err("mismatched or duplicate capture-start marker".into()),
    }
}

struct Session {
    helpers: PathBuf,
    log: Vec<u8>,
    path: PathBuf,
    revision: String,
    sharing: Option<Inspection>,
}
impl Session {
    fn call(&mut self, name: &str, args: &[&str], deadline: Instant) -> Result<String> {
        let path = if name == "pmset" {
            PathBuf::from("/usr/bin/pmset")
        } else {
            self.helpers.join(name)
        };
        let output = bounded(
            &path,
            args,
            deadline.min(Instant::now() + Duration::from_secs(1)),
        );
        let record = format!("{name} {args:?}: {output:?}\n");
        if self.log.len() + record.len() > 512 * 1024 {
            return Err("helper evidence cap exceeded".into());
        }
        self.log.extend_from_slice(record.as_bytes());
        output
    }
    fn conditions(&mut self, deadline: Instant) -> Result<bool> {
        let deadline = sample_deadline(Instant::now(), deadline);
        let presence =
            helpers::parse_presence(&self.call(helpers::WINDOW, &["idle"], deadline)?)?;
        let power = gate::parse_power(&self.call("pmset", &["-g", "ps"], deadline)?)?;
        remaining(deadline)?;
        Ok(!presence.locked && presence.idle_seconds >= 60.0 && power == gate::Power::Ac)
    }
    fn require_conditions(&mut self, deadline: Instant) -> Result<()> {
        if !self.conditions(deadline)? {
            return Err("input, lock or power changed; session unavailable".into());
        }
        Ok(())
    }
    fn verify(
        &mut self,
        app: &mut Owned,
        window: &helpers::WindowInfo,
        capture: Option<&Owned>,
        deadline: Instant,
    ) -> Result<()> {
        let deadline = sample_deadline(Instant::now(), deadline);
        self.require_conditions(deadline)?;
        self.window(app, Some(window), capture, deadline)?;
        remaining(deadline)?;
        Ok(())
    }
    fn window(
        &mut self,
        app: &mut Owned,
        pin: Option<&helpers::WindowInfo>,
        capture: Option<&Owned>,
        deadline: Instant,
    ) -> Result<helpers::WindowInfo> {
        let pid = i32::try_from(app.child.id()).map_err(|e| e.to_string())?;
        let mut list =
            helpers::parse_window_list(&self.call(helpers::WINDOW, &["scene-list"], deadline)?)?;
        let start_check = if self.sharing.is_none()
            && let (Some(pin), Some(capture)) = (pin, capture)
            && list
                .windows
                .iter()
                .filter(|w| w.pid == pid && w.alpha > 0.0)
                .count()
                == 2
        {
            self.confirm_start_pair(&mut list, pin, capture, deadline)
        } else {
            Ok(())
        };
        let owned = list
            .windows
            .iter()
            .filter(|w| w.pid == pid && w.alpha > 0.0);
        let count = owned.clone().count();
        let result = start_check
            .and_then(|()| select_fixture_window(&list, pid, pin, self.sharing.is_some()))
            .and_then(|(main, child)| {
                if let Some(child) = child {
                    self.sharing
                        .as_mut()
                        .ok_or("capture start unavailable")?
                        .verify(app, &main, child, deadline)?;
                }
                Ok(main)
            });
        match result {
            Err(error) if count != 1 => {
                let mut cmd = String::from("inspect-stop");
                for w in owned.take(4) {
                    write!(&mut cmd, " {}", w.id).map_err(|e| e.to_string())?;
                }
                cmd.push('\n');
                let end = sample_deadline(Instant::now(), deadline);
                let stopped = self
                    .require_conditions(end)
                    .and_then(|()| inspect_stop(app, &cmd, end));
                Err(format!(
                    "{error} (count={count}, command={}, inspection={stopped:?})",
                    cmd.trim_end()
                ))
            }
            other => other,
        }
    }
    fn confirm_start_pair(
        &mut self,
        first: &mut helpers::WindowList,
        pin: &helpers::WindowInfo,
        capture: &Owned,
        deadline: Instant,
    ) -> Result<()> {
        let pid = u32::try_from(pin.pid).map_err(|e| e.to_string())?;
        *first = consistent_start_sample(first, pin, || {
            while !capture_started(&Owned::text(&capture.err)?, pid, pin.id)? {
                thread::sleep(remaining(deadline)?.min(POLL));
            }
            helpers::parse_window_list(&self.call(helpers::WINDOW, &["scene-list"], deadline)?)
        })?;
        self.sharing = Some(Inspection::default());
        Ok(())
    }
}

fn consistent_start_sample(
    first: &helpers::WindowList,
    pin: &helpers::WindowInfo,
    after_start: impl FnOnce() -> Result<helpers::WindowList>,
) -> Result<helpers::WindowList> {
    let (main, child) = select_fixture_window(first, pin.pid, Some(pin), true)?;
    let child = child.ok_or("fixture windows changed across capture start")?;
    let fresh = after_start()?;
    let (next_main, next_child) = select_fixture_window(&fresh, pin.pid, Some(pin), true)?;
    if !same_window(&main, &next_main) || !next_child.is_some_and(|next| same_window(child, next)) {
        return Err("fixture windows changed across capture start".into());
    }
    Ok(fresh)
}

fn select_fixture_window<'a>(
    list: &'a helpers::WindowList,
    pid: i32,
    pin: Option<&helpers::WindowInfo>,
    active: bool,
) -> Result<(helpers::WindowInfo, Option<&'a helpers::WindowInfo>)> {
    if list.windows.iter().any(|w| {
        ![w.alpha, w.x, w.y, w.width, w.height]
            .iter()
            .all(|v| v.is_finite())
            || !(0.0..=1.0).contains(&w.alpha)
            || w.width < 0.0
            || w.height < 0.0
    }) {
        return Err("invalid CG window list geometry".into());
    }
    let main = match analysis::visibility(list, pid, 100.0) {
        analysis::Visibility::Visible(main) => main,
        other => return Err(other.describe()),
    };
    if pin.is_some_and(|w| !same_window(w, &main)) {
        return Err("pinned fixture changed".into());
    }
    let owned = list
        .windows
        .iter()
        .filter(|w| w.pid == pid && w.alpha > 0.0);
    if owned.clone().filter(|w| w.id == main.id).count() != 1 {
        return Err("ambiguous fixture identity".into());
    }
    let mut others = owned.filter(|w| w.id != main.id);
    let child = others.next();
    if let Some(child) = child
        && (!active
            || pin.is_none()
            || others.next().is_some()
            || child.layer != 0
            || child.alpha.to_bits() != 1.0f64.to_bits()
            || child.order >= main.order
            || child.id == 0)
    {
        return Err("fixture has an unsupported visible window".into());
    }
    Ok((main, child))
}

#[derive(Default)]
struct Inspection {
    sequence: u8,
    previous: Option<Proof>,
}
impl Inspection {
    fn audit(&self, text: &str) -> Result<()> {
        let mut seen = 0usize;
        for line in text
            .lines()
            .filter(|line| line.starts_with("scene-sharing"))
        {
            seen += 1;
            let mut fields = line.split('\t');
            if fields.next() != Some("scene-sharing")
                || fields.next().and_then(|s| s.parse::<usize>().ok()) != Some(seen)
            {
                return Err("unsolicited or stale inspection reply".into());
            }
        }
        if seen != usize::from(self.sequence) {
            return Err("unexpected inspection reply count".into());
        }
        Ok(())
    }
    fn verify(
        &mut self,
        app: &mut Owned,
        main: &helpers::WindowInfo,
        child: &helpers::WindowInfo,
        deadline: Instant,
    ) -> Result<()> {
        if self.sequence >= 32 {
            return Err("sharing inspection limit exceeded".into());
        }
        remaining(deadline)?;
        let prior = app.output()?;
        self.audit(&prior)?;
        if !prior.ends_with('\n') {
            return Err("incomplete fixture output before inspection".into());
        }
        self.sequence += 1;
        app.send(&format!(
            "sharing {} {} {}\n",
            self.sequence, main.id, child.id
        ))?;
        loop {
            remaining(deadline)?;
            if app.done()? {
                return Err("fixture ended during inspection".into());
            }
            let text = app.output()?;
            let fresh = text.strip_prefix(&prior).ok_or("fixture output changed")?;
            if let Some((complete, _)) = fresh.rsplit_once('\n') {
                let mut replies = complete.lines().filter(|l| l.starts_with("scene-sharing"));
                if let Some(line) = replies.next() {
                    let proof = Proof::read(line, self.sequence, main, child)?;
                    if replies.next().is_some()
                        || self.previous.as_ref().is_some_and(|p| p != &proof)
                    {
                        return Err("duplicate or changed sharing indicator".into());
                    }
                    remaining(deadline)?;
                    self.previous = Some(proof);
                    return Ok(());
                }
            }
            thread::sleep(remaining(deadline)?.min(POLL));
        }
    }
}

fn inspect_stop(app: &mut Owned, command: &str, deadline: Instant) -> Result<()> {
    remaining(deadline)?;
    app.send(command)?;
    while !app.done()? {
        thread::sleep(remaining(deadline)?.min(POLL));
    }
    remaining(deadline)?;
    let output = app.output()?;
    if !output
        .lines()
        .any(|line| line.starts_with("scene-inspect-end\t"))
        || !output
            .lines()
            .any(|line| line.starts_with("scene-cleanup\texact=true\t"))
    {
        return Err("inspection or exact teardown record unavailable".into());
    }
    Ok(())
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::write(&self.path, &self.log);
    }
}

#[allow(
    clippy::float_cmp,
    reason = "fixed fixture geometry must match exactly"
)]
fn region(window: &helpers::WindowInfo) -> Result<(f64, f64)> {
    if !window.width.is_finite()
        || !(500.0..=504.0).contains(&window.width)
        || window.width % 2.0 != 0.0
        || !window.height.is_finite()
        || !(300.0..=350.0).contains(&window.height)
        || window.height % 2.0 != 0.0
        || !window.x.is_finite()
        || !window.y.is_finite()
        || window.alpha != 1.0
    {
        return Err("unsupported fixture outer geometry".into());
    }
    Ok(((window.width - 500.0) / 2.0, (window.height - 40.0) / 2.0))
}
#[allow(
    clippy::float_cmp,
    reason = "fixed fixture geometry must match exactly"
)]
fn ready(text: &str, pid: u32, revision: &str) -> Result<()> {
    if text
        .lines()
        .filter(|line| line.starts_with("scene-build\t"))
        .collect::<Vec<_>>()
        != [format!("scene-build\t{revision}")]
    {
        return Err("fixture build revision mismatch".into());
    }
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("scene-ready\t"))
        .collect();
    if lines.len() != 1 {
        return Err("expected one fixture ready record".into());
    }
    let f: Vec<_> = lines[0].split('\t').collect();
    if f.len() != 7 || f[1] != pid.to_string() || f[2] != "500" || f[3] != "300" {
        return Err("fixture identity/content mismatch".into());
    }
    let values = f[4..]
        .iter()
        .map(|v| v.parse::<f64>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>>>()?;
    if !values[0].is_finite()
        || !(1.0..=4.0).contains(&values[0])
        || values[1] != 500.0 * values[0]
        || values[2] != 300.0 * values[0]
    {
        return Err("fixture backing geometry mismatch".into());
    }
    Ok(())
}
#[allow(
    clippy::float_cmp,
    reason = "fixed fixture geometry must match exactly"
)]
fn same_window(a: &helpers::WindowInfo, b: &helpers::WindowInfo) -> bool {
    a.id == b.id
        && a.pid == b.pid
        && a.layer == b.layer
        && a.x == b.x
        && a.y == b.y
        && a.width == b.width
        && a.height == b.height
        && a.alpha == b.alpha
}
fn action_record(text: &str, pid: u32, window: u32, tick: u64) -> Result<String> {
    let lines: Vec<_> = text
        .lines()
        .filter(|l| l.starts_with("scene-action\t"))
        .collect();
    let expected =
        format!("scene-action\talpine-scene-diagnostic/v1\t{pid}\t{window}\t{tick}\tA\tB");
    if lines != [expected.as_str()] {
        return Err("trusted fixture did not accept exactly the issued action".into());
    }
    Ok(expected + "\n")
}

fn start_capture(
    helpers: &Path,
    dir: &Path,
    pid: u32,
    window: u32,
    region: (f64, f64),
) -> Result<Owned> {
    Owned::spawn(
        &helpers.join(helpers::CAPTURE),
        &[
            "scene",
            "--pid",
            &pid.to_string(),
            "--window-id",
            &window.to_string(),
            "--duration-ms",
            "8000",
            "--region",
            &format!("{},{},500,40", region.0, region.1),
            "--scale",
            "1",
        ],
        CAP,
        Some(dir.join("capture")),
        None,
    )
}

fn wait_for_baseline(
    session: &mut Session,
    app: &mut Owned,
    capture: &mut Owned,
    window: &helpers::WindowInfo,
    region: (f64, f64),
    deadline: Instant,
) -> Result<()> {
    let baseline_deadline = (Instant::now() + Duration::from_secs(5)).min(deadline);
    let mut next_check = Instant::now();
    loop {
        remaining(baseline_deadline)?;
        let started = capture_started(&Owned::text(&capture.err)?, app.child.id(), window.id)?;
        let baseline = started
            && physical_evidence::baseline(
                &capture.output()?,
                app.child.id(),
                window.id,
                window.width,
                window.height,
                region.0,
                region.1,
            )
            .is_ok();
        if started {
            session.sharing.get_or_insert_with(Inspection::default);
        }
        if Instant::now() >= next_check {
            let start = Instant::now();
            next_check = start + Duration::from_secs(1);
            session.verify(
                app,
                window,
                Some(capture),
                sample_deadline(start, baseline_deadline),
            )?;
        }
        if app.done()? || capture.done()? {
            return Err("child ended before baseline".into());
        }
        if baseline {
            break;
        }
        thread::sleep(poll_delay(Instant::now(), next_check, baseline_deadline)?);
    }
    Ok(())
}

fn case(session: &mut Session, fixture: &Path, dir: &Path, control: &str) -> Result<String> {
    session.sharing = None;
    fs::create_dir(dir).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let home = dir.join("home");
    fs::create_dir_all(home.join("tmp")).map_err(|e| e.to_string())?;
    let mut app = Owned::spawn(
        fixture,
        &[control],
        65_536,
        Some(dir.join("fixture")),
        Some(&home),
    )?;
    let pid = app.child.id();
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    let mut next_check = Instant::now();
    loop {
        remaining(ready_deadline)?;
        if Instant::now() >= next_check {
            let start = Instant::now();
            next_check = start + Duration::from_secs(1);
            session.require_conditions(sample_deadline(start, ready_deadline))?;
        }
        if app.done()? {
            return Err("fixture exited before ready".into());
        }
        if ready(&app.output()?, pid, &session.revision).is_ok() {
            break;
        }
        thread::sleep(poll_delay(Instant::now(), next_check, ready_deadline)?);
    }
    session.require_conditions(ready_deadline)?;
    session.call(
        helpers::WINDOW,
        &["activate", "--pid", &pid.to_string()],
        ready_deadline,
    )?;
    let window = session.window(&mut app, None, None, ready_deadline)?;
    let region = region(&window)?;
    session.verify(&mut app, &window, None, deadline)?;
    let capture_deadline = (Instant::now() + Duration::from_secs(10)).min(deadline);
    let mut capture = start_capture(&session.helpers, dir, pid, window.id, region)?;
    wait_for_baseline(session, &mut app, &mut capture, &window, region, deadline)?;
    session.verify(&mut app, &window, None, capture_deadline)?;
    let (tick, timebase) =
        helpers::parse_now(&session.call(helpers::WINDOW, &["now"], capture_deadline)?)?;
    physical_evidence::clock_matches(&capture.output()?, tick, timebase)?;
    let mut action = Issued {
        pid,
        window: window.id,
        tick,
        witness: None,
    };
    app.send(&format!("transition {} {tick}\n", window.id))?;
    monitor_capture(
        session,
        &mut app,
        &mut capture,
        &window,
        control,
        &mut action,
        (dir, capture_deadline),
    )?;
    session.verify(&mut app, &window, None, deadline)?;
    app.send("stop\n")?;
    let drain_deadline = (Instant::now() + Duration::from_secs(5)).min(deadline);
    while !app.done()? {
        remaining(drain_deadline)?;
        thread::sleep(POLL);
    }
    let native = app.output()?;
    if !capture_started(&Owned::text(&capture.err)?, pid, window.id)? {
        return Err("missing capture-start marker".into());
    }
    session
        .sharing
        .as_ref()
        .ok_or("missing capture baseline")?
        .audit(&native)?;
    finish_case(&native, &capture.output()?, dir, control, &action)
}

struct Issued {
    pid: u32,
    window: u32,
    tick: u64,
    witness: Option<u64>,
}

fn monitor_capture(
    session: &mut Session,
    app: &mut Owned,
    capture: &mut Owned,
    window: &helpers::WindowInfo,
    control: &str,
    action: &mut Issued,
    run: (&Path, Instant),
) -> Result<()> {
    let (dir, deadline) = run;
    let mut next_check = Instant::now();
    loop {
        remaining(deadline)?;
        let native = app.output()?;
        let witness_due = control == "stale"
            && action.witness.is_none()
            && native.rsplit_once('\n').is_some_and(|(complete, _)| {
                complete.lines().any(|l| l.starts_with("scene-action\t"))
            });
        if Instant::now() >= next_check || witness_due {
            let start = Instant::now();
            next_check = start + Duration::from_secs(1);
            let end = sample_deadline(start, deadline);
            session.verify(app, window, None, end)?;
            if witness_due {
                action_record(&native, action.pid, action.window, action.tick)?;
                let (after, base) =
                    helpers::parse_now(&session.call(helpers::WINDOW, &["now"], end)?)?;
                physical_evidence::clock_matches(&capture.output()?, after, base)?;
                if after <= action.tick {
                    return Err("admission witness did not follow issued action".into());
                }
                fs::write(
                    dir.join("admission.tsv"),
                    format!(
                        "scene-admission-witness\t{}\t{}\t{}\t{after}\t{}\t{}\n",
                        action.pid, action.window, action.tick, base.numer, base.denom
                    ),
                )
                .map_err(|e| e.to_string())?;
                action.witness = Some(after);
            }
        }
        if app.done()? {
            return Err("fixture ended during capture".into());
        }
        if capture.done()? {
            return Ok(());
        }
        thread::sleep(poll_delay(Instant::now(), next_check, deadline)?);
    }
}

fn finish_case(
    native: &str,
    captured: &str,
    dir: &Path,
    control: &str,
    action: &Issued,
) -> Result<String> {
    if !native
        .lines()
        .any(|l| l.starts_with("scene-cleanup\texact=true\t"))
    {
        return Err("no exact native owner cleanup".into());
    }
    fs::write(
        dir.join("action.tsv"),
        action_record(native, action.pid, action.window, action.tick)?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(dir.join("capture.tsv"), captured).map_err(|e| e.to_string())?;
    let capture_path = dir.join("capture.tsv");
    let action_path = dir.join("action.tsv");
    let replay = physical_evidence::run(&capture_path, &action_path);
    let accepted = if control == "positive" {
        replay.as_ref().map(|_| ()).map_err(Clone::clone)
    } else {
        physical_evidence::negative(&capture_path, &action_path, control, action.witness)
    };
    let report = format!(
        "control={control}; expected_outcome={}; replay={replay:?}; control_validation={accepted:?}; physical qualification=false; typing qualification=false; action tick is command-issued lower bound",
        accepted.is_ok()
    );
    fs::write(dir.join("result.txt"), &report).map_err(|e| e.to_string())?;
    accepted?;
    Ok(report)
}

fn stamp(fixture: &Path, helpers: &Path, output: &Path) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("repository missing")?;
    let repo = repo.to_str().ok_or("non-UTF8 repository")?;
    let revision = bounded(
        Path::new("/usr/bin/git"),
        &["-C", repo, "rev-parse", "HEAD"],
        deadline,
    )?
    .trim()
    .to_owned();
    if !bounded(
        Path::new("/usr/bin/git"),
        &["-C", repo, "status", "--porcelain"],
        deadline,
    )?
    .trim()
    .is_empty()
    {
        return Err("scene session requires a clean repository".into());
    }
    if option_env!("ALPINE_SCENE_REVISION") != Some(revision.as_str()) {
        return Err("rebuild bench with ALPINE_SCENE_REVISION at clean HEAD".into());
    }
    let mut helper_stamp = String::new();
    fs::File::open(helpers.join("stamp.tsv"))
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_string(&mut helper_stamp)
        .map_err(|e| e.to_string())?;
    if helper_stamp.len() > 65_536
        || !helper_stamp
            .lines()
            .any(|l| l == format!("revision\t{revision}"))
        || !helper_stamp.lines().any(|l| l == "tree\tclean")
    {
        return Err("helper build stamp is not clean HEAD".into());
    }
    let mut record = format!(
        "revision={revision}\ntree=clean\nfixture_env=cleared; HOME/CFFIXED_USER_HOME/TMPDIR=disposable; PATH=/usr/bin:/bin; MTL_DEBUG_LAYER=1\nprofile=scale1,central500x40,8seconds,one positive/stale/wrong,no retries\n{helper_stamp}"
    );
    for path in [
        fixture.to_path_buf(),
        std::env::current_exe().map_err(|e| e.to_string())?,
        helpers.join(helpers::WINDOW),
        helpers.join(helpers::CAPTURE),
    ] {
        let hash = bounded(
            Path::new("/usr/bin/shasum"),
            &["-a", "256", path.to_str().ok_or("non-UTF8 binary path")?],
            deadline,
        )?;
        let sha = hash
            .split_whitespace()
            .next()
            .ok_or("missing binary hash")?;
        if path == helpers.join(helpers::WINDOW) || path == helpers.join(helpers::CAPTURE) {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("helper name")?;
            if !helper_stamp
                .lines()
                .any(|l| l == format!("binary\t{name}\t{sha}"))
            {
                return Err("helper binary differs from build stamp".into());
            }
        }
        record.push_str(&hash);
    }
    fs::write(output.join("stamp.txt"), record).map_err(|e| e.to_string())?;
    Ok(revision)
}

/// Runs one prospective set of controls after explicit screen-session approval.
pub fn run(fixture: &Path, helper_dir: &Path, output: &Path) -> Result<()> {
    fs::create_dir(output).map_err(|e| e.to_string())?;
    let revision = stamp(fixture, helper_dir, output).inspect_err(|error| {
        let _ = fs::write(
            output.join("result.txt"),
            format!("unavailable: {error}; physical qualification=false\n"),
        );
    })?;
    let mut session = Session {
        helpers: helper_dir.to_path_buf(),
        log: Vec::new(),
        path: output.join("helpers.log"),
        revision,
        sharing: None,
    };
    let wait = Instant::now() + Duration::from_mins(5);
    let precondition = (|| {
        let mut next_check = Instant::now();
        loop {
            remaining(wait)?;
            if Instant::now() >= next_check {
                let start = Instant::now();
                next_check = start + Duration::from_secs(1);
                if session.conditions(sample_deadline(start, wait))? {
                    return Ok(());
                }
            }
            thread::sleep(poll_delay(Instant::now(), next_check, wait)?);
        }
    })();
    if let Err(error) = precondition {
        fs::write(
            output.join("result.txt"),
            format!("unavailable: {error}; physical qualification=false\n"),
        )
        .map_err(|e| e.to_string())?;
        return Err(error);
    }
    let mut report =
        String::from("DIAGNOSTIC ONLY; one run per control; no retries; Metal gate unchanged\n");
    for control in ["positive", "stale", "wrong"] {
        let result = session
            .require_conditions(Instant::now() + Duration::from_secs(3))
            .and_then(|()| case(&mut session, fixture, &output.join(control), control));
        writeln!(&mut report, "{control}: {result:?}").map_err(|e| e.to_string())?;
        fs::write(output.join("result.txt"), &report).map_err(|e| e.to_string())?;
        if result.is_err() {
            return Err("session unavailable; see retained result and child logs".into());
        }
    }
    println!("{report}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> helpers::WindowInfo {
        helpers::WindowInfo {
            order: 0,
            id: 3,
            pid: 2,
            owner: "fixture".into(),
            layer: 0,
            alpha: 1.0,
            x: 10.0,
            y: 20.0,
            width: 500.0,
            height: 328.0,
        }
    }
    fn observed(pid: i32) -> (helpers::WindowInfo, helpers::WindowInfo, String) {
        let mut main = window();
        main.pid = pid;
        main.id = 220_325;
        main.x = 506.0;
        main.y = 342.0;
        main.height = 332.0;
        main.order = 13;
        let mut child = main.clone();
        child.id = 220_333;
        child.order = 12;
        child.x = 512.0;
        child.y = 348.0;
        child.width = 66.0;
        child.height = 20.0;
        let reply = format!(
            "scene-sharing\t1\t{pid}\t220325\t220333\tNSKVONotifying_NSLocalWindowSharingWindow\t220325\t506,342,500,332\t506,374,500,300\t512,348,66,20\tNSLocalWindowSharingWindow\tkvo-direct"
        );
        (main, child, reply)
    }
    #[test]
    fn sharing_requires_a_pinned_main_and_checks_all_layers() -> Result<()> {
        let (main, child, _) = observed(2);
        let mut list = helpers::WindowList {
            front_pid: 2,
            front_name: "fixture".into(),
            windows: vec![main.clone()],
        };
        assert!(select_fixture_window(&list, 2, None, false)?.1.is_none());
        list.windows.insert(0, child.clone());
        assert!(select_fixture_window(&list, 2, Some(&main), false).is_err());
        assert!(select_fixture_window(&list, 2, None, true).is_err());
        assert_eq!(
            select_fixture_window(&list, 2, Some(&main), true)?.1,
            Some(&child)
        );
        for layer in [1, 10, -1] {
            list.windows[0].layer = layer;
            assert!(select_fixture_window(&list, 2, Some(&main), true).is_err());
        }
        list.windows[0] = child.clone();
        list.windows[0].pid = 9;
        assert!(select_fixture_window(&list, 2, Some(&main), true).is_err());
        list.windows[0] = child.clone();
        list.windows.push(child.clone());
        assert!(select_fixture_window(&list, 2, Some(&main), true).is_err());
        list.windows.pop();
        list.windows[0].alpha = f64::NAN;
        assert!(select_fixture_window(&list, 2, Some(&main), true).is_err());
        list.windows[0] = child;
        list.windows[1].id += 1;
        assert!(select_fixture_window(&list, 2, Some(&main), true).is_err());
        Ok(())
    }
    #[test]
    fn requires_one_complete_start_marker_for_the_launched_capture() -> Result<()> {
        let marker = "scene-stream-start\talpine-scene-diagnostic/v1\t24\t35\n";
        assert!(capture_started(marker, 24, 35)?);
        assert!(!capture_started("scene-source\t24\t35\n", 24, 35)?);
        assert!(!capture_started(marker.trim_end(), 24, 35)?);
        assert!(capture_started(marker, 23, 35).is_err());
        assert!(capture_started(marker, 24, 34).is_err());
        assert!(capture_started(&marker.repeat(2), 24, 35).is_err());
        Ok(())
    }
    #[test]
    fn recorded_base_indicator_and_verified_kvo_view_have_one_identity() -> Result<()> {
        let (mut main, mut child, _) = observed(37968);
        main.id = 220_503;
        child.id = 220_511;
        let row = "scene-sharing\t1\t37968\t220503\t220511\tNSLocalWindowSharingWindow\t220503\t506,342,500,332\t506,374,500,300\t512,348,66,20";
        assert!(Proof::read(row, 1, &main, &child).is_err());
        let exact = format!("{row}\tNSLocalWindowSharingWindow\texact");
        let wrapped = exact
            .replacen(
                "\tNSLocalWindowSharingWindow\t220503",
                "\tNSKVONotifying_NSLocalWindowSharingWindow\t220503",
                1,
            )
            .replace("\texact", "\tkvo-direct");
        assert_eq!(
            Proof::read(&exact, 1, &main, &child)?,
            Proof::read(&wrapped, 1, &main, &child)?
        );
        for invalid in [
            exact.replace("\texact", "\tkvo-direct"),
            wrapped.replace("\tkvo-direct", "\texact"),
            wrapped.replace("KVONotifying_", "OtherSubclass_"),
            wrapped.replace("KVONotifying_", "KVONotifying_NSKVONotifying_"),
            exact.replace("\tNSLocalWindowSharingWindow\texact", "\tunresolved\texact"),
            wrapped.replace("\tkvo-direct", "\tunresolved"),
        ] {
            assert!(Proof::read(&invalid, 1, &main, &child).is_err());
        }
        Ok(())
    }
    #[test]
    fn sharing_proof_rejects_unresolved_moved_and_content_overlapping_rows() -> Result<()> {
        let (main, child, reply) = observed(2);
        Proof::read(&reply, 1, &main, &child)?;
        for bad in [
            reply.replace("Notifying_", ""),
            reply.replace("\t220325\t506,", "\t17\t506,"),
            reply.replace("\t220333\t", "\t9\t"),
            reply.replace("\t2\t220325", "\t7\t220325"),
            reply.replace("\t1\t2", "\t3\t2"),
            reply.replace("512,348,66,20", "512,348,66,19"),
            reply.replace("506,342,500,332", "506,343,500,332"),
            reply.replace("506,374,500,300", "506,375,500,300"),
            reply.replace("512,348", "512,NaN"),
            reply.clone() + "\textra",
            "scene-sharing\t1\tunresolved".into(),
        ] {
            assert!(Proof::read(&bad, 1, &main, &child).is_err(), "{bad}");
        }
        for (x, y) in [
            (505.0, 348.0),
            (512.0, 341.0),
            (512.0, 355.0),
            (512.0, 500.0),
        ] {
            let mut changed = child.clone();
            changed.x = x;
            changed.y = y;
            assert!(
                Proof::read(
                    &reply.replace("512,348,66,20", &format!("{x},{y},66,20")),
                    1,
                    &main,
                    &changed
                )
                .is_err()
            );
        }
        let audit = Inspection {
            sequence: 1,
            previous: None,
        };
        audit.audit(&reply)?;
        assert!(audit.audit("").is_err());
        assert!(
            audit
                .audit(&reply.replace("sharing\t1", "sharing\t2"))
                .is_err()
        );
        assert!(audit.audit(&(reply.clone() + "\n" + &reply)).is_err());
        Ok(())
    }
    #[test]
    fn start_marker_arriving_during_window_list_enables_inspection_before_first_frame() -> Result<()>
    {
        exercise_start_samples("stable")
    }
    #[test]
    fn first_sample_cannot_hide_an_occlusion_or_a_changed_indicator() -> Result<()> {
        exercise_start_samples("vanished")?;
        let (main, child, _) = observed(2);
        let clean = helpers::WindowList {
            front_pid: 2,
            front_name: "fixture".into(),
            windows: vec![child, main.clone()],
        };
        for kind in [
            "exchanged",
            "occluded",
            "moved",
            "reordered",
            "layer",
            "opacity",
            "main moved",
            "expanded",
        ] {
            let mut first = clean.clone();
            match kind {
                "exchanged" => first.windows[0].id += 1,
                "moved" => first.windows[0].x += 1.0,
                "reordered" => first.windows[0].order = 14,
                "layer" => first.windows[0].layer = 1,
                "opacity" => first.windows[0].alpha = 0.5,
                "main moved" => first.windows[1].x += 1.0,
                "expanded" => first.windows[0].width += 1.0,
                _ => {
                    let mut other = first.windows[0].clone();
                    other.order = 0;
                    other.id = 9;
                    other.pid = 99;
                    first.windows.insert(0, other);
                }
            }
            let mut sampled = false;
            assert!(
                consistent_start_sample(&first, &main, || {
                    sampled = true;
                    Ok(clean.clone())
                })
                .is_err(),
                "{kind}"
            );
            assert_eq!(sampled, matches!(kind, "exchanged" | "moved" | "expanded"));
        }
        Ok(())
    }
    fn exercise_start_samples(kind: &str) -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = crate::test_dir(&format!("sharing-list-race-{kind}"))?;
        let fixture = r#"printf 'ready\n'; while read -r verb seq main child; do test "$verb" = sharing || exit 0; printf 'scene-sharing\t%s\t%s\t%s\t%s\tNSKVONotifying_NSLocalWindowSharingWindow\t%s\t506,342,500,332\t506,374,500,300\t512,348,66,20\tNSLocalWindowSharingWindow\tkvo-direct\n' "$seq" "$$" "$main" "$child" "$main"; done"#;
        let mut app = Owned::spawn(Path::new("/bin/sh"), &["-c", fixture], 4096, None, None)?;
        let pid = i32::try_from(app.child.id()).map_err(|e| e.to_string())?;
        let (main, child, _) = observed(pid);
        let row = |w: &helpers::WindowInfo| {
            format!(
                "window\t{}\t{}\t{pid}\tfixture\t0\t1\t{}\t{}\t{}\t{}\n",
                w.order, w.id, w.x, w.y, w.width, w.height
            )
        };
        let write =
            |name: &str, data: String| fs::write(dir.join(name), data).map_err(|e| e.to_string());
        write(
            "list",
            format!("front\t{pid}\tfixture\n{}{}", row(&child), row(&main)),
        )?;
        write(
            "after",
            format!(
                "front\t{pid}\tfixture\n{}{}",
                if kind == "vanished" {
                    String::new()
                } else {
                    row(&child)
                },
                row(&main)
            ),
        )?;
        write(
            helpers::WINDOW,
            r#"#!/bin/sh
set -eu
base=${0%/*}
: > "$base/started"
while [ ! -f "$base/collected" ]; do /bin/sleep .01; done
if [ -f "$base/listed" ]; then /bin/cat "$base/after"; else : > "$base/listed"; /bin/cat "$base/list"; fi
"#
            .into(),
        )?;
        fs::set_permissions(dir.join(helpers::WINDOW), fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        write(
            "marker",
            format!("scene-stream-start\talpine-scene-diagnostic/v1\t{pid}\t220325\n"),
        )?;
        let capture_script = r#"while [ ! -f "$1/started" ]; do /bin/sleep .01; done; /bin/cat "$1/marker" >&2; read -r done"#;
        let capture = Owned::spawn(
            Path::new("/bin/sh"),
            &["-c", capture_script, "capture", dir.to_str().ok_or("path")?],
            4096,
            None,
            None,
        )?;
        assert!(capture.output()?.is_empty());
        let buffer = Arc::clone(&capture.err);
        let marker = dir.join("collected");
        let signal = thread::spawn(move || -> Result<()> {
            let end = Instant::now() + Duration::from_secs(2);
            while !Owned::text(&buffer)?.contains("scene-stream-start\t") {
                remaining(end)?;
                thread::sleep(POLL);
            }
            fs::write(marker, b"").map_err(|e| e.to_string())
        });
        let mut session = Session {
            helpers: dir.clone(),
            log: Vec::new(),
            path: dir.join("helpers.log"),
            revision: "test".into(),
            sharing: None,
        };
        let first = session.window(
            &mut app,
            Some(&main),
            Some(&capture),
            Instant::now() + Duration::from_secs(1),
        );
        signal.join().map_err(|_| "signal failed")??;
        if kind == "vanished" {
            let reason = first.err().ok_or("vanished window was admitted")?;
            assert!(reason.contains("changed across capture start"), "{reason}");
            assert!(!app.output()?.contains("scene-sharing\t"));
            return Ok(());
        }
        assert_eq!(first?, main);
        assert_eq!(
            String::from_utf8_lossy(&session.log)
                .matches("scene-list")
                .count(),
            2
        );
        let state = session.sharing.as_mut().ok_or("missing inspection")?;
        assert_eq!(state.sequence, 1);
        state.verify(
            &mut app,
            &main,
            &child,
            Instant::now() + Duration::from_secs(1),
        )?;
        assert_eq!(state.sequence, 2);
        let mut moved = child;
        moved.id += 1;
        assert!(
            state
                .verify(
                    &mut app,
                    &main,
                    &moved,
                    Instant::now() + Duration::from_secs(1)
                )
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn inspection_requires_owned_exit_and_exact_cleanup() -> Result<()> {
        for (script, valid) in [
            (
                "read -r command; test \"$command\" = 'inspect-stop 3 4' || exit 1; printf 'scene-inspect-end\trequested=2\tresolved=0\nscene-cleanup\texact=true\tfixture\n'",
                true,
            ),
            (
                "read -r command; printf 'scene-inspect-end\trequested=2\tresolved=0\n'",
                false,
            ),
            (
                "read -r command; printf 'scene-cleanup\texact=true\tfixture\n'",
                false,
            ),
        ] {
            let mut owned = Owned::spawn(Path::new("/bin/sh"), &["-c", script], 4096, None, None)?;
            assert_eq!(
                inspect_stop(
                    &mut owned,
                    "inspect-stop 3 4\n",
                    Instant::now() + Duration::from_secs(1)
                )
                .is_ok(),
                valid
            );
        }
        Ok(())
    }
    #[test]
    fn central_strip_and_backing_geometry_are_exact() -> Result<()> {
        assert_eq!(region(&window())?, (0.0, 144.0));
        let mut bad = window();
        for value in [499.0, 501.0, 503.0, 505.0, 502.5, f64::NAN] {
            bad.width = value;
            assert!(region(&bad).is_err());
        }
        bad = window();
        for value in [299.0, 331.0, 351.0, 328.5, f64::INFINITY] {
            bad.height = value;
            assert!(region(&bad).is_err());
        }
        bad = window();
        bad.x += 1.0;
        assert!(!same_window(&window(), &bad));
        let ready_line = "scene-build\t123\nscene-ready\t2\t500\t300\t2\t1000\t600\n";
        ready(ready_line, 2, "123")?;
        for line in [
            ready_line.replace("1000", "999"),
            ready_line.replace("\t2\t1000", "\tNaN\t1000"),
            ready_line.repeat(2),
        ] {
            assert!(ready(&line, 2, "123").is_err());
        }
        assert!(ready(ready_line, 3, "123").is_err());
        assert!(ready(ready_line, 2, "different").is_err());
        Ok(())
    }
    #[test]
    fn observed_outer_frame_accepts_the_content_border() -> Result<()> {
        let mut observed = window();
        observed.width = 502.0;
        observed.height = 332.0;
        assert_eq!(region(&observed)?, (1.0, 146.0));
        observed.width = 504.0;
        assert_eq!(region(&observed)?, (2.0, 146.0));
        for height in [300.0, 350.0] {
            observed.height = height;
            let (_, y) = region(&observed)?;
            assert!(y >= height - 300.0 && y + 40.0 <= 300.0);
        }
        Ok(())
    }
    #[test]
    fn accepted_action_must_match_the_issued_command_once() -> Result<()> {
        let line = "scene-action\talpine-scene-diagnostic/v1\t2\t3\t4\tA\tB\n";
        assert_eq!(action_record(line, 2, 3, 4)?, line);
        for bad in [
            line.repeat(2),
            line.replace("\t4\t", "\t5\t"),
            line.replace("\t3\t", "\t7\t"),
        ] {
            assert!(action_record(&bad, 2, 3, 4).is_err());
        }
        Ok(())
    }
    #[test]
    fn deadline_and_output_caps_stop_owned_processes() -> Result<()> {
        assert!(remaining(Instant::now()).is_err());
        assert!(
            bounded(
                Path::new("/bin/sleep"),
                &["10"],
                Instant::now() + Duration::from_millis(50)
            )
            .is_err()
        );
        let mut process = Owned::spawn(Path::new("/usr/bin/yes"), &[], 32, None, None)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            remaining(deadline)?;
            if process.done().is_err() {
                break;
            }
            thread::sleep(POLL);
        }
        assert!(process.out.lock().map_err(|e| e.to_string())?.0.len() <= 32);
        Ok(())
    }
    #[test]
    fn inherited_descriptors_do_not_escape_deadlines() -> Result<()> {
        let start = Instant::now();
        let output = bounded(
            Path::new("/bin/sh"),
            &["-c", "/bin/sleep 30 & echo $!"],
            start + Duration::from_secs(1),
        )?;
        let pid = output.trim().parse::<u32>().map_err(|e| e.to_string())?;
        let listing = bounded(
            Path::new("/bin/ps"),
            &[
                "-o",
                "pid=,stat=",
                "-p",
                &format!("{},{}", std::process::id(), pid),
            ],
            Instant::now() + Duration::from_secs(1),
        )?;
        assert!(
            listing.lines().any(|line| line.split_whitespace().next()
                == Some(std::process::id().to_string().as_str()))
        );
        assert!(
            listing
                .lines()
                .filter(|line| line.split_whitespace().next() == Some(pid.to_string().as_str()))
                .all(|line| line
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|state| state.starts_with('Z')))
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        Ok(())
    }
    #[test]
    fn sample_schedule_does_not_add_helper_time_to_poll_period() -> Result<()> {
        let start = Instant::now();
        let phase = start + Duration::from_secs(5);
        let next = sample_deadline(start, phase);
        assert_eq!(next, start + Duration::from_secs(1));
        assert_eq!(
            poll_delay(start + Duration::from_millis(990), next, phase)?,
            Duration::from_millis(10)
        );
        assert_eq!(poll_delay(next, next, phase)?, Duration::ZERO);
        assert_eq!(
            poll_delay(next + Duration::from_millis(1), next, phase)?,
            Duration::ZERO
        );
        let phase = start + Duration::from_millis(500);
        assert_eq!(sample_deadline(start, phase), phase);
        assert_eq!(
            poll_delay(start + Duration::from_millis(495), next, phase)?,
            Duration::from_millis(5)
        );
        assert!(poll_delay(phase, next, phase).is_err());
        assert!(poll_delay(phase + Duration::from_nanos(1), next, phase).is_err());
        Ok(())
    }
    #[test]
    fn partial_utf8_does_not_fail_running_child_checks() -> Result<()> {
        let buffer = Arc::new(Mutex::new((vec![0xe2, 0x82], false)));
        Owned::check(&buffer)?;
        assert!(Owned::text(&buffer).is_err());
        buffer.lock().map_err(|e| e.to_string())?.0.push(0xac);
        assert_eq!(Owned::text(&buffer)?, "€");
        Ok(())
    }
    #[test]
    fn collector_admits_exact_cap_and_rejects_one_more() -> Result<()> {
        for size in [32, 33] {
            let (read, mut write) = UnixStream::pair().map_err(|e| e.to_string())?;
            let stopped = Arc::new(AtomicBool::new(false));
            let (buffer, join) = collector(read, 32, stopped)?;
            write
                .write_all(&vec![b'x'; size])
                .map_err(|e| e.to_string())?;
            drop(write);
            join.join().map_err(|_| "collector panicked")?;
            assert_eq!(Owned::text(&buffer).is_ok(), size == 32);
        }
        Ok(())
    }
}
