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

use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

/// Title shown in the leftmost menu and in the Dock.
pub(crate) const APPLICATION_NAME: &str = "Alpine Editor";

struct Item {
    title: &'static str,
    selector: objc2::runtime::Sel,
    key: &'static str,
    /// Adds Shift to the default Command modifier.
    shift: bool,
}

fn separator(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    NSMenuItem::separatorItem(mtm)
}

fn item(mtm: MainThreadMarker, spec: &Item) -> Retained<NSMenuItem> {
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
    entry
}

fn submenu(
    mtm: MainThreadMarker,
    main: &NSMenu,
    title: &str,
    entries: &[Option<Item>],
) -> Retained<NSMenu> {
    let heading = NSString::from_str(title);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &heading);
    for entry in entries {
        match entry {
            Some(spec) => menu.addItem(&item(mtm, spec)),
            None => menu.addItem(&separator(mtm)),
        }
    }
    let anchor = NSMenuItem::new(mtm);
    anchor.setSubmenu(Some(&menu));
    main.addItem(&anchor);
    menu
}

/// Builds the leftmost application menu.
fn application_menu(mtm: MainThreadMarker, main: &NSMenu) {
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
            }),
            None,
            Some(Item {
                title: "Hide Alpine Editor",
                selector: sel!(hide:),
                key: "h",
                shift: false,
            }),
            Some(Item {
                title: "Hide Others",
                selector: sel!(hideOtherApplications:),
                key: "h",
                shift: true,
            }),
            Some(Item {
                title: "Show All",
                selector: sel!(unhideAllApplications:),
                key: "",
                shift: false,
            }),
            None,
            Some(Item {
                title: "Quit Alpine Editor",
                selector: sel!(terminate:),
                key: "q",
                shift: false,
            }),
        ],
    );
}

/// Builds the File menu.
fn file_menu(mtm: MainThreadMarker, main: &NSMenu) {
    submenu(
        mtm,
        main,
        "File",
        &[Some(Item {
            title: "Close Window",
            selector: sel!(performClose:),
            key: "w",
            shift: false,
        })],
    );
}

/// Builds the Edit menu.
///
/// These selectors travel the responder chain, so `AppKit` disables any the
/// focused responder does not implement rather than failing silently.
fn edit_menu(mtm: MainThreadMarker, main: &NSMenu) {
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
            }),
            Some(Item {
                title: "Redo",
                selector: sel!(redo:),
                key: "z",
                shift: true,
            }),
            None,
            Some(Item {
                title: "Cut",
                selector: sel!(cut:),
                key: "x",
                shift: false,
            }),
            Some(Item {
                title: "Copy",
                selector: sel!(copy:),
                key: "c",
                shift: false,
            }),
            Some(Item {
                title: "Paste",
                selector: sel!(paste:),
                key: "v",
                shift: false,
            }),
            Some(Item {
                title: "Select All",
                selector: sel!(selectAll:),
                key: "a",
                shift: false,
            }),
        ],
    );
}

/// Builds the Window menu and returns it so it can be registered.
fn window_menu(mtm: MainThreadMarker, main: &NSMenu) -> Retained<NSMenu> {
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
            }),
            Some(Item {
                title: "Zoom",
                selector: sel!(performZoom:),
                key: "",
                shift: false,
            }),
        ],
    )
}

/// Installs the main menu, replacing any menu already set.
///
/// Idempotent: calling it again rebuilds the same structure.
pub(crate) fn install(application: &NSApplication, mtm: MainThreadMarker) {
    let main = NSMenu::new(mtm);
    application_menu(mtm, &main);
    file_menu(mtm, &main);
    edit_menu(mtm, &main);
    let windows = window_menu(mtm, &main);
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
