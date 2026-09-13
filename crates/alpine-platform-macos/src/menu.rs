//! The application menu bar.
//!
//! Without a main menu macOS shows only the process name and offers no Quit,
//! Hide, Close or Minimize, so the application does not behave like one. This
//! installs the standard structure and routes each item through `AppKit`'s own
//! selectors, which travel the responder chain to whichever view is focused.
//!
//! Items whose selector no responder implements are disabled by `AppKit` rather
//! than silently doing nothing, so the menu never claims a capability the
//! editor does not have.

use crate::MenuAction;
use objc2::runtime::AnyObject;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSModalResponseOK, NSOpenPanel,
    NSSavePanel,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use std::cell::RefCell;
use std::path::PathBuf;

/// Title shown in the leftmost menu and in the Dock.
pub(crate) const APPLICATION_NAME: &str = "Alpine Editor";

/// Receives a menu command whose file choice is already resolved.
pub(crate) type MenuHandler = Box<dyn FnMut(MenuAction) + 'static>;

struct Item {
    title: &'static str,
    selector: objc2::runtime::Sel,
    key: &'static str,
    /// Adds Shift to the default Command modifier.
    shift: bool,
    /// Routes to the editor's own target instead of the responder chain.
    owned: bool,
}

#[derive(Default)]
pub(crate) struct MenuTargetIvars {
    handler: RefCell<Option<MenuHandler>>,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - MenuTarget has no custom Drop implementation.
    // - The object is main-thread-only, matching AppKit menu dispatch.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetIvars]
    pub(crate) struct MenuTarget;

    // SAFETY: NSObjectProtocol adds no methods with unfulfilled invariants.
    unsafe impl NSObjectProtocol for MenuTarget {}

    // SAFETY: Each selector takes the standard single sender argument, which is
    // borrowed only for the call and never escapes. Every emitted value owns
    // its path, so no AppKit object crosses the boundary.
    impl MenuTarget {
        #[unsafe(method(alpineNewFile:))]
        fn new_file(&self, _sender: Option<&AnyObject>) {
            self.emit(MenuAction::NewFile);
        }

        #[unsafe(method(alpineOpenFile:))]
        fn open_file(&self, _sender: Option<&AnyObject>) {
            if let Some(path) = choose_path(self.mtm(), false) {
                self.emit(MenuAction::OpenPath(path));
            }
        }

        #[unsafe(method(alpineOpenFolder:))]
        fn open_folder(&self, _sender: Option<&AnyObject>) {
            if let Some(path) = choose_path(self.mtm(), true) {
                self.emit(MenuAction::OpenPath(path));
            }
        }

        #[unsafe(method(alpineSave:))]
        fn save(&self, _sender: Option<&AnyObject>) {
            self.emit(MenuAction::Save);
        }

        #[unsafe(method(alpineSaveAs:))]
        fn save_as(&self, _sender: Option<&AnyObject>) {
            if let Some(path) = choose_save_path(self.mtm()) {
                self.emit(MenuAction::SaveAsPath(path));
            }
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars::default());
        // SAFETY: NSObject's designated initializer, called once on the
        // allocation above.
        unsafe { msg_send![super(this), init] }
    }

    /// Installs the handler that receives every menu command.
    pub(crate) fn install_handler(&self, handler: MenuHandler) {
        if let Ok(mut installed) = self.ivars().handler.try_borrow_mut() {
            *installed = Some(handler);
        }
    }

    /// Drops the handler so later menu commands are ignored.
    pub(crate) fn clear_handler(&self) {
        if let Ok(mut installed) = self.ivars().handler.try_borrow_mut() {
            installed.take();
        }
    }

    /// Delivers one action, ignoring it when no handler is installed.
    ///
    /// A failed borrow means a menu command arrived while another was still
    /// dispatching, which reentrant modal panels can produce. Dropping the
    /// second is correct: the first is still mutating editor state.
    fn emit(&self, action: MenuAction) {
        if let Ok(mut handler) = self.ivars().handler.try_borrow_mut()
            && let Some(handler) = handler.as_mut()
        {
            handler(action);
        }
    }
}

/// Runs an open panel and returns the chosen path, or `None` when cancelled.
fn choose_path(mtm: MainThreadMarker, directories: bool) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(!directories);
    panel.setCanChooseDirectories(directories);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URL()?;
    url_path(&url)
}

