//! Compiled, validated Studio settings without runtime registration.

use std::{error::Error, fmt};

use alpine_core::LinearRgba;
use alpine_platform_macos::Modifiers;

use crate::{commands::StudioCommand, syntax::SyntaxClass};

pub(crate) const KEY_A: u16 = 0;
pub(crate) const KEY_S: u16 = 1;
pub(crate) const KEY_F: u16 = 3;
pub(crate) const KEY_Z: u16 = 6;
pub(crate) const KEY_W: u16 = 13;
pub(crate) const KEY_E: u16 = 14;
pub(crate) const KEY_RIGHT_BRACKET: u16 = 30;
pub(crate) const KEY_LEFT_BRACKET: u16 = 33;
pub(crate) const KEY_P: u16 = 35;
pub(crate) const KEY_RETURN: u16 = 36;
pub(crate) const KEY_TAB: u16 = 48;
pub(crate) const KEY_DELETE_BACKWARD: u16 = 51;
pub(crate) const KEY_ESCAPE: u16 = 53;
pub(crate) const KEY_HOME: u16 = 115;
pub(crate) const KEY_DELETE_FORWARD: u16 = 117;
pub(crate) const KEY_END: u16 = 119;
pub(crate) const KEY_LEFT: u16 = 123;
pub(crate) const KEY_RIGHT: u16 = 124;
pub(crate) const KEY_DOWN: u16 = 125;
pub(crate) const KEY_UP: u16 = 126;

pub(crate) const FONT_FAMILY: u64 = 1;
pub(crate) const FONT_NAME: &str = "Menlo-Regular";
pub(crate) const FONT_SIZE: f32 = 15.0;
pub(crate) const FONT_SCALE: f32 = 2.0;
pub(crate) const LINE_HEIGHT: f32 = 22.0;
pub(crate) const TAB_COLUMNS: u32 = 4;

const COMMAND_SHIFT: u8 = Modifiers::COMMAND.saturating_add(Modifiers::SHIFT);
const COMMAND_OPTION: u8 = Modifiers::COMMAND.saturating_add(Modifiers::OPTION);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EditorSettings {
    pub(crate) font_family: u64,
    pub(crate) font_name: &'static str,
    pub(crate) font_size: f32,
    pub(crate) font_scale: f32,
    pub(crate) line_height: f32,
    pub(crate) tab_columns: u32,
}

impl EditorSettings {
    pub(crate) const COMPILED: Self = Self {
        font_family: FONT_FAMILY,
        font_name: FONT_NAME,
        font_size: FONT_SIZE,
        font_scale: FONT_SCALE,
        line_height: LINE_HEIGHT,
        tab_columns: TAB_COLUMNS,
    };

