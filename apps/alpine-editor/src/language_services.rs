//! Bounded language-server pool keyed by workspace and server identity.

use std::{
    env, mem,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(test)]
use crate::language_registry::LanguageRegistryError;
use crate::{
    language_registry::{LanguageRegistry, LanguageSpec},
    lsp_process::ProcessWake,
    rust_diagnostics::{
        LanguageEffect, LanguageWake, RustDiagnostics, RustDiagnosticsSnapshot, RustDocumentInput,
        discover_binaries,
    },
    syntax::SyntaxLanguage,
};

/// Hard concurrency cap: a sixth identity evicts rather than failing the open.
pub(crate) const MAX_WARM_SERVERS: usize = 5;
const MAX_OPEN_PATHS: usize = 32;
const IDLE_TTL: Duration = Duration::from_mins(1);

struct Slot {
    workspace_root: PathBuf,
    server_id: Box<str>,
    last_used: Instant,
    attached: bool,
    idle_since: Option<Instant>,
    discovered: bool,
    model: RustDiagnostics,
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
        self.slots[index].model.sync(Some(input), wake_factory)
    }

    pub(crate) fn poll(&mut self, wake: LanguageWake) -> LanguageEffect {
        let mut effect = LanguageEffect::default();
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
            discovered: false,
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
        if self.discovery_enabled && !self.slots[index].discovered {
            let pinned = spec.env_override().and_then(env::var_os);
            let discovered =
                discover_binaries(spec.binaries(), pinned, self.discovery_overlay.as_deref());
            self.slots[index].model.bind_discovered_server(discovered);
            self.slots[index].discovered = true;
        }
        let slot = &mut self.slots[index];
        slot.model.configure_protocol(language_id, spec.arguments());
        slot.attached = true;
        slot.idle_since = None;
        slot.last_used = now;
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
}