/// Runs a save panel and returns the chosen path, or `None` when cancelled.
fn choose_save_path(mtm: MainThreadMarker) -> Option<PathBuf> {
    let panel = NSSavePanel::savePanel(mtm);
    panel.setCanCreateDirectories(true);
    panel.setNameFieldStringValue(&NSString::from_str("untitled"));
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URL()?;
    url_path(&url)
}

/// Converts a file URL to an owned path, rejecting anything not absolute.
fn url_path(url: &objc2_foundation::NSURL) -> Option<PathBuf> {
    let path = PathBuf::from(url.path()?.to_string());
    path.is_absolute().then_some(path)
}

fn separator(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    NSMenuItem::separatorItem(mtm)
}

fn item(mtm: MainThreadMarker, spec: &Item, target: &MenuTarget) -> Retained<NSMenuItem> {
    let title = NSString::from_str(spec.title);
    let key = NSString::from_str(spec.key);
    // SAFETY: Title and key equivalent are owned NSStrings that outlive the
    // call, and the selector is a compile-time constant from `sel!`.
    let entry = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &title,
            Some(spec.selector),
            &key,
        )
    };
    if spec.shift {
        entry.setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
        );
    }
    if spec.owned {
        // SAFETY: AppKit holds the target weakly, and the thread-local owns it
        // for the lifetime of the process, so it outlives the menu.
        unsafe { entry.setTarget(Some(target)) };
    }
    entry
}

fn submenu(
    mtm: MainThreadMarker,
    main: &NSMenu,
    title: &str,
    entries: &[Option<Item>],
    target: &MenuTarget,
) -> Retained<NSMenu> {
    let heading = NSString::from_str(title);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &heading);
    for entry in entries {
        match entry {
            Some(spec) => menu.addItem(&item(mtm, spec, target)),
            None => menu.addItem(&separator(mtm)),
        }
    }
    let anchor = NSMenuItem::new(mtm);
    anchor.setSubmenu(Some(&menu));
    main.addItem(&anchor);
    menu
}

/// Builds the leftmost application menu.
fn application_menu(mtm: MainThreadMarker, main: &NSMenu, target: &MenuTarget) {
    submenu(
        mtm,
        main,
        APPLICATION_NAME,
        &[
            Some(Item {
                title: "About Alpine Editor",
                selector: sel!(orderFrontStandardAboutPanel:),
                key: "",
                shift: false,
                owned: false,
            }),
            None,
            Some(Item {
                title: "Hide Alpine Editor",
                selector: sel!(hide:),
                key: "h",
                shift: false,
                owned: false,
            }),
            Some(Item {
                title: "Hide Others",
                selector: sel!(hideOtherApplications:),
                key: "h",
                shift: true,
                owned: false,
            }),
            Some(Item {
                title: "Show All",
                selector: sel!(unhideAllApplications:),
                key: "",
                shift: false,
                owned: false,
            }),
            None,
            Some(Item {
                title: "Quit Alpine Editor",
                selector: sel!(terminate:),
                key: "q",
                shift: false,
                owned: false,
            }),
        ],
        target,
    );
}

/// Builds the File menu.
///
/// Open and Save As route to the editor's own target, which runs the panel and
/// reports only the chosen path.
fn file_menu(mtm: MainThreadMarker, main: &NSMenu, target: &MenuTarget) {
    submenu(
        mtm,
        main,
        "File",
        &[
            Some(Item {
                title: "New File",
                selector: sel!(alpineNewFile:),
                key: "n",
                shift: false,
                owned: true,
            }),
            None,
            Some(Item {
                title: "Open\u{2026}",
                selector: sel!(alpineOpenFile:),
                key: "o",
                shift: false,
                owned: true,
            }),
            Some(Item {
                title: "Open Folder\u{2026}",
                selector: sel!(alpineOpenFolder:),
                key: "o",
                shift: true,
                owned: true,
            }),
            None,
            Some(Item {
                title: "Save",
                selector: sel!(alpineSave:),
                key: "s",
                shift: false,
                owned: true,
            }),
            Some(Item {
                title: "Save As\u{2026}",
                selector: sel!(alpineSaveAs:),
                key: "s",
                shift: true,
                owned: true,
            }),
            None,
            Some(Item {
                title: "Close Window",
                selector: sel!(performClose:),
                key: "w",
                shift: false,
                owned: false,
            }),
        ],
        target,
    );
}

