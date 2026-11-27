// SPDX-License-Identifier: MIT OR Apache-2.0
//! Everything the person can ask for, by id: the menus, the ribbon, the keyboard and the command
//! search all run these, and the Linux global menu names them by [`Cmd::id`].

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cmd {
    NewBlank,
    OpenSample,
    Open,
    Save,
    SaveAs,
    Quit,
    AddMath,
    AddText,
    EvaluateAndAdd,
    Duplicate,
    DeleteCell,
    MoveUp,
    MoveDown,
    ToggleNavigator,
    ToggleSidePanel,
    ShowUnits,
    ShowDocumentSettings,
    ShowDocument,
    ShowScript,
    ToggleWorkspacePanel,
    ToggleFiguresPanel,
    RunScript,
    StopScript,
    ClearWorkspace,
    ClearCommandWindow,
    CloseFigures,
    ShowBlocks,
    RunBlocks,
    StopBlocks,
    DeleteBlock,
    BlocksExample,
    ExportSimulink,
    CommandSearch,
    Preferences,
    KeyboardShortcuts,
    About,
}

/// Where a command is listed in the shortcuts window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    File,
    Cells,
    View,
    Script,
    Blocks,
    Help,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::File,
        Category::Cells,
        Category::View,
        Category::Script,
        Category::Blocks,
        Category::Help,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Cells => "Cells",
            Category::View => "View",
            Category::Script => "Script",
            Category::Blocks => "Blocks",
            Category::Help => "Help",
        }
    }
}

const fn shortcut(command: bool, shift: bool, alt: bool, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(
        Modifiers {
            alt,
            ctrl: false,
            shift,
            mac_cmd: false,
            command,
        },
        key,
    ))
}

impl Cmd {
    pub const ALL: [Cmd; 36] = [
        Cmd::NewBlank,
        Cmd::OpenSample,
        Cmd::Open,
        Cmd::Save,
        Cmd::SaveAs,
        Cmd::Quit,
        Cmd::AddMath,
        Cmd::AddText,
        Cmd::EvaluateAndAdd,
        Cmd::Duplicate,
        Cmd::DeleteCell,
        Cmd::MoveUp,
        Cmd::MoveDown,
        Cmd::ToggleNavigator,
        Cmd::ToggleSidePanel,
        Cmd::ShowUnits,
        Cmd::ShowDocumentSettings,
        Cmd::ShowDocument,
        Cmd::ShowScript,
        Cmd::ToggleWorkspacePanel,
        Cmd::ToggleFiguresPanel,
        Cmd::RunScript,
        Cmd::StopScript,
        Cmd::ClearWorkspace,
        Cmd::ClearCommandWindow,
        Cmd::CloseFigures,
        Cmd::ShowBlocks,
        Cmd::RunBlocks,
        Cmd::StopBlocks,
        Cmd::DeleteBlock,
        Cmd::BlocksExample,
        Cmd::ExportSimulink,
        Cmd::CommandSearch,
        Cmd::Preferences,
        Cmd::KeyboardShortcuts,
        Cmd::About,
    ];

