//! Bounded language-server pool keyed by workspace and server identity.

use std::{
    mem,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
use crate::language_registry::LanguageRegistryError;
use crate::{
    language_registry::{LanguageRegistry, LanguageSpec},
    lsp_process::ProcessWake,
    rust_diagnostics::{
        DiscoveryEnv, LanguageEffect, LanguageWake, RustDiagnostics, RustDiagnosticsSnapshot,
        RustDocumentInput, discover_binaries,
    },
    syntax::SyntaxLanguage,
};

/// Hard concurrency cap: a sixth identity evicts rather than failing the open.
pub(crate) const MAX_WARM_SERVERS: usize = 5;
// Servers one process sample may see: every warm slot plus the fallback.
const RUNNING_SERVERS: usize = MAX_WARM_SERVERS + 1;
const _: () = assert!(RUNNING_SERVERS <= alpine_platform_macos::MAX_SAMPLED_CHILDREN);
const MAX_OPEN_PATHS: usize = 32;
const IDLE_TTL: Duration = Duration::from_mins(1);
// Discovery jobs that may wait behind the one in flight.
const DISCOVERY_QUEUE: usize = MAX_WARM_SERVERS;

struct Slot {
    workspace_root: PathBuf,
    server_id: Box<str>,
    last_used: Instant,
    attached: bool,
    idle_since: Option<Instant>,
    discovery: Discovery,
    model: RustDiagnostics,
}

enum Discovery {
    Needed,
    /// Holds the newest input and the wake built for the session's generation.
    Pending {
        token: u64,
        wake: ProcessWake,
        input: RustDocumentInput,
    },
    Done,
}

/// Finds one server's executable. Runs only on the discovery thread.
type Discover = Arc<dyn Fn(&DiscoveryJob) -> Option<PathBuf> + Send + Sync>;

struct DiscoveryJob {
    token: u64,
    workspace_root: PathBuf,
    server_id: Box<str>,
    binaries: Vec<Box<str>>,
    overlay: Option<PathBuf>,
    environment: DiscoveryEnv,
    wake: ProcessWake,
}

struct DiscoveryResult {
    token: u64,
    workspace_root: PathBuf,
    server_id: Box<str>,
    server: Option<PathBuf>,
}

fn run_discovery(job: &DiscoveryJob) -> Option<PathBuf> {
    discover_binaries(&job.binaries, job.overlay.as_deref(), &job.environment)
}

/// One long-lived thread that runs discovery jobs in order. Dropping this
/// closes the queue; the thread ends after its current job and is never
/// joined, so a probe blocked for seconds cannot hold up shutdown or quit.
struct DiscoveryWorker {
    jobs: SyncSender<DiscoveryJob>,
    results: Receiver<DiscoveryResult>,
}

impl DiscoveryWorker {
    fn spawn(discover: Discover) -> Option<Self> {
        let (jobs, queued) = mpsc::sync_channel::<DiscoveryJob>(DISCOVERY_QUEUE);
        // Every sync drains results before it queues a job, so the queue,
        // the job in flight and undelivered results never exceed this.
        let (finished, results) = mpsc::sync_channel(DISCOVERY_QUEUE + 1);
        thread::Builder::new()
            .name(String::from("alpine-lsp-discovery"))
            .spawn(move || {
                while let Ok(job) = queued.recv() {
                    let server = discover(&job);
                    let result = DiscoveryResult {
                        token: job.token,
                        workspace_root: job.workspace_root,
                        server_id: job.server_id,
                        server,
                    };
                    if finished.send(result).is_err() {
                        break;
                    }
                    (job.wake)();
                }
            })
            .ok()?;
        Some(Self { jobs, results })
    }
}

/// Registry lookup plus at most five warm language-server sessions.
///
/// Highlighting never waits on this type. Idle servers with no attached buffers
/// shut down even when the cap is not hit. Opening a sixth identity evicts the
/// idle-oldest slot instead of refusing the file.
pub(crate) struct LanguageServices {
    registry: LanguageRegistry,
    discovery_overlay: Option<PathBuf>,
    discovery_enabled: bool,
    open_paths: Vec<PathBuf>,
    slots: Vec<Slot>,
    active_index: Option<usize>,
    fallback: RustDiagnostics,
    discover: Discover,
    discovery_worker: Option<DiscoveryWorker>,
    discovery_token: u64,
    #[cfg(test)]
    discovery_env: Option<DiscoveryEnv>,
    #[cfg(test)]
    discovery_dispatches: u64,
    #[cfg(test)]
    stale_discoveries: u64,
}

impl Default for LanguageServices {
    fn default() -> Self {
        Self {
            registry: LanguageRegistry::compiled(),
            discovery_overlay: None,
            discovery_enabled: false,
            open_paths: Vec::new(),
            slots: Vec::new(),
            active_index: None,
            fallback: RustDiagnostics::default(),
            discover: Arc::new(run_discovery),
            discovery_worker: None,
            discovery_token: 0,
            #[cfg(test)]
            discovery_env: None,
            #[cfg(test)]
            discovery_dispatches: 0,
            #[cfg(test)]
            stale_discoveries: 0,
        }
    }
}

impl From<RustDiagnostics> for LanguageServices {
    fn from(model: RustDiagnostics) -> Self {
        Self {
            fallback: model,
            ..Self::default()
        }
    }
}

impl Deref for LanguageServices {
    type Target = RustDiagnostics;

    fn deref(&self) -> &Self::Target {
        self.active_model()
    }
}

impl DerefMut for LanguageServices {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.active_model_mut()
    }
}