    fn validate(self) -> Result<(), SettingsError> {
        validate_metric("font size", self.font_size)?;
        validate_metric("font scale", self.font_scale)?;
        validate_metric("line height", self.line_height)?;
        if self.font_family == 0 {
            return Err(SettingsError::InvalidFontFamily);
        }
        if self.font_name.is_empty() {
            return Err(SettingsError::EmptyFontName);
        }
        if self.tab_columns == 0 {
            return Err(SettingsError::InvalidTabColumns);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SyntaxTheme {
    comment: LinearRgba,
    keyword: LinearRgba,
    string: LinearRgba,
    number: LinearRgba,
    type_name: LinearRgba,
    property: LinearRgba,
    heading: LinearRgba,
    code: LinearRgba,
}

impl SyntaxTheme {
    pub(crate) const fn color(self, class: SyntaxClass) -> LinearRgba {
        match class {
            SyntaxClass::Comment => self.comment,
            SyntaxClass::Keyword => self.keyword,
            SyntaxClass::String => self.string,
            SyntaxClass::Number => self.number,
            SyntaxClass::Type => self.type_name,
            SyntaxClass::Property => self.property,
            SyntaxClass::Heading => self.heading,
            SyntaxClass::Code => self.code,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StudioTheme {
    pub(crate) clear: LinearRgba,
    pub(crate) background: LinearRgba,
    pub(crate) editor_background: LinearRgba,
    pub(crate) selection: LinearRgba,
    pub(crate) text: LinearRgba,
    pub(crate) caret: LinearRgba,
    pub(crate) status_background: LinearRgba,
    pub(crate) sidebar_background: LinearRgba,
    pub(crate) active_row: LinearRgba,
    pub(crate) tab_background: LinearRgba,
    pub(crate) active_tab: LinearRgba,
    pub(crate) find_match: LinearRgba,
    pub(crate) find_background: LinearRgba,
    pub(crate) quick_open_background: LinearRgba,
    pub(crate) quick_open_selected: LinearRgba,
    pub(crate) project_search_background: LinearRgba,
    pub(crate) project_search_selected: LinearRgba,
    pub(crate) command_palette_background: LinearRgba,
    pub(crate) command_palette_selected: LinearRgba,
    pub(crate) syntax: SyntaxTheme,
}

impl StudioTheme {
    fn compiled() -> Result<Self, SettingsError> {
        Ok(Self {
            clear: color("clear", 0.02, 0.02, 0.02, 1.0)?,
            background: color("background", 0.035, 0.04, 0.045, 1.0)?,
            editor_background: color("editor background", 0.055, 0.06, 0.067, 1.0)?,
            selection: color("selection", 0.18, 0.48, 0.72, 0.42)?,
            text: color("text", 0.86, 0.88, 0.9, 1.0)?,
            caret: color("caret", 0.94, 0.72, 0.25, 1.0)?,
            status_background: color("status background", 0.34, 0.075, 0.065, 0.96)?,
            sidebar_background: color("sidebar background", 0.027, 0.031, 0.035, 1.0)?,
            active_row: color("active row", 0.12, 0.16, 0.19, 1.0)?,
            tab_background: color("tab background", 0.025, 0.028, 0.032, 1.0)?,
            active_tab: color("active tab", 0.095, 0.105, 0.115, 1.0)?,
            find_match: color("find match", 0.62, 0.45, 0.08, 0.38)?,
            find_background: color("find background", 0.08, 0.09, 0.10, 0.98)?,
            quick_open_background: color("quick open background", 0.045, 0.052, 0.058, 0.99)?,
            quick_open_selected: color("quick open selected", 0.12, 0.25, 0.31, 1.0)?,
            project_search_background: project_search_background()?,
            project_search_selected: color("project search selected", 0.10, 0.30, 0.22, 1.0)?,
            command_palette_background: command_palette_background()?,
            command_palette_selected: color("command palette selected", 0.34, 0.22, 0.075, 1.0)?,
            syntax: SyntaxTheme {
                comment: color("syntax comment", 0.48, 0.60, 0.53, 1.0)?,
                keyword: color("syntax keyword", 0.96, 0.48, 0.39, 1.0)?,
                string: color("syntax string", 0.55, 0.78, 0.49, 1.0)?,
                number: color("syntax number", 0.42, 0.67, 0.94, 1.0)?,
                type_name: color("syntax type", 0.94, 0.70, 0.32, 1.0)?,
                property: color("syntax property", 0.35, 0.76, 0.79, 1.0)?,
                heading: color("syntax heading", 0.96, 0.75, 0.34, 1.0)?,
                code: color("syntax code", 0.62, 0.76, 0.55, 1.0)?,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyAction {
    CommandPalette,
    Command(StudioCommand),
    SelectAll,
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyBinding {
    physical_key: u16,
    required_modifiers: u8,
    action: KeyAction,
    label: &'static str,
}

const DEFAULT_BINDINGS: [KeyBinding; 13] = [
    binding(
        KEY_P,
        COMMAND_SHIFT,
        KeyAction::CommandPalette,
        "Cmd+Shift+P",
    ),
    binding(
        KEY_F,
        COMMAND_SHIFT,
        KeyAction::Command(StudioCommand::OpenProjectSearch),
        "Cmd+Shift+F",
    ),
    binding(
        KEY_E,
        COMMAND_SHIFT,
        KeyAction::Command(StudioCommand::ToggleFileTree),
        "Cmd+Shift+E",
    ),
    binding(
        KEY_F,
        COMMAND_OPTION,
        KeyAction::Command(StudioCommand::OpenReplace),
        "Cmd+Opt+F",
    ),
    binding(
        KEY_P,
        Modifiers::COMMAND,
        KeyAction::Command(StudioCommand::OpenQuickOpen),
        "Cmd+P",
    ),
    binding(
        KEY_F,
        Modifiers::COMMAND,
        KeyAction::Command(StudioCommand::OpenFind),
        "Cmd+F",
    ),
    binding(KEY_A, Modifiers::COMMAND, KeyAction::SelectAll, "Cmd+A"),
    binding(
        KEY_S,
        Modifiers::COMMAND,
        KeyAction::Command(StudioCommand::SaveFile),
        "Cmd+S",
    ),
    binding(KEY_Z, COMMAND_SHIFT, KeyAction::Redo, "Cmd+Shift+Z"),
    binding(KEY_Z, Modifiers::COMMAND, KeyAction::Undo, "Cmd+Z"),
    binding(
        KEY_W,
        Modifiers::COMMAND,
        KeyAction::Command(StudioCommand::CloseTab),
        "Cmd+W",
    ),
    binding(
        KEY_LEFT_BRACKET,
        Modifiers::COMMAND,
        KeyAction::Command(StudioCommand::NavigateBack),
        "Cmd+[",
    ),
    binding(
        KEY_RIGHT_BRACKET,
        Modifiers::COMMAND,
        KeyAction::Command(StudioCommand::NavigateForward),
        "Cmd+]",
    ),
];

const fn binding(
    physical_key: u16,
    required_modifiers: u8,
    action: KeyAction,
    label: &'static str,
) -> KeyBinding {
    KeyBinding {
        physical_key,
        required_modifiers,
        action,
        label,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Keymap {
    bindings: &'static [KeyBinding],
}

impl Keymap {
    fn compiled() -> Result<Self, SettingsError> {
        validate_bindings(&DEFAULT_BINDINGS)?;
        Ok(Self {
            bindings: &DEFAULT_BINDINGS,
        })
    }

    pub(crate) fn resolve(self, physical_key: u16, modifiers: Modifiers) -> Option<KeyAction> {
        self.bindings
            .iter()
            .find(|binding| {
                binding.physical_key == physical_key
                    && modifiers.contains(binding.required_modifiers)
            })
            .map(|binding| binding.action)
    }

    pub(crate) fn shortcut_for(self, command: StudioCommand) -> Option<&'static str> {
        self.bindings.iter().find_map(|binding| {
            (binding.action == KeyAction::Command(command)).then_some(binding.label)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StudioSettings {
    pub(crate) editor: EditorSettings,
    pub(crate) theme: StudioTheme,
    pub(crate) keymap: Keymap,
}

impl StudioSettings {
    pub(crate) fn compiled() -> Result<Self, SettingsError> {
        let editor = EditorSettings::COMPILED;
        editor.validate()?;
        Ok(Self {
            editor,
            theme: StudioTheme::compiled()?,
            keymap: Keymap::compiled()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsError {
    InvalidMetric(&'static str),
    InvalidFontFamily,
    EmptyFontName,
    InvalidTabColumns,
    InvalidColor(&'static str),
    InvalidShortcutLabel,
    DuplicateBinding { physical_key: u16, modifiers: u8 },
    ShadowedBinding { physical_key: u16, modifiers: u8 },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetric(name) => {
                write!(formatter, "setting {name} must be positive and finite")
            }
            Self::InvalidFontFamily => formatter.write_str("font family identity must be nonzero"),
            Self::EmptyFontName => formatter.write_str("font name must not be empty"),
            Self::InvalidTabColumns => formatter.write_str("tab columns must be nonzero"),
            Self::InvalidColor(name) => write!(formatter, "theme color {name} is invalid"),
            Self::InvalidShortcutLabel => {
                formatter.write_str("shortcut label must be 1 to 16 ASCII bytes")
            }
            Self::DuplicateBinding {
                physical_key,
                modifiers,
            } => write!(
                formatter,
                "key {physical_key} with modifiers {modifiers:#04x} is duplicated"
            ),
            Self::ShadowedBinding {
                physical_key,
                modifiers,
            } => write!(
                formatter,
                "key {physical_key} with modifiers {modifiers:#04x} is unreachable"
            ),
        }
    }
}

impl Error for SettingsError {}

fn validate_metric(name: &'static str, value: f32) -> Result<(), SettingsError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(SettingsError::InvalidMetric(name))
    }
}

fn color(
    name: &'static str,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
) -> Result<LinearRgba, SettingsError> {
    LinearRgba::new(red, green, blue, alpha).ok_or(SettingsError::InvalidColor(name))
}

fn project_search_background() -> Result<LinearRgba, SettingsError> {
    color("project search background", 0.04, 0.06, 0.055, 0.995)
}

fn command_palette_background() -> Result<LinearRgba, SettingsError> {
    color("command palette background", 0.055, 0.062, 0.067, 0.995)
}

fn validate_bindings(bindings: &[KeyBinding]) -> Result<(), SettingsError> {
    for (index, binding) in bindings.iter().enumerate() {
        if binding.label.is_empty() || binding.label.len() > 16 || !binding.label.is_ascii() {
            return Err(SettingsError::InvalidShortcutLabel);
        }
        for previous in &bindings[..index] {
            if previous.physical_key != binding.physical_key {
                continue;
            }
            if previous.required_modifiers == binding.required_modifiers {
                return Err(SettingsError::DuplicateBinding {
                    physical_key: binding.physical_key,
                    modifiers: binding.required_modifiers,
                });
            }
            if binding.required_modifiers & previous.required_modifiers
                == previous.required_modifiers
            {
                return Err(SettingsError::ShadowedBinding {
                    physical_key: binding.physical_key,
                    modifiers: binding.required_modifiers,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_settings_are_valid_static_and_bounded() -> Result<(), SettingsError> {
        let settings = StudioSettings::compiled()?;
        assert_eq!(settings.editor.font_name, "Menlo-Regular");
        assert_eq!(settings.editor.tab_columns, 4);
        assert_eq!(settings.keymap.bindings.len(), 13);
        assert!(std::mem::size_of::<StudioSettings>() <= 512);
        let classes = [
            SyntaxClass::Comment,
            SyntaxClass::Keyword,
            SyntaxClass::String,
            SyntaxClass::Number,
            SyntaxClass::Type,
            SyntaxClass::Property,
            SyntaxClass::Heading,
            SyntaxClass::Code,
        ];
        let expected = [
            color("expected comment", 0.48, 0.60, 0.53, 1.0)?,
            color("expected keyword", 0.96, 0.48, 0.39, 1.0)?,
            color("expected string", 0.55, 0.78, 0.49, 1.0)?,
            color("expected number", 0.42, 0.67, 0.94, 1.0)?,
            color("expected type", 0.94, 0.70, 0.32, 1.0)?,
            color("expected property", 0.35, 0.76, 0.79, 1.0)?,
            color("expected heading", 0.96, 0.75, 0.34, 1.0)?,
            color("expected code", 0.62, 0.76, 0.55, 1.0)?,
        ];
        assert_eq!(
            classes.map(|class| settings.theme.syntax.color(class)),
            expected
        );
        assert_eq!(
            settings.keymap.shortcut_for(StudioCommand::OpenFind),
            Some("Cmd+F")
        );
        assert_eq!(
            settings.keymap.shortcut_for(StudioCommand::SplitRight),
            None
        );
        assert_eq!(COMMAND_SHIFT, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(COMMAND_OPTION, Modifiers::COMMAND | Modifiers::OPTION);
        Ok(())
    }

    #[test]
    fn keymap_preserves_specificity_and_ignores_unrelated_extra_modifiers()
    -> Result<(), SettingsError> {
        let keymap = Keymap::compiled()?;
        let bindings = [
            (KEY_P, COMMAND_SHIFT, KeyAction::CommandPalette),
            (
                KEY_F,
                COMMAND_SHIFT,
                KeyAction::Command(StudioCommand::OpenProjectSearch),
            ),
            (
                KEY_E,
                COMMAND_SHIFT,
                KeyAction::Command(StudioCommand::ToggleFileTree),
            ),
            (
                KEY_F,
                COMMAND_OPTION,
                KeyAction::Command(StudioCommand::OpenReplace),
            ),
            (
                KEY_P,
                Modifiers::COMMAND,
                KeyAction::Command(StudioCommand::OpenQuickOpen),
            ),
            (
                KEY_F,
                Modifiers::COMMAND,
                KeyAction::Command(StudioCommand::OpenFind),
            ),
            (KEY_A, Modifiers::COMMAND, KeyAction::SelectAll),
            (
                KEY_S,
                Modifiers::COMMAND,
                KeyAction::Command(StudioCommand::SaveFile),
            ),
            (KEY_Z, COMMAND_SHIFT, KeyAction::Redo),
            (KEY_Z, Modifiers::COMMAND, KeyAction::Undo),
            (
                KEY_W,
                Modifiers::COMMAND,
                KeyAction::Command(StudioCommand::CloseTab),
            ),
            (
                KEY_LEFT_BRACKET,
                Modifiers::COMMAND,
                KeyAction::Command(StudioCommand::NavigateBack),
            ),
            (
                KEY_RIGHT_BRACKET,
                Modifiers::COMMAND,
                KeyAction::Command(StudioCommand::NavigateForward),
            ),
        ];
        for (key, modifiers, expected) in bindings {
            assert_eq!(
                keymap.resolve(key, Modifiers::from_bits(modifiers)),
                Some(expected)
            );
        }
        assert_eq!(
            keymap.resolve(
                KEY_F,
                Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::OPTION)
            ),
            Some(KeyAction::Command(StudioCommand::OpenProjectSearch))
        );
        assert_eq!(
            keymap.resolve(
                KEY_S,
                Modifiers::from_bits(Modifiers::COMMAND | Modifiers::CONTROL)
            ),
            Some(KeyAction::Command(StudioCommand::SaveFile))
        );
        assert_eq!(keymap.resolve(KEY_S, Modifiers::default()), None);
        Ok(())
    }

    #[test]
    fn duplicate_and_shadowed_bindings_fail_with_exact_identity() {
        let duplicate = [
            binding(KEY_A, Modifiers::COMMAND, KeyAction::SelectAll, "Cmd+A"),
            binding(KEY_A, Modifiers::COMMAND, KeyAction::Undo, "Cmd+A"),
        ];
        assert_eq!(
            validate_bindings(&duplicate),
            Err(SettingsError::DuplicateBinding {
                physical_key: KEY_A,
                modifiers: Modifiers::COMMAND,
            })
        );
        let shadowed = [
            binding(KEY_F, Modifiers::COMMAND, KeyAction::Undo, "Cmd+F"),
            binding(
                KEY_F,
                Modifiers::COMMAND | Modifiers::SHIFT,
                KeyAction::Redo,
                "Cmd+Shift+F",
            ),
        ];
        assert_eq!(
            validate_bindings(&shadowed),
            Err(SettingsError::ShadowedBinding {
                physical_key: KEY_F,
                modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
            })
        );
    }

    #[test]
    fn each_editor_setting_guard_is_independent() {
        let mut settings = EditorSettings::COMPILED;
        settings.font_size = f32::NAN;
        assert_eq!(
            settings.validate(),
            Err(SettingsError::InvalidMetric("font size"))
        );
        settings = EditorSettings::COMPILED;
        settings.font_scale = 0.0;
        assert_eq!(
            settings.validate(),
            Err(SettingsError::InvalidMetric("font scale"))
        );
        settings = EditorSettings::COMPILED;
        settings.line_height = -1.0;
        assert_eq!(
            settings.validate(),
            Err(SettingsError::InvalidMetric("line height"))
        );
        settings = EditorSettings::COMPILED;
        settings.font_family = 0;
        assert_eq!(settings.validate(), Err(SettingsError::InvalidFontFamily));
        settings = EditorSettings::COMPILED;
        settings.font_name = "";
        assert_eq!(settings.validate(), Err(SettingsError::EmptyFontName));
        settings = EditorSettings::COMPILED;
        settings.tab_columns = 0;
        assert_eq!(settings.validate(), Err(SettingsError::InvalidTabColumns));
    }

    #[test]
    fn shortcut_labels_are_static_ascii_and_bounded() {
        let boundary = [binding(
            KEY_A,
            Modifiers::COMMAND,
            KeyAction::SelectAll,
            "1234567890123456",
        )];
        assert_eq!(validate_bindings(&boundary), Ok(()));
        let invalid = [binding(KEY_A, Modifiers::COMMAND, KeyAction::SelectAll, "")];
        assert_eq!(
            validate_bindings(&invalid),
            Err(SettingsError::InvalidShortcutLabel)
        );
        let oversized = [binding(
            KEY_A,
            Modifiers::COMMAND,
            KeyAction::SelectAll,
            "Command+Control+A",
        )];
        assert_eq!(
            validate_bindings(&oversized),
            Err(SettingsError::InvalidShortcutLabel)
        );
    }

    #[test]
    fn invalid_color_and_every_diagnostic_are_exact() {
        assert_eq!(
            color("broken", f32::NAN, 0.0, 0.0, 1.0),
            Err(SettingsError::InvalidColor("broken"))
        );
        let errors = [
            (
                SettingsError::InvalidMetric("line height"),
                "setting line height must be positive and finite".to_owned(),
            ),
            (
                SettingsError::InvalidFontFamily,
                "font family identity must be nonzero".to_owned(),
            ),
            (
                SettingsError::EmptyFontName,
                "font name must not be empty".to_owned(),
            ),
            (
                SettingsError::InvalidTabColumns,
                "tab columns must be nonzero".to_owned(),
            ),
            (
                SettingsError::InvalidColor("selection"),
                "theme color selection is invalid".to_owned(),
            ),
            (
                SettingsError::InvalidShortcutLabel,
                "shortcut label must be 1 to 16 ASCII bytes".to_owned(),
            ),
            (
                SettingsError::DuplicateBinding {
                    physical_key: KEY_A,
                    modifiers: Modifiers::COMMAND,
                },
                format!(
                    "key {KEY_A} with modifiers {:#04x} is duplicated",
                    Modifiers::COMMAND
                ),
            ),
            (
                SettingsError::ShadowedBinding {
                    physical_key: KEY_F,
                    modifiers: COMMAND_SHIFT,
                },
                format!("key {KEY_F} with modifiers {COMMAND_SHIFT:#04x} is unreachable"),
            ),
        ];
        for (error, expected) in errors {
            assert_eq!(error.to_string(), expected);
        }
    }
}