    /// A stable name, the one the Linux global menu sends back.
    pub fn id(self) -> &'static str {
        match self {
            Cmd::NewBlank => "file.new",
            Cmd::OpenSample => "file.sample",
            Cmd::Open => "file.open",
            Cmd::Save => "file.save",
            Cmd::SaveAs => "file.save-as",
            Cmd::Quit => "app.quit",
            Cmd::AddMath => "cell.add-math",
            Cmd::AddText => "cell.add-text",
            Cmd::EvaluateAndAdd => "cell.evaluate-and-add",
            Cmd::Duplicate => "cell.duplicate",
            Cmd::DeleteCell => "cell.delete",
            Cmd::MoveUp => "cell.move-up",
            Cmd::MoveDown => "cell.move-down",
            Cmd::ToggleNavigator => "view.navigator",
            Cmd::ToggleSidePanel => "view.side-panel",
            Cmd::ShowUnits => "view.units",
            Cmd::ShowDocumentSettings => "view.document",
            Cmd::ShowDocument => "view.maths-document",
            Cmd::ShowScript => "view.script",
            Cmd::ToggleWorkspacePanel => "view.workspace",
            Cmd::ToggleFiguresPanel => "view.figures",
            Cmd::RunScript => "script.run",
            Cmd::StopScript => "script.stop",
            Cmd::ClearWorkspace => "script.clear-workspace",
            Cmd::ClearCommandWindow => "script.clear-window",
            Cmd::CloseFigures => "script.close-figures",
            Cmd::ShowBlocks => "view.blocks",
            Cmd::RunBlocks => "blocks.run",
            Cmd::StopBlocks => "blocks.stop",
            Cmd::DeleteBlock => "blocks.delete",
            Cmd::BlocksExample => "blocks.example",
            Cmd::ExportSimulink => "blocks.export-simulink",
            Cmd::CommandSearch => "app.search",
            Cmd::Preferences => "app.preferences",
            Cmd::KeyboardShortcuts => "app.shortcuts",
            Cmd::About => "app.about",
        }
    }

    pub fn from_id(id: &str) -> Option<Cmd> {
        Cmd::ALL.iter().copied().find(|c| c.id() == id)
    }

    /// The English label; run it through [`tr`] to draw it.
    pub fn label(self) -> &'static str {
        match self {
            Cmd::NewBlank => "New document",
            Cmd::OpenSample => "Open the example",
            Cmd::Open => "Open…",
            Cmd::Save => "Save",
            Cmd::SaveAs => "Save as…",
            Cmd::Quit => "Quit",
            Cmd::AddMath => "Add a maths cell",
            Cmd::AddText => "Add a text cell",
            Cmd::EvaluateAndAdd => "Evaluate and add a cell",
            Cmd::Duplicate => "Duplicate the cell",
            Cmd::DeleteCell => "Delete the cell",
            Cmd::MoveUp => "Move the cell up",
            Cmd::MoveDown => "Move the cell down",
            Cmd::ToggleNavigator => "Show or hide the outline",
            Cmd::ToggleSidePanel => "Show or hide the side panel",
            Cmd::ShowUnits => "Units and constants",
            Cmd::ShowDocumentSettings => "Document settings",
            Cmd::ShowDocument => "Maths document",
            Cmd::ShowScript => "Script and command window",
            Cmd::ToggleWorkspacePanel => "Show or hide the workspace",
            Cmd::ToggleFiguresPanel => "Show or hide the figures",
            Cmd::RunScript => "Run the script",
            Cmd::StopScript => "Stop the run",
            Cmd::ClearWorkspace => "Clear the variables",
            Cmd::ClearCommandWindow => "Clear the command window",
            Cmd::CloseFigures => "Close all figures",
            Cmd::ShowBlocks => "Block diagrams",
            Cmd::RunBlocks => "Run the simulation",
            Cmd::StopBlocks => "Stop the simulation",
            Cmd::DeleteBlock => "Delete the selection",
            Cmd::BlocksExample => "Open the example diagram",
            Cmd::ExportSimulink => "Export as a Simulink model…",
            Cmd::CommandSearch => "Search commands",
            Cmd::Preferences => "Preferences",
            Cmd::KeyboardShortcuts => "Keyboard shortcuts",
            Cmd::About => "About Ábaco",
        }
    }

    pub fn translated(self) -> &'static str {
        tr(self.label())
    }

    pub fn category(self) -> Category {
        match self {
            Cmd::NewBlank | Cmd::OpenSample | Cmd::Open | Cmd::Save | Cmd::SaveAs | Cmd::Quit => {
                Category::File
            }
            Cmd::AddMath
            | Cmd::AddText
            | Cmd::EvaluateAndAdd
            | Cmd::Duplicate
            | Cmd::DeleteCell
            | Cmd::MoveUp
            | Cmd::MoveDown => Category::Cells,
            Cmd::ToggleNavigator
            | Cmd::ToggleSidePanel
            | Cmd::ShowUnits
            | Cmd::ShowDocumentSettings
            | Cmd::ShowDocument
            | Cmd::ShowScript
            | Cmd::ShowBlocks
            | Cmd::ToggleWorkspacePanel
            | Cmd::ToggleFiguresPanel => Category::View,
            Cmd::RunScript
            | Cmd::StopScript
            | Cmd::ClearWorkspace
            | Cmd::ClearCommandWindow
            | Cmd::CloseFigures => Category::Script,
            Cmd::RunBlocks
            | Cmd::StopBlocks
            | Cmd::DeleteBlock
            | Cmd::BlocksExample
            | Cmd::ExportSimulink => Category::Blocks,
            Cmd::CommandSearch | Cmd::Preferences | Cmd::KeyboardShortcuts | Cmd::About => {
                Category::Help
            }
        }
    }

    /// The icon in the ribbon and menus (an NDL icon name), when it has one.
    pub fn icon(self) -> Option<&'static str> {
        Some(match self {
            Cmd::NewBlank => "new",
            Cmd::OpenSample => "book",
            Cmd::Open => "open",
            Cmd::Save => "save",
            Cmd::SaveAs => "save-as",
            Cmd::Quit => "log-out",
            Cmd::AddMath => "calculator",
            Cmd::AddText => "text",
            Cmd::EvaluateAndAdd => "run",
            Cmd::Duplicate => "copy",
            Cmd::DeleteCell => "delete",
            Cmd::MoveUp => "move-up",
            Cmd::MoveDown => "move-down",
            Cmd::ToggleNavigator => "navigator",
            Cmd::ToggleSidePanel => "panel-right",
            Cmd::ShowUnits => "ruler",
            Cmd::ShowDocumentSettings => "settings",
            Cmd::ShowDocument => "file-text",
            Cmd::ShowScript => "terminal",
            Cmd::ToggleWorkspacePanel => "workspace",
            Cmd::ToggleFiguresPanel => "chart",
            Cmd::RunScript => "run",
            Cmd::StopScript => "stop",
            Cmd::ClearWorkspace => "eraser",
            Cmd::ClearCommandWindow => "brush-remove",
            Cmd::CloseFigures => "window-close",
            Cmd::ShowBlocks => "blocks",
            Cmd::RunBlocks => "run",
            Cmd::StopBlocks => "stop",
            Cmd::DeleteBlock => "delete",
            Cmd::BlocksExample => "book",
            Cmd::ExportSimulink => "save-as",
            Cmd::CommandSearch => "search",
            Cmd::Preferences => "settings",
            Cmd::KeyboardShortcuts => "keyboard",
            Cmd::About => "info",
        })
    }

    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        match self {
            Cmd::NewBlank => shortcut(true, false, false, Key::N),
            Cmd::Open => shortcut(true, false, false, Key::O),
            Cmd::Save => shortcut(true, false, false, Key::S),
            Cmd::SaveAs => shortcut(true, true, false, Key::S),
            Cmd::Quit => shortcut(true, false, false, Key::Q),
            Cmd::AddMath => shortcut(true, true, false, Key::M),
            Cmd::AddText => shortcut(true, true, false, Key::T),
            Cmd::EvaluateAndAdd => shortcut(true, false, false, Key::Enter),
            Cmd::Duplicate => shortcut(true, true, false, Key::D),
            Cmd::DeleteCell => shortcut(true, true, false, Key::Delete),
            Cmd::MoveUp => shortcut(false, false, true, Key::ArrowUp),
            Cmd::MoveDown => shortcut(false, false, true, Key::ArrowDown),
            Cmd::ToggleNavigator => shortcut(true, false, false, Key::B),
            Cmd::ToggleSidePanel => shortcut(true, true, false, Key::B),
            Cmd::ShowScript => shortcut(true, false, false, Key::J),
            Cmd::ShowDocument => shortcut(true, true, false, Key::J),
            Cmd::RunScript => shortcut(false, false, false, Key::F5),
            Cmd::ShowBlocks => shortcut(true, true, false, Key::L),
            Cmd::RunBlocks => shortcut(false, false, false, Key::F6),
            Cmd::StopBlocks => shortcut(false, true, false, Key::F6),
            Cmd::StopScript => shortcut(true, false, false, Key::Period),
            Cmd::CommandSearch => shortcut(true, false, false, Key::K),
            Cmd::Preferences => shortcut(true, false, false, Key::Comma),
            Cmd::KeyboardShortcuts => shortcut(true, false, false, Key::Slash),
            _ => None,
        }
    }

    /// The shortcut as the platform writes it (`Ctrl+S`, `⌘S`).
    pub fn shortcut_label(self, mac: bool) -> Option<String> {
        self.shortcut().map(|s| label_of(&s, mac))
    }
}