impl LanguageServices {
    #[cfg(test)]
    pub(crate) fn registry(&self) -> &LanguageRegistry {
        &self.registry
    }

    pub(crate) fn highlighter_for_path(&self, path: Option<&Path>) -> SyntaxLanguage {
        self.registry.highlighter_for_path(path)
    }

    pub(crate) fn language_for_path(&self, path: Option<&Path>) -> Option<&LanguageSpec> {
        self.registry.for_path(path)
    }

    pub(crate) fn workspace_root<'a>(&self, path: &'a Path) -> &'a Path {
        self.registry.workspace_root(path)
    }

    pub(crate) fn set_discovery_overlay(&mut self, overlay: Option<&Path>) {
        self.discovery_overlay = overlay.map(Path::to_path_buf);
        self.discovery_enabled = true;
    }

    pub(crate) fn reload_registry_from_home(&mut self) {
        self.registry = LanguageRegistry::from_process_home();
    }

    #[cfg(test)]
    pub(crate) fn apply_overlay(&mut self, text: &str) -> Result<(), LanguageRegistryError> {
        self.registry.apply_overlay(text)
    }

    pub(crate) fn replace_open_paths<'a>(&mut self, paths: impl Iterator<Item = &'a Path>) {
        self.open_paths.clear();
        if self.open_paths.try_reserve(MAX_OPEN_PATHS).is_err() {
            return;
        }
        for path in paths {
            if self.open_paths.len() == MAX_OPEN_PATHS {
                break;
            }
            self.open_paths.push(path.to_path_buf());
        }
    }

    #[cfg(test)]
    pub(crate) fn warm_count(&self) -> usize {
        self.slots.len()
    }

    /// Running servers' process IDs, each with its executable's file name.
    pub(crate) fn server_processes(&self) -> impl Iterator<Item = (u32, &str)> {
        let models = self.slots.iter().map(|slot| &slot.model);
        models
            .chain([&self.fallback])
            .filter_map(|model| Some((model.server_process_id()?, model.server_name())))
    }

    #[cfg(test)]
    pub(crate) fn discovery_dispatches_for_test(&self) -> u64 {
        self.discovery_dispatches
    }

    #[cfg(test)]
    pub(crate) fn has_server_id(&self, server_id: &str) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.server_id.as_ref() == server_id)
    }

    pub(crate) fn sync<F>(
        &mut self,
        input: Option<RustDocumentInput>,
        wake_factory: F,
    ) -> LanguageEffect
    where
        F: FnOnce(LanguageWake) -> ProcessWake,
    {
        let now = Instant::now();
        // This sync starts the active slot itself, with the current input.
        let _ = self.admit_discoveries();
        self.reap_idle(now);
        let needed = self.needed_keys(input.as_ref());
        self.detach_unneeded(&needed, now);
        let Some(input) = input else {
            self.active_index = None;
            return LanguageEffect::default();
        };
        let Some(spec) = self.registry.for_path(Some(input.path())).cloned() else {
            self.active_index = None;
            return LanguageEffect::default();
        };
        let Some(server_id) = spec.server_id() else {
            self.active_index = None;
            return LanguageEffect::default();
        };
        let workspace_root = input.workspace_root();
        let Some(index) = self.ensure_slot(workspace_root, server_id, now) else {
            self.active_index = None;
            return LanguageEffect::default();
        };
        self.prepare_slot(index, &spec, now);
        self.active_index = Some(index);
        let Some((input, wake_factory)) = self.discover(index, &spec, input, wake_factory) else {
            return LanguageEffect::default();
        };
        self.slots[index].model.sync(Some(input), wake_factory)
    }

    pub(crate) fn poll(&mut self, wake: LanguageWake) -> LanguageEffect {
        let mut effect = LanguageEffect::default();
        if let Some((index, input, session_wake)) = self.admit_discoveries()
            && let Some(slot) = self.slots.get_mut(index)
        {
            // Built for this generation when discovery was queued.
            effect.merge(slot.model.sync(Some(input), move |_| session_wake));
        }
        if self.slots.is_empty() {
            effect.merge(self.fallback.poll(wake));
            self.reap_idle(Instant::now());
            return effect;
        }
        let matching = wake.generation();
        for slot in &mut self.slots {
            if let Some(generation) = slot.model.current_generation() {
                let slot_wake = if generation == matching {
                    wake
                } else {
                    LanguageWake::new(generation)
                };
                effect.merge(slot.model.poll(slot_wake));
            }
        }
        if self.fallback.has_session() {
            effect.merge(self.fallback.poll(wake));
        }
        self.reap_idle(Instant::now());
        effect
    }

    pub(crate) fn shutdown(&mut self) -> RustDiagnosticsSnapshot {
        self.discovery_worker = None;
        for slot in &mut self.slots {
            let _ = slot.model.shutdown();
        }
        self.slots.clear();
        self.active_index = None;
        self.fallback.shutdown()
    }

    fn active_model(&self) -> &RustDiagnostics {
        self.active_index
            .and_then(|index| self.slots.get(index))
            .map_or(&self.fallback, |slot| &slot.model)
    }

    fn active_model_mut(&mut self) -> &mut RustDiagnostics {
        if let Some(index) = self.active_index
            && let Some(slot) = self.slots.get_mut(index)
        {
            return &mut slot.model;
        }
        &mut self.fallback
    }

    fn needed_keys(&self, input: Option<&RustDocumentInput>) -> Vec<(PathBuf, Box<str>)> {
        let mut keys = Vec::new();
        let mut push = |path: &Path, workspace: Option<&Path>| {
            let Some(spec) = self.registry.for_path(Some(path)) else {
                return;
            };
            let Some(server_id) = spec.server_id() else {
                return;
            };
            let workspace_root = workspace.map_or_else(
                || self.registry.workspace_root(path).to_path_buf(),
                Path::to_path_buf,
            );
            if keys.iter().any(|key: &(PathBuf, Box<str>)| {
                key.0 == workspace_root && key.1.as_ref() == server_id
            }) {
                return;
            }
            keys.push((workspace_root, Box::from(server_id)));
        };
        for path in &self.open_paths {
            let workspace = input
                .filter(|input| input.path() == path)
                .map(RustDocumentInput::workspace_root);
            push(path, workspace);
        }
        if let Some(input) = input {
            push(input.path(), Some(input.workspace_root()));
        }
        keys
    }

    fn detach_unneeded(&mut self, needed: &[(PathBuf, Box<str>)], now: Instant) {
        for slot in &mut self.slots {
            let keep = needed.iter().any(|(root, server_id)| {
                *root == slot.workspace_root && server_id.as_ref() == slot.server_id.as_ref()
            });
            if keep {
                slot.attached = true;
                slot.idle_since = None;
            } else if slot.attached {
                slot.attached = false;
                slot.idle_since = Some(now);
            }
        }
    }

    fn ensure_slot(
        &mut self,
        workspace_root: &Path,
        server_id: &str,
        now: Instant,
    ) -> Option<usize> {
        if let Some(index) = self.find_slot(workspace_root, server_id) {
            return Some(index);
        }
        while self.slots.len() >= MAX_WARM_SERVERS {
            if !self.evict_one() {
                return None;
            }
        }
        if self.slots.try_reserve(1).is_err() {
            return None;
        }
        let model = if self.slots.is_empty() {
            mem::take(&mut self.fallback)
        } else {
            RustDiagnostics::default()
        };
        self.slots.push(Slot {
            workspace_root: workspace_root.to_path_buf(),
            server_id: Box::from(server_id),
            last_used: now,
            attached: true,
            idle_since: None,
            discovery: Discovery::Needed,
            model,
        });
        Some(self.slots.len().saturating_sub(1))
    }

    fn find_slot(&self, workspace_root: &Path, server_id: &str) -> Option<usize> {
        self.slots.iter().position(|slot| {
            slot.workspace_root == workspace_root && slot.server_id.as_ref() == server_id
        })
    }

    fn prepare_slot(&mut self, index: usize, spec: &LanguageSpec, now: Instant) {
        let language_id = spec.lsp_language_id().unwrap_or(spec.id());
        let slot = &mut self.slots[index];
        slot.model.configure_protocol(language_id, spec.arguments());
        slot.attached = true;
        slot.idle_since = None;
        slot.last_used = now;
    }

    /// Queues discovery for a slot that has none, so the event thread never
    /// probes binaries. Hands the input back once the slot may start.
    fn discover<F>(
        &mut self,
        index: usize,
        spec: &LanguageSpec,
        input: RustDocumentInput,
        wake_factory: F,
    ) -> Option<(RustDocumentInput, F)>
    where
        F: FnOnce(LanguageWake) -> ProcessWake,
    {
        if !self.discovery_enabled {
            return Some((input, wake_factory));
        }
        match &mut self.slots[index].discovery {
            Discovery::Done => return Some((input, wake_factory)),
            Discovery::Pending { input: newest, .. } => {
                *newest = input;
                return None;
            }
            Discovery::Needed => {}
        }
        #[cfg(not(test))]
        let environment = DiscoveryEnv::from_process(spec.env_override());
        #[cfg(test)]
        let environment = self
            .discovery_env
            .clone()
            .unwrap_or_else(|| DiscoveryEnv::from_process(spec.env_override()));
        // A pinned path needs no search, so it binds at once, as before.
        if let Some(pinned) = environment.pinned.as_ref() {
            let slot = &mut self.slots[index];
            slot.model
                .bind_discovered_server(Some(PathBuf::from(pinned)));
            slot.discovery = Discovery::Done;
            return Some((input, wake_factory));
        }
        if self.discovery_worker.is_none() {
            self.discovery_worker = DiscoveryWorker::spawn(Arc::clone(&self.discover));
        }
        // A thread that failed to spawn is retried on a later sync.
        let worker = self.discovery_worker.as_ref()?;
        let Some(token) = self.discovery_token.checked_add(1) else {
            self.slots[index].discovery = Discovery::Done;
            return Some((input, wake_factory));
        };
        let slot = &self.slots[index];
        let wake = wake_factory(slot.model.next_session_wake());
        let job = DiscoveryJob {
            token,
            workspace_root: slot.workspace_root.clone(),
            server_id: slot.server_id.clone(),
            binaries: spec.binaries().to_vec(),
            overlay: self.discovery_overlay.clone(),
            environment,
            wake: Arc::clone(&wake),
        };
        match worker.jobs.try_send(job) {
            Ok(()) => {
                self.discovery_token = token;
                self.slots[index].discovery = Discovery::Pending { token, wake, input };
                #[cfg(test)]
                {
                    self.discovery_dispatches = self.discovery_dispatches.saturating_add(1);
                }
            }
            // A full queue retries on a later sync.
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => self.replace_discovery_worker(),
        }
        None
    }

    /// A dead thread lost its jobs: queue them again on a new one.
    fn replace_discovery_worker(&mut self) {
        self.discovery_worker = None;
        for slot in &mut self.slots {
            if matches!(slot.discovery, Discovery::Pending { .. }) {
                slot.discovery = Discovery::Needed;
            }
        }
    }

    /// Binds each finished discovery to the slot that asked for it, if that
    /// slot still exists unchanged; drops the rest. Returns the active slot's
    /// start: its index, newest input and session wake.
    fn admit_discoveries(&mut self) -> Option<(usize, RustDocumentInput, ProcessWake)> {
        let worker = self.discovery_worker.as_ref()?;
        let mut start = None;
        let mut dead = false;
        loop {
            let result = match worker.results.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    dead = true;
                    break;
                }
            };
            let current = self.slots.iter().position(|slot| {
                slot.workspace_root == result.workspace_root
                    && slot.server_id == result.server_id
                    && matches!(
                        slot.discovery,
                        Discovery::Pending { token, .. } if token == result.token
                    )
            });
            let Some(index) = current else {
                #[cfg(test)]
                {
                    self.stale_discoveries = self.stale_discoveries.saturating_add(1);
                }
                continue;
            };
            let slot = &mut self.slots[index];
            let discovery = mem::replace(&mut slot.discovery, Discovery::Done);
            slot.model.bind_discovered_server(result.server);
            if let Discovery::Pending { wake, input, .. } = discovery
                && self.active_index == Some(index)
            {
                start = Some((index, input, wake));
            }
        }
        if dead {
            self.replace_discovery_worker();
        }
        start
    }

    fn evict_one(&mut self) -> bool {
        let Some(index) = self
            .slots
            .iter()
            .enumerate()
            .min_by_key(|(_, slot)| (slot.attached, slot.last_used))
            .map(|(index, _)| index)
        else {
            return false;
        };
        let mut slot = self.slots.remove(index);
        let _ = slot.model.shutdown();
        if let Some(active) = self.active_index {
            if active == index {
                self.active_index = None;
            } else if active > index {
                self.active_index = Some(active.saturating_sub(1));
            }
        }
        true
    }

    fn reap_idle(&mut self, now: Instant) {
        let mut index = 0;
        while index < self.slots.len() {
            let expired = {
                let slot = &self.slots[index];
                !slot.attached
                    && slot.idle_since.is_some_and(|idle_since| {
                        now.saturating_duration_since(idle_since) >= IDLE_TTL
                    })
            };
            if expired {
                let mut slot = self.slots.remove(index);
                let _ = slot.model.shutdown();
                if let Some(active) = self.active_index {
                    if active == index {
                        self.active_index = None;
                    } else if active > index {
                        self.active_index = Some(active.saturating_sub(1));
                    }
                }
            } else {
                index = index.saturating_add(1);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_slot_for_test(&mut self, workspace_root: &Path, server_id: &str) -> bool {
        self.ensure_slot(workspace_root, server_id, Instant::now())
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn mark_unattached_for_test(&mut self, server_id: &str) {
        let now = Instant::now();
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.server_id.as_ref() == server_id)
        {
            slot.attached = false;
            slot.idle_since = Some(now);
            slot.last_used = now.checked_sub(Duration::from_hours(1)).unwrap_or(now);
        }
    }

    #[cfg(test)]
    pub(crate) fn expire_idle_for_test(&mut self, server_id: &str) {
        let now = Instant::now();
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.server_id.as_ref() == server_id)
        {
            slot.attached = false;
            slot.idle_since = now
                .checked_sub(IDLE_TTL + Duration::from_secs(1))
                .or(Some(now));
        }
        self.reap_idle(now);
    }
}

#[cfg(test)]
impl LanguageServices {
    pub(crate) fn install_for_test(
        &mut self,
        input: RustDocumentInput,
        params: &serde_json::value::RawValue,
        executable: &Path,
    ) -> Result<(), crate::rust_diagnostics::RustDiagnosticsError> {
        self.active_model_mut()
            .install_for_test(input, params, executable)
    }

    pub(crate) fn install_completion_for_test(
        &mut self,
        request_id: u32,
        identity: crate::rust_diagnostics::LanguageIdentity,
        result: &serde_json::value::RawValue,
    ) -> Result<(), crate::rust_diagnostics::RustDiagnosticsError> {
        self.active_model_mut()
            .install_completion_for_test(request_id, identity, result)
    }

    pub(crate) fn install_navigation_for_test(
        &mut self,
        identity: crate::rust_diagnostics::LanguageIdentity,
        kind: crate::rust_diagnostics::NavigationRequestKind,
        result: &serde_json::value::RawValue,
    ) -> Result<(), crate::rust_navigation::NavigationError> {
        self.active_model_mut()
            .install_navigation_for_test(identity, kind, result)
    }

    pub(crate) fn install_symbols_for_test(
        &mut self,
        identity: crate::rust_diagnostics::LanguageIdentity,
        kind: crate::rust_symbols::SymbolRequestKind,
        result: &serde_json::value::RawValue,
    ) -> Result<(), crate::rust_symbols::SymbolError> {
        self.active_model_mut()
            .install_symbols_for_test(identity, kind, result)
    }

    pub(crate) fn force_continuation_once_for_test(&mut self) {
        self.active_model_mut().force_continuation_once_for_test();
    }

    pub(crate) fn stage_workspace_edit_preparation_for_test(
        &mut self,
        identity: crate::rust_diagnostics::WorkspaceEditIdentity,
        workspace_root: &Path,
        document_uri: &str,
        result: &serde_json::value::RawValue,
    ) -> Result<(), crate::rust_workspace_edit::WorkspaceEditError> {
        self.active_model_mut()
            .stage_workspace_edit_preparation_for_test(
                identity,
                workspace_root,
                document_uri,
                result,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rust_diagnostics::RustDiagnosticsError;
    use std::{
        error::Error,
        fs,
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
            mpsc::Sender,
        },
    };

    // Bounds a hang only; no assertion depends on elapsed time.
    const BACKSTOP: Duration = Duration::from_secs(20);

    #[test]
    fn sixth_identity_evicts_the_idle_oldest() {
        let mut services = LanguageServices::default();
        let root = Path::new("/tmp/alpine-phase2-pool");
        for server_id in ["rust", "python", "cpp", "java", "typescript"] {
            assert!(services.insert_slot_for_test(root, server_id));
        }
        assert_eq!(services.warm_count(), MAX_WARM_SERVERS);
        services.mark_unattached_for_test("java");
        assert!(services.insert_slot_for_test(root, "markdown-idle"));
        assert_eq!(services.warm_count(), MAX_WARM_SERVERS);
        assert!(!services.has_server_id("java"));
        assert!(services.has_server_id("markdown-idle"));
        assert!(services.has_server_id("rust"));
        assert!(services.has_server_id("python"));
    }

    #[test]
    fn idle_ttl_drops_unattached_slots_without_hitting_the_cap() {
        let mut services = LanguageServices::default();
        let root = Path::new("/tmp/alpine-phase2-idle");
        assert!(services.insert_slot_for_test(root, "python"));
        assert!(services.insert_slot_for_test(root, "rust"));
        assert_eq!(services.warm_count(), 2);
        services.expire_idle_for_test("python");
        assert_eq!(services.warm_count(), 1);
        assert!(services.has_server_id("rust"));
        assert!(!services.has_server_id("python"));
    }

    #[test]
    fn typescript_and_javascript_share_one_server_identity() {
        let registry = LanguageRegistry::compiled();
        assert_eq!(
            registry.get("javascript").and_then(LanguageSpec::server_id),
            Some("typescript")
        );
        let mut services = LanguageServices::default();
        let root = Path::new("/tmp/alpine-phase2-ts");
        assert!(services.insert_slot_for_test(root, "typescript"));
        assert!(services.insert_slot_for_test(root, "typescript"));
        assert_eq!(services.warm_count(), 1);
    }

    #[test]
    fn overlay_disable_removes_lookup_from_the_live_registry() {
        let mut services = LanguageServices::default();
        assert!(services.registry().get("python").is_some());
        assert!(
            services
                .language_for_path(Some(Path::new("Main.java")))
                .is_some()
        );
        let _ = services.apply_overlay(r#"disabled = ["java"]"#);
        assert!(
            services
                .language_for_path(Some(Path::new("Main.java")))
                .is_none()
        );
        assert_eq!(
            services.highlighter_for_path(Some(Path::new("Main.java"))),
            SyntaxLanguage::PlainText
        );
        assert!(
            services
                .language_for_path(Some(Path::new("main.py")))
                .is_some()
        );
    }

    #[test]
    fn open_tabs_keep_per_file_workspace_roots() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("alpine-phase2-roots-{}", std::process::id()));
        let rust_root = root.join("rs");
        let python_root = root.join("py");
        std::fs::create_dir_all(rust_root.join("src"))?;
        std::fs::create_dir_all(&python_root)?;
        std::fs::write(rust_root.join("Cargo.toml"), "[package]\nname = \"x\"\n")?;
        std::fs::write(
            python_root.join("pyproject.toml"),
            "[project]\nname = \"x\"\n",
        )?;
        let rust_file = rust_root.join("src/lib.rs");
        let python_file = python_root.join("main.py");
        std::fs::write(&rust_file, "fn x() {}\n")?;
        std::fs::write(&python_file, "x = 1\n")?;

        let mut services = LanguageServices::default();
        services.replace_open_paths([&rust_file, &python_file].into_iter().map(PathBuf::as_path));
        let identity = crate::rust_diagnostics::LanguageIdentity {
            workspace_id: 1,
            workspace_revision: 1,
            document_id: 1,
            document_revision: 1,
            buffer_revision: 1,
            selection_revision: 1,
        };
        let snapshot = alpine_text::Buffer::new("fn x() {}\n").snapshot();
        let input = crate::rust_diagnostics::RustDocumentInput::new(
            &rust_file, &rust_root, identity, snapshot,
        );
        let keys = services.needed_keys(Some(&input));
        assert!(
            keys.iter()
                .any(|(root, server)| root == &rust_root && server.as_ref() == "rust")
        );
        assert!(
            keys.iter()
                .any(|(root, server)| root == &python_root && server.as_ref() == "python")
        );
        assert!(
            !keys
                .iter()
                .any(|(root, server)| root == &rust_root && server.as_ref() == "python")
        );
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    fn python_document(
        root: &Path,
    ) -> Result<(PathBuf, crate::rust_diagnostics::RustDocumentInput), Box<dyn std::error::Error>>
    {
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"x\"\n")?;
        let path = root.join("main.py");
        std::fs::write(&path, "value = 1\n")?;
        let identity = crate::rust_diagnostics::LanguageIdentity {
            workspace_id: 1,
            workspace_revision: 1,
            document_id: 1,
            document_revision: 1,
            buffer_revision: 1,
            selection_revision: 1,
        };
        let snapshot = alpine_text::Buffer::new("value = 1\n").snapshot();
        let input =
            crate::rust_diagnostics::RustDocumentInput::new(&path, root, identity, snapshot);
        Ok((path, input))
    }

    /// A discovery that blocks until the test releases it with an answer.
    struct Gate {
        started: Receiver<u64>,
        release: SyncSender<Option<PathBuf>>,
        finished: Arc<AtomicU64>,
    }

    fn gated_discovery() -> (Discover, Gate) {
        let (started_sender, started) = mpsc::channel();
        let (release, released) = mpsc::sync_channel::<Option<PathBuf>>(4);
        let released = Mutex::new(released);
        let finished = Arc::new(AtomicU64::new(0));
        let done = Arc::clone(&finished);
        let discover: Discover = Arc::new(move |job: &DiscoveryJob| {
            let _ = started_sender.send(job.token);
            let server = released
                .lock()
                .ok()
                .and_then(|released| released.recv_timeout(BACKSTOP).ok())
                .flatten();
            done.fetch_add(1, Ordering::SeqCst);
            server
        });
        let gate = Gate {
            started,
            release,
            finished,
        };
        (discover, gate)
    }

    fn wake_into(sender: &Sender<LanguageWake>) -> impl FnOnce(LanguageWake) -> ProcessWake {
        let sender = sender.clone();
        move |wake| {
            Arc::new(move || {
                let _ = sender.send(wake);
            })
        }
    }

    fn discovering_services(discover: Discover) -> LanguageServices {
        let mut services = LanguageServices::default();
        services.set_discovery_overlay(None);
        services.discover = discover;
        services
    }

    fn missing_server_status() -> String {
        RustDiagnosticsError::MissingServer.to_string()
    }

    fn scratch_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("alpine-phase2-{label}-{}", std::process::id()))
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn a_warm_slot_discovers_once_across_later_syncs() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("discover");
        let (_path, input) = python_document(&root)?;
        let runs = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&runs);
        let mut services = discovering_services(Arc::new(move |_: &DiscoveryJob| {
            counted.fetch_add(1, Ordering::SeqCst);
            None
        }));
        let (publish, wakes) = mpsc::channel();
        for _ in 0..32 {
            let _ = services.sync(Some(input.clone()), wake_into(&publish));
        }
        let _ = wakes.recv_timeout(BACKSTOP)?;
        for _ in 0..32 {
            let _ = services.sync(Some(input.clone()), wake_into(&publish));
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(services.discovery_dispatches, 1);
        assert_eq!(
            services.status_message().as_deref(),
            Some(missing_server_status().as_str())
        );
        let _ = services.shutdown();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn opening_a_document_probes_nothing_on_the_event_thread() -> Result<(), Box<dyn Error>> {
        crate::rust_diagnostics::reset_discover_binaries_calls();
        let root = scratch_root("offmain");
        let (_path, input) = python_document(&root)?;
        let mut services = LanguageServices::default();
        services.set_discovery_overlay(None);
        services.discovery_env = Some(DiscoveryEnv {
            path: Some(root.clone().into_os_string()),
            home: Some(root.join("home").into_os_string()),
            ..DiscoveryEnv::default()
        });
        let (publish, wakes) = mpsc::channel();
        let _ = services.sync(Some(input), wake_into(&publish));
        assert_eq!(
            crate::rust_diagnostics::discover_binaries_calls(),
            0,
            "opening a document probed for its server on the event thread"
        );
        let _ = wakes.recv_timeout(BACKSTOP)?;
        assert_eq!(crate::rust_diagnostics::discover_binaries_calls(), 0);
        assert_eq!(services.discovery_dispatches, 1);
        let _ = services.shutdown();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn a_blocked_discovery_never_blocks_sync_or_poll() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("blocked");
        let (_path, input) = python_document(&root)?;
        let (discover, gate) = gated_discovery();
        let mut services = discovering_services(discover);
        let (publish, wakes) = mpsc::channel();
        let _ = services.sync(Some(input.clone()), wake_into(&publish));
        let _ = gate.started.recv_timeout(BACKSTOP)?;
        for _ in 0..32 {
            let effect = services.sync(Some(input.clone()), wake_into(&publish));
            assert_eq!(effect, LanguageEffect::default());
            assert_eq!(
                services.poll(LanguageWake::new(1)),
                LanguageEffect::default()
            );
        }
        // Every call above returned while discovery was still blocked.
        assert_eq!(gate.finished.load(Ordering::SeqCst), 0);
        assert!(!services.has_session());
        assert_eq!(services.status_message(), None);

        gate.release.send(None)?;
        let wake = wakes.recv_timeout(BACKSTOP)?;
        assert!(services.poll(wake).visual_changed);
        assert_eq!(
            services.status_message().as_deref(),
            Some(missing_server_status().as_str())
        );
        let _ = services.shutdown();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn the_server_starts_when_its_discovery_arrives() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("arrives");
        let (_path, input) = python_document(&root)?;
        let bin = root.join("bin");
        fs::create_dir_all(&bin)?;
        std::os::unix::fs::symlink(
            crate::rust_diagnostics::tests::mock_executable(),
            bin.join("pylsp"),
        )?;
        let mut services = LanguageServices::default();
        services.set_discovery_overlay(None);
        services.discovery_env = Some(DiscoveryEnv {
            path: Some(bin.into_os_string()),
            home: Some(root.join("home").into_os_string()),
            ..DiscoveryEnv::default()
        });
        let (publish, wakes) = mpsc::channel();
        let _ = services.sync(Some(input.clone()), wake_into(&publish));
        assert_eq!(services.server_processes().count(), 0);

        assert!(!services.has_session());
        let wake = wakes.recv_timeout(BACKSTOP)?;
        assert!(services.poll(wake).visual_changed);
        assert!(services.has_session());
        assert_eq!(services.current_generation(), Some(wake.generation()));
        // The discovery wake now serves the session: drive it to diagnostics.
        while services.snapshot().diagnostic_publications == 0 {
            let wake = wakes.recv_timeout(BACKSTOP)?;
            if let Some(continuation) = services.poll(wake).continuation {
                publish.send(continuation)?;
            }
        }
        let _ = services.sync(Some(input), wake_into(&publish));
        let names: Vec<String> = services
            .server_processes()
            .map(|(_, name)| name.to_owned())
            .collect();
        assert_eq!(names, ["pylsp"]);
        assert!(!services.shutdown().active);
        assert_eq!(services.server_processes().count(), 0);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn a_result_for_a_replaced_or_evicted_slot_is_dropped() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("stale");
        let (_path, input) = python_document(&root)?;
        let (discover, gate) = gated_discovery();
        let mut services = discovering_services(discover);
        let (publish, wakes) = mpsc::channel();
        let server = || Some(crate::rust_diagnostics::tests::mock_executable().to_path_buf());

        let _ = services.sync(Some(input.clone()), wake_into(&publish));
        let first = gate.started.recv_timeout(BACKSTOP)?;
        services.mark_unattached_for_test("python");
        for server_id in ["rust", "cpp", "java", "typescript", "markdown-idle"] {
            assert!(services.insert_slot_for_test(&root, server_id));
        }
        assert!(!services.has_server_id("python"));
        // Same root and server id again: a new slot that queues a new job.
        let _ = services.sync(Some(input.clone()), wake_into(&publish));
        assert!(services.has_server_id("python"));

        gate.release.send(server())?;
        let _ = services.poll(wakes.recv_timeout(BACKSTOP)?);
        assert_eq!(services.stale_discoveries, 1);
        assert_eq!(services.server_processes().count(), 0);
        let second = gate.started.recv_timeout(BACKSTOP)?;
        assert_ne!(first, second);

        services.mark_unattached_for_test("python");
        assert!(services.insert_slot_for_test(&root, "go"));
        assert!(!services.has_server_id("python"));
        gate.release.send(server())?;
        let _ = services.poll(wakes.recv_timeout(BACKSTOP)?);
        assert_eq!(services.stale_discoveries, 2);
        assert_eq!(services.server_processes().count(), 0);
        let _ = services.shutdown();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn sessions(services: &LanguageServices) -> usize {
        services
            .slots
            .iter()
            .filter(|slot| slot.model.has_session())
            .count()
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn a_result_for_an_inactive_slot_waits_for_its_document() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("inactive");
        let (_path, input) = python_document(&root)?;
        let (discover, gate) = gated_discovery();
        let mut services = discovering_services(discover);
        let (publish, wakes) = mpsc::channel();
        let _ = services.sync(Some(input.clone()), wake_into(&publish));
        let _ = gate.started.recv_timeout(BACKSTOP)?;
        // Focus leaves the document while its discovery runs.
        let _ = services.sync(None, wake_into(&publish));
        let server = crate::rust_diagnostics::tests::mock_executable().to_path_buf();
        gate.release.send(Some(server))?;
        let _ = services.poll(wakes.recv_timeout(BACKSTOP)?);
        assert_eq!(sessions(&services), 0);
        assert_eq!(services.stale_discoveries, 0);

        let _ = services.sync(Some(input), wake_into(&publish));
        assert_eq!(sessions(&services), 1);
        let _ = services.shutdown();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn discovery_starts_the_server_with_the_newest_input() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("newest");
        let (path, input) = python_document(&root)?;
        let (discover, gate) = gated_discovery();
        let mut services = discovering_services(discover);
        let (publish, wakes) = mpsc::channel();
        let _ = services.sync(Some(input), wake_into(&publish));
        let _ = gate.started.recv_timeout(BACKSTOP)?;
        let identity = crate::rust_diagnostics::LanguageIdentity {
            workspace_id: 1,
            workspace_revision: 1,
            document_id: 1,
            document_revision: 9,
            buffer_revision: 9,
            selection_revision: 9,
        };
        let snapshot = alpine_text::Buffer::new("value = 2\n").snapshot();
        let newer =
            crate::rust_diagnostics::RustDocumentInput::new(&path, &root, identity, snapshot);
        let _ = services.sync(Some(newer), wake_into(&publish));

        let server = crate::rust_diagnostics::tests::mock_executable().to_path_buf();
        gate.release.send(Some(server))?;
        let _ = services.poll(wakes.recv_timeout(BACKSTOP)?);
        let started = services.active_model().session_identity_for_test();
        assert_eq!(started.map(|identity| identity.document_revision), Some(9));
        let _ = services.shutdown();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    /// Reports when the last copy of a wake is gone.
    struct DropSignal(Sender<&'static str>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            let _ = self.0.send("dropped");
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not emulate the discovery thread's host walk"
    )]
    fn shutdown_never_waits_for_a_blocked_discovery() -> Result<(), Box<dyn Error>> {
        let root = scratch_root("shutdown");
        let (_path, input) = python_document(&root)?;
        let (discover, gate) = gated_discovery();
        let mut services = discovering_services(discover);
        let (events, observed) = mpsc::channel();
        let _ = services.sync(Some(input), move |_| {
            let signal = DropSignal(events.clone());
            Arc::new(move || {
                let _ = signal.0.send("publish");
            })
        });
        let _ = gate.started.recv_timeout(BACKSTOP)?;
        let _ = services.shutdown();
        // Shutdown returned while discovery was provably still blocked.
        assert_eq!(gate.finished.load(Ordering::SeqCst), 0);

        gate.release.send(Some(PathBuf::from("/unused")))?;
        // The thread drops its job without waking anyone, then exits.
        assert_eq!(observed.recv_timeout(BACKSTOP)?, "dropped");
        assert_eq!(gate.finished.load(Ordering::SeqCst), 1);
        assert_eq!(services.server_processes().count(), 0);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