/// Builds the Edit menu.
///
/// These selectors travel the responder chain, so `AppKit` disables any the
/// focused responder does not implement rather than failing silently.
fn edit_menu(mtm: MainThreadMarker, main: &NSMenu, target: &MenuTarget) {
    submenu(
        mtm,
        main,
        "Edit",
        &[
            Some(Item {
                title: "Undo",
                selector: sel!(undo:),
                key: "z",
                shift: false,
                owned: false,
            }),
            Some(Item {
                title: "Redo",
                selector: sel!(redo:),
                key: "z",
                shift: true,
                owned: false,
            }),
            None,
            Some(Item {
                title: "Cut",
                selector: sel!(cut:),
                key: "x",
                shift: false,
                owned: false,
            }),
            Some(Item {
                title: "Copy",
                selector: sel!(copy:),
                key: "c",
                shift: false,
                owned: false,
            }),
            Some(Item {
                title: "Paste",
                selector: sel!(paste:),
                key: "v",
                shift: false,
                owned: false,
            }),
            Some(Item {
                title: "Select All",
                selector: sel!(selectAll:),
                key: "a",
                shift: false,
                owned: false,
            }),
        ],
        target,
    );
}

/// Builds the Window menu and returns it so it can be registered.
fn window_menu(mtm: MainThreadMarker, main: &NSMenu, target: &MenuTarget) -> Retained<NSMenu> {
    submenu(
        mtm,
        main,
        "Window",
        &[
            Some(Item {
                title: "Minimize",
                selector: sel!(performMiniaturize:),
                key: "m",
                shift: false,
                owned: false,
            }),
            Some(Item {
                title: "Zoom",
                selector: sel!(performZoom:),
                key: "",
                shift: false,
                owned: false,
            }),
        ],
        target,
    )
}

thread_local! {
    /// Owns the menu target, which `AppKit` only references weakly.
    static MENU_TARGET: RefCell<Option<Retained<MenuTarget>>> = const { RefCell::new(None) };
}

/// Returns the process-wide menu target, creating it on first use.
fn shared_target(mtm: MainThreadMarker) -> Option<Retained<MenuTarget>> {
    MENU_TARGET.with(|cell| {
        let mut cell = cell.try_borrow_mut().ok()?;
        Some(cell.get_or_insert_with(|| MenuTarget::new(mtm)).clone())
    })
}

/// Routes every later menu command to `handler`, replacing any previous one.
///
/// Creates the target when it does not exist yet, so this does not depend on
/// the menu having been installed first. Returns false only when the target is
/// already borrowed, which the caller treats as the surface failure it is
/// rather than silently losing every menu command.
pub(crate) fn install_handler(mtm: MainThreadMarker, handler: MenuHandler) -> bool {
    let Some(target) = shared_target(mtm) else {
        return false;
    };
    target.install_handler(handler);
    true
}

/// Stops delivering menu commands to the installed handler.
pub(crate) fn clear_handler() {
    MENU_TARGET.with(|cell| {
        if let Ok(cell) = cell.try_borrow()
            && let Some(target) = cell.as_ref()
        {
            target.clear_handler();
        }
    });
}

/// Installs the main menu, replacing any menu already set.
///
/// Idempotent: calling it again rebuilds the same structure against the same
/// target, so an installed handler survives the rebuild.
pub(crate) fn install(application: &NSApplication, mtm: MainThreadMarker) {
    let Some(target) = shared_target(mtm) else {
        return;
    };
    let main = NSMenu::new(mtm);
    application_menu(mtm, &main, &target);
    file_menu(mtm, &main, &target);
    edit_menu(mtm, &main, &target);
    let windows = window_menu(mtm, &main, &target);
    application.setMainMenu(Some(&main));
    application.setWindowsMenu(Some(&windows));
}

#[cfg(test)]
mod tests {
    use super::APPLICATION_NAME;

    #[test]
    fn the_application_name_is_the_product_name_not_the_executable() {
        // With no main menu macOS falls back to the executable, which shows as
        // "alpine-editor" in the menu bar.
        assert_eq!(APPLICATION_NAME, "Alpine Editor");
        assert!(!APPLICATION_NAME.contains('-'));
    }
}