fn key_name(key: Key) -> String {
    match key {
        Key::Enter => "Enter".to_owned(),
        Key::Delete => "Delete".to_owned(),
        Key::ArrowUp => "↑".to_owned(),
        Key::ArrowDown => "↓".to_owned(),
        Key::Comma => ",".to_owned(),
        Key::Slash => "/".to_owned(),
        Key::Period => ".".to_owned(),
        other => other.name().to_owned(),
    }
}

/// How a shortcut is written on this platform.
pub fn label_of(s: &KeyboardShortcut, mac: bool) -> String {
    let m = s.modifiers;
    let key = key_name(s.logical_key);
    if mac {
        let mut out = String::new();
        if m.alt {
            out.push('⌥');
        }
        if m.shift {
            out.push('⇧');
        }
        if m.command {
            out.push('⌘');
        }
        out.push_str(&key);
        out
    } else {
        let mut parts: Vec<&str> = Vec::new();
        if m.command {
            parts.push("Ctrl");
        }
        if m.alt {
            parts.push("Alt");
        }
        if m.shift {
            parts.push("Shift");
        }
        let mut out = parts.join("+");
        if !out.is_empty() {
            out.push('+');
        }
        out.push_str(&key);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for cmd in Cmd::ALL {
            assert!(seen.insert(cmd.id()), "{} twice", cmd.id());
            assert_eq!(Cmd::from_id(cmd.id()), Some(cmd));
        }
        assert_eq!(Cmd::from_id("nope"), None);
    }

    #[test]
    fn shortcuts_do_not_collide() {
        let mut seen = std::collections::HashMap::new();
        for cmd in Cmd::ALL {
            if let Some(s) = cmd.shortcut() {
                assert!(
                    seen.insert(
                        (
                            s.modifiers.command,
                            s.modifiers.shift,
                            s.modifiers.alt,
                            s.logical_key
                        ),
                        cmd
                    )
                    .is_none(),
                    "{cmd:?} reuses a shortcut"
                );
            }
        }
        assert!(seen.len() >= 18);
    }

    #[test]
    fn shortcuts_are_written_for_the_platform() {
        assert_eq!(Cmd::Save.shortcut_label(false).as_deref(), Some("Ctrl+S"));
        assert_eq!(
            Cmd::SaveAs.shortcut_label(false).as_deref(),
            Some("Ctrl+Shift+S")
        );
        assert_eq!(Cmd::SaveAs.shortcut_label(true).as_deref(), Some("⇧⌘S"));
        assert_eq!(
            Cmd::EvaluateAndAdd.shortcut_label(false).as_deref(),
            Some("Ctrl+Enter")
        );
        assert_eq!(Cmd::MoveUp.shortcut_label(false).as_deref(), Some("Alt+↑"));
        assert_eq!(Cmd::About.shortcut_label(false), None);
        assert_eq!(Cmd::RunScript.shortcut_label(false).as_deref(), Some("F5"));
        assert_eq!(
            Cmd::StopScript.shortcut_label(false).as_deref(),
            Some("Ctrl+.")
        );
    }

    #[test]
    fn every_command_has_a_label_and_a_known_icon() {
        for cmd in Cmd::ALL {
            assert!(!cmd.label().is_empty());
            if let Some(icon) = cmd.icon() {
                assert!(
                    nodalia_theme::icons::get(icon).is_some(),
                    "{cmd:?}: no NDL icon called {icon}"
                );
            }
        }
    }

    #[test]
    fn every_command_and_category_has_spanish() {
        let has = |english: &str| {
            crate::spanish::SPANISH
                .binary_search_by(|(en, _)| en.cmp(&english))
                .is_ok()
        };
        let mut missing: Vec<&str> = Cmd::ALL
            .iter()
            .map(|c| c.label())
            .filter(|l| !has(l))
            .collect();
        missing.extend(Category::ALL.iter().map(|c| c.label()).filter(|l| !has(l)));
        assert!(missing.is_empty(), "without Spanish: {missing:?}");
    }
}
