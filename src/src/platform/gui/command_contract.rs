use crate::domain::ROOT_NODE_ID;
#[cfg(any(target_os = "linux", test))]
use crate::domain::{AppearanceTheme, TextEncoding, UiLanguage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GuiCommand {
    SaveDocument,
    ImportText,
    ExportText,
    ExportAllText,
    CloseTab,
    CloseWindow,
    Undo,
    Cut,
    Copy,
    Paste,
    DeleteSelection,
    SelectAll,
    FindText,
    ReplaceText,
    NewDocument,
    NewChildDocument,
    Rename,
    MoveUp,
    MoveDown,
    MoveToTrash,
    Restore,
    DeletePermanently,
    ShowActiveTree,
    ShowTrash,
    WordWrap,
    EditorFont,
    About,
}

impl GuiCommand {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 27] = [
        Self::SaveDocument,
        Self::ImportText,
        Self::ExportText,
        Self::ExportAllText,
        Self::CloseTab,
        Self::CloseWindow,
        Self::Undo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::DeleteSelection,
        Self::SelectAll,
        Self::FindText,
        Self::ReplaceText,
        Self::NewDocument,
        Self::NewChildDocument,
        Self::Rename,
        Self::MoveUp,
        Self::MoveDown,
        Self::MoveToTrash,
        Self::Restore,
        Self::DeletePermanently,
        Self::ShowActiveTree,
        Self::ShowTrash,
        Self::WordWrap,
        Self::EditorFont,
        Self::About,
    ];

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn gtk_action_name(self) -> &'static str {
        match self {
            Self::SaveDocument => "save",
            Self::ImportText => "import-text",
            Self::ExportText => "export-text",
            Self::ExportAllText => "export-all-text",
            Self::CloseTab => "close-tab",
            Self::CloseWindow => "close-window",
            Self::Undo => "undo",
            Self::Cut => "cut",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::DeleteSelection => "delete-selection",
            Self::SelectAll => "select-all",
            Self::FindText => "find",
            Self::ReplaceText => "replace",
            Self::NewDocument => "new-document",
            Self::NewChildDocument => "new-child-document",
            Self::Rename => "rename",
            Self::MoveUp => "move-up",
            Self::MoveDown => "move-down",
            Self::MoveToTrash => "delete",
            Self::Restore => "restore",
            Self::DeletePermanently => "delete-permanently",
            Self::ShowActiveTree | Self::ShowTrash => "tree-mode",
            Self::WordWrap => "word-wrap",
            Self::EditorFont => "editor-font",
            Self::About => "about",
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn gtk_detailed_action(self) -> &'static str {
        match self {
            Self::SaveDocument => "win.save",
            Self::ImportText => "win.import-text",
            Self::ExportText => "win.export-text",
            Self::ExportAllText => "win.export-all-text",
            Self::CloseTab => "win.close-tab",
            Self::CloseWindow => "win.close-window",
            Self::Undo => "win.undo",
            Self::Cut => "win.cut",
            Self::Copy => "win.copy",
            Self::Paste => "win.paste",
            Self::DeleteSelection => "win.delete-selection",
            Self::SelectAll => "win.select-all",
            Self::FindText => "win.find",
            Self::ReplaceText => "win.replace",
            Self::NewDocument => "win.new-document",
            Self::NewChildDocument => "win.new-child-document",
            Self::Rename => "win.rename",
            Self::MoveUp => "win.move-up",
            Self::MoveDown => "win.move-down",
            Self::MoveToTrash => "win.delete",
            Self::Restore => "win.restore",
            Self::DeletePermanently => "win.delete-permanently",
            Self::ShowActiveTree => "win.tree-mode::active",
            Self::ShowTrash => "win.tree-mode::trash",
            Self::WordWrap => "win.word-wrap",
            Self::EditorFont => "win.editor-font",
            Self::About => "win.about",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuiOptionMenu {
    ImportEncoding,
    ExportEncoding,
    Theme,
    Language,
}

impl GuiOptionMenu {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn gtk_action_name(self) -> &'static str {
        match self {
            Self::ImportEncoding => "import-encoding",
            Self::ExportEncoding => "export-encoding",
            Self::Theme => "theme",
            Self::Language => "language",
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn gtk_detailed_action_for_encoding(self, encoding: TextEncoding) -> Option<String> {
        match self {
            Self::ImportEncoding if encoding.is_import_supported() => Some(format!(
                "win.{}::{}",
                self.gtk_action_name(),
                encoding.storage_value()
            )),
            Self::ExportEncoding if encoding.is_export_supported() => Some(format!(
                "win.{}::{}",
                self.gtk_action_name(),
                encoding.storage_value()
            )),
            _ => None,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn gtk_detailed_action_for_theme(self, theme: AppearanceTheme) -> Option<String> {
        (self == Self::Theme)
            .then(|| format!("win.{}::{}", self.gtk_action_name(), theme.storage_value()))
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn gtk_detailed_action_for_language(self, language: UiLanguage) -> Option<String> {
        (self == Self::Language).then(|| {
            format!(
                "win.{}::{}",
                self.gtk_action_name(),
                language.storage_value()
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuiMenuKind {
    File,
    Edit,
    Document,
    View,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuiMenuEntry {
    Command(GuiCommand),
    Separator,
    OptionMenu(GuiOptionMenu),
}

pub(crate) struct GuiMenuSpec {
    pub(crate) kind: GuiMenuKind,
    pub(crate) entries: &'static [GuiMenuEntry],
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GuiShortcutScope {
    Global,
    Tree,
    Editor,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GuiShortcut {
    pub(crate) command: GuiCommand,
    pub(crate) scope: GuiShortcutScope,
    pub(crate) accelerator: &'static str,
}

#[cfg(any(target_os = "linux", test))]
pub(crate) const GUI_SHORTCUTS: &[GuiShortcut] = &[
    GuiShortcut {
        command: GuiCommand::SaveDocument,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary>s",
    },
    GuiShortcut {
        command: GuiCommand::SaveDocument,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary><Alt>s",
    },
    GuiShortcut {
        command: GuiCommand::NewDocument,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary>n",
    },
    GuiShortcut {
        command: GuiCommand::NewDocument,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary><Alt>n",
    },
    GuiShortcut {
        command: GuiCommand::CloseTab,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary>w",
    },
    GuiShortcut {
        command: GuiCommand::CloseTab,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary><Alt>w",
    },
    GuiShortcut {
        command: GuiCommand::FindText,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary>f",
    },
    GuiShortcut {
        command: GuiCommand::FindText,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary><Alt>f",
    },
    GuiShortcut {
        command: GuiCommand::ReplaceText,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary>h",
    },
    GuiShortcut {
        command: GuiCommand::ReplaceText,
        scope: GuiShortcutScope::Global,
        accelerator: "<Primary><Alt>h",
    },
    GuiShortcut {
        command: GuiCommand::SelectAll,
        scope: GuiShortcutScope::Editor,
        accelerator: "<Primary>a",
    },
    GuiShortcut {
        command: GuiCommand::NewDocument,
        scope: GuiShortcutScope::Tree,
        accelerator: "Return",
    },
    GuiShortcut {
        command: GuiCommand::NewChildDocument,
        scope: GuiShortcutScope::Tree,
        accelerator: "<Primary>Return",
    },
    GuiShortcut {
        command: GuiCommand::Rename,
        scope: GuiShortcutScope::Tree,
        accelerator: "F2",
    },
    GuiShortcut {
        command: GuiCommand::MoveToTrash,
        scope: GuiShortcutScope::Tree,
        accelerator: "Delete",
    },
    GuiShortcut {
        command: GuiCommand::MoveUp,
        scope: GuiShortcutScope::Tree,
        accelerator: "<Primary>Up",
    },
    GuiShortcut {
        command: GuiCommand::MoveDown,
        scope: GuiShortcutScope::Tree,
        accelerator: "<Primary>Down",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuiTreeMode {
    Active,
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuiCommandAvailability {
    pub(crate) save_enabled: bool,
    pub(crate) close_tab_enabled: bool,
    pub(crate) new_child_document_enabled: bool,
    pub(crate) new_document_enabled: bool,
    pub(crate) rename_enabled: bool,
    pub(crate) move_up_enabled: bool,
    pub(crate) move_down_enabled: bool,
    pub(crate) delete_enabled: bool,
    pub(crate) restore_enabled: bool,
    pub(crate) delete_permanently_enabled: bool,
    pub(crate) active_tree_checked: bool,
    pub(crate) trash_checked: bool,
}

impl GuiCommandAvailability {
    pub(crate) fn for_context(
        tree_mode: GuiTreeMode,
        search_active: bool,
        selected_node_id: Option<i64>,
        move_up_enabled: bool,
        move_down_enabled: bool,
        active_tab: bool,
    ) -> Self {
        let active_tree_checked = tree_mode == GuiTreeMode::Active;
        let trash_checked = tree_mode == GuiTreeMode::Trash;
        let selected = selected_node_id.is_some();

        match tree_mode {
            GuiTreeMode::Active if search_active => Self {
                save_enabled: active_tab,
                close_tab_enabled: active_tab,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked,
                trash_checked,
            },
            GuiTreeMode::Active => Self {
                save_enabled: active_tab,
                close_tab_enabled: active_tab,
                new_child_document_enabled: selected,
                new_document_enabled: true,
                rename_enabled: selected,
                move_up_enabled,
                move_down_enabled,
                delete_enabled: selected_node_id.is_some_and(|node_id| node_id != ROOT_NODE_ID),
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked,
                trash_checked,
            },
            GuiTreeMode::Trash => Self {
                save_enabled: active_tab,
                close_tab_enabled: active_tab,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: selected,
                delete_permanently_enabled: selected,
                active_tree_checked,
                trash_checked,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuiEditorAvailability {
    pub(crate) undo_enabled: bool,
    pub(crate) cut_enabled: bool,
    pub(crate) copy_enabled: bool,
    pub(crate) paste_enabled: bool,
    pub(crate) delete_enabled: bool,
    pub(crate) select_all_enabled: bool,
    pub(crate) find_replace_enabled: bool,
}

impl GuiEditorAvailability {
    pub(crate) fn for_context(
        active_tab: bool,
        editable: bool,
        has_selection: bool,
        can_undo: bool,
        has_text: bool,
    ) -> Self {
        Self {
            undo_enabled: active_tab && editable && can_undo,
            cut_enabled: active_tab && editable && has_selection,
            copy_enabled: active_tab && has_selection,
            paste_enabled: active_tab && editable,
            delete_enabled: active_tab && editable && has_selection,
            select_all_enabled: active_tab && has_text,
            find_replace_enabled: active_tab && editable,
        }
    }
}

const FILE_MENU_ENTRIES: &[GuiMenuEntry] = &[
    GuiMenuEntry::Command(GuiCommand::SaveDocument),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::ImportText),
    GuiMenuEntry::OptionMenu(GuiOptionMenu::ImportEncoding),
    GuiMenuEntry::Command(GuiCommand::ExportText),
    GuiMenuEntry::Command(GuiCommand::ExportAllText),
    GuiMenuEntry::OptionMenu(GuiOptionMenu::ExportEncoding),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::CloseTab),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::CloseWindow),
];

const EDIT_MENU_ENTRIES: &[GuiMenuEntry] = &[
    GuiMenuEntry::Command(GuiCommand::Undo),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::Cut),
    GuiMenuEntry::Command(GuiCommand::Copy),
    GuiMenuEntry::Command(GuiCommand::Paste),
    GuiMenuEntry::Command(GuiCommand::DeleteSelection),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::SelectAll),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::FindText),
    GuiMenuEntry::Command(GuiCommand::ReplaceText),
];

const DOCUMENT_MENU_ENTRIES: &[GuiMenuEntry] = &[
    GuiMenuEntry::Command(GuiCommand::NewDocument),
    GuiMenuEntry::Command(GuiCommand::NewChildDocument),
    GuiMenuEntry::Command(GuiCommand::Rename),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::MoveUp),
    GuiMenuEntry::Command(GuiCommand::MoveDown),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::MoveToTrash),
    GuiMenuEntry::Command(GuiCommand::Restore),
    GuiMenuEntry::Command(GuiCommand::DeletePermanently),
];

const VIEW_MENU_ENTRIES: &[GuiMenuEntry] = &[
    GuiMenuEntry::Command(GuiCommand::ShowActiveTree),
    GuiMenuEntry::Command(GuiCommand::ShowTrash),
    GuiMenuEntry::Separator,
    GuiMenuEntry::OptionMenu(GuiOptionMenu::Theme),
    GuiMenuEntry::OptionMenu(GuiOptionMenu::Language),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::WordWrap),
    GuiMenuEntry::Command(GuiCommand::EditorFont),
];

const HELP_MENU_ENTRIES: &[GuiMenuEntry] = &[GuiMenuEntry::Command(GuiCommand::About)];

pub(crate) const MAIN_MENU_SPECS: &[GuiMenuSpec] = &[
    GuiMenuSpec {
        kind: GuiMenuKind::File,
        entries: FILE_MENU_ENTRIES,
    },
    GuiMenuSpec {
        kind: GuiMenuKind::Edit,
        entries: EDIT_MENU_ENTRIES,
    },
    GuiMenuSpec {
        kind: GuiMenuKind::Document,
        entries: DOCUMENT_MENU_ENTRIES,
    },
    GuiMenuSpec {
        kind: GuiMenuKind::View,
        entries: VIEW_MENU_ENTRIES,
    },
    GuiMenuSpec {
        kind: GuiMenuKind::Help,
        entries: HELP_MENU_ENTRIES,
    },
];

pub(crate) const TREE_CONTEXT_MENU_ENTRIES: &[GuiMenuEntry] = &[
    GuiMenuEntry::Command(GuiCommand::NewDocument),
    GuiMenuEntry::Command(GuiCommand::NewChildDocument),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::Rename),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::MoveUp),
    GuiMenuEntry::Command(GuiCommand::MoveDown),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::MoveToTrash),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::Restore),
    GuiMenuEntry::Command(GuiCommand::DeletePermanently),
];

pub(crate) const EDITOR_CONTEXT_MENU_ENTRIES: &[GuiMenuEntry] = &[
    GuiMenuEntry::Command(GuiCommand::Undo),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::Cut),
    GuiMenuEntry::Command(GuiCommand::Copy),
    GuiMenuEntry::Command(GuiCommand::Paste),
    GuiMenuEntry::Command(GuiCommand::DeleteSelection),
    GuiMenuEntry::Separator,
    GuiMenuEntry::Command(GuiCommand::SelectAll),
];

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    fn command_entries(entries: &[GuiMenuEntry]) -> impl Iterator<Item = GuiCommand> + '_ {
        entries.iter().filter_map(|entry| match entry {
            GuiMenuEntry::Command(command) => Some(*command),
            GuiMenuEntry::Separator | GuiMenuEntry::OptionMenu(_) => None,
        })
    }

    fn detailed_action_target<'a>(action: &'a str, action_name: &str) -> &'a str {
        let (owner, target) = action
            .split_once("::")
            .unwrap_or_else(|| panic!("{action} must include a state target"));
        assert_eq!(owner, format!("win.{action_name}"));
        assert!(
            !target.is_empty(),
            "{action} must include a non-empty target"
        );
        target
    }

    fn shortcut_accelerators(
        command: GuiCommand,
        scope: GuiShortcutScope,
    ) -> HashSet<&'static str> {
        GUI_SHORTCUTS
            .iter()
            .filter(|shortcut| shortcut.command == command && shortcut.scope == scope)
            .map(|shortcut| shortcut.accelerator)
            .collect()
    }

    #[test]
    fn main_menu_contract_lists_every_command_once() {
        let mut commands = MAIN_MENU_SPECS
            .iter()
            .flat_map(|spec| command_entries(spec.entries))
            .collect::<Vec<_>>();
        commands.sort_by_key(|command| *command as u8);

        let mut expected = GuiCommand::ALL.to_vec();
        expected.sort_by_key(|command| *command as u8);

        assert_eq!(commands, expected);
    }

    #[test]
    fn gtk_detailed_actions_are_unique() {
        let mut actions = HashSet::new();
        for command in GuiCommand::ALL {
            assert!(
                actions.insert(command.gtk_detailed_action()),
                "duplicate GTK detailed action for {command:?}"
            );
        }
    }

    #[test]
    fn gtk_detailed_actions_match_declared_window_action_names() {
        for command in GuiCommand::ALL {
            let detailed_action = command.gtk_detailed_action();
            let (action, target) = detailed_action
                .split_once("::")
                .map_or((detailed_action, None), |(action, target)| {
                    (action, Some(target))
                });

            assert_eq!(action, format!("win.{}", command.gtk_action_name()));
            match command {
                GuiCommand::ShowActiveTree => assert_eq!(target, Some("active")),
                GuiCommand::ShowTrash => assert_eq!(target, Some("trash")),
                _ => assert_eq!(target, None, "{command:?} should not require a GTK target"),
            }
        }
    }

    #[test]
    fn gtk_action_names_are_unique_except_tree_mode_radio_pair() {
        let mut commands_by_action = HashMap::new();
        for command in GuiCommand::ALL {
            commands_by_action
                .entry(command.gtk_action_name())
                .or_insert_with(Vec::new)
                .push(command);
        }

        for (action, commands) in commands_by_action {
            if commands.len() == 1 {
                continue;
            }
            assert_eq!(action, "tree-mode");
            assert_eq!(
                commands,
                vec![GuiCommand::ShowActiveTree, GuiCommand::ShowTrash]
            );
        }
    }

    #[test]
    fn context_menus_are_main_menu_command_subsets() {
        let main_commands = MAIN_MENU_SPECS
            .iter()
            .flat_map(|spec| command_entries(spec.entries))
            .collect::<HashSet<_>>();

        for command in command_entries(TREE_CONTEXT_MENU_ENTRIES)
            .chain(command_entries(EDITOR_CONTEXT_MENU_ENTRIES))
        {
            assert!(
                main_commands.contains(&command),
                "context command {command:?} must also be in the main menu contract"
            );
        }
    }

    #[test]
    fn option_menu_actions_match_supported_options() {
        let import = GuiOptionMenu::ImportEncoding;
        for encoding in TextEncoding::import_options() {
            assert!(import.gtk_detailed_action_for_encoding(*encoding).is_some());
        }

        let export = GuiOptionMenu::ExportEncoding;
        for encoding in TextEncoding::export_options() {
            assert!(export.gtk_detailed_action_for_encoding(*encoding).is_some());
        }
        assert!(export
            .gtk_detailed_action_for_encoding(TextEncoding::AutoDetect)
            .is_none());

        for theme in AppearanceTheme::options() {
            assert!(GuiOptionMenu::Theme
                .gtk_detailed_action_for_theme(*theme)
                .is_some());
        }

        for language in UiLanguage::options() {
            assert!(GuiOptionMenu::Language
                .gtk_detailed_action_for_language(*language)
                .is_some());
        }
    }

    #[test]
    fn stateful_menu_action_targets_match_storage_values() {
        assert_eq!(
            detailed_action_target(
                GuiCommand::ShowActiveTree.gtk_detailed_action(),
                "tree-mode"
            ),
            "active"
        );
        assert_eq!(
            detailed_action_target(GuiCommand::ShowTrash.gtk_detailed_action(), "tree-mode"),
            "trash"
        );

        let import = GuiOptionMenu::ImportEncoding;
        for encoding in TextEncoding::import_options() {
            let action = import
                .gtk_detailed_action_for_encoding(*encoding)
                .expect("import encoding should have a GTK action target");
            let target = detailed_action_target(&action, import.gtk_action_name());
            assert_eq!(target, encoding.storage_value());
            assert_eq!(
                TextEncoding::from_import_storage_value(target),
                Some(*encoding)
            );
        }

        let export = GuiOptionMenu::ExportEncoding;
        for encoding in TextEncoding::export_options() {
            let action = export
                .gtk_detailed_action_for_encoding(*encoding)
                .expect("export encoding should have a GTK action target");
            let target = detailed_action_target(&action, export.gtk_action_name());
            assert_eq!(target, encoding.storage_value());
            assert_eq!(
                TextEncoding::from_export_storage_value(target),
                Some(*encoding)
            );
        }

        for theme in AppearanceTheme::options() {
            let action = GuiOptionMenu::Theme
                .gtk_detailed_action_for_theme(*theme)
                .expect("theme should have a GTK action target");
            let target = detailed_action_target(&action, GuiOptionMenu::Theme.gtk_action_name());
            assert_eq!(target, theme.storage_value());
            assert_eq!(AppearanceTheme::from_storage_value(target), Some(*theme));
        }

        for language in UiLanguage::options() {
            let action = GuiOptionMenu::Language
                .gtk_detailed_action_for_language(*language)
                .expect("language should have a GTK action target");
            let target = detailed_action_target(&action, GuiOptionMenu::Language.gtk_action_name());
            assert_eq!(target, language.storage_value());
            assert_eq!(UiLanguage::from_storage_value(target), Some(*language));
        }
    }

    #[test]
    fn shortcut_contract_keeps_focus_scoped_shortcuts_out_of_global_accelerators() {
        let global_commands = GUI_SHORTCUTS
            .iter()
            .filter(|shortcut| shortcut.scope == GuiShortcutScope::Global)
            .map(|shortcut| shortcut.command)
            .collect::<HashSet<_>>();

        assert!(global_commands.contains(&GuiCommand::SaveDocument));
        assert!(global_commands.contains(&GuiCommand::NewDocument));
        assert!(global_commands.contains(&GuiCommand::CloseTab));
        assert!(global_commands.contains(&GuiCommand::FindText));
        assert!(global_commands.contains(&GuiCommand::ReplaceText));

        assert!(!global_commands.contains(&GuiCommand::NewChildDocument));
        assert!(!global_commands.contains(&GuiCommand::Rename));
        assert!(!global_commands.contains(&GuiCommand::MoveToTrash));
        assert!(!global_commands.contains(&GuiCommand::MoveUp));
        assert!(!global_commands.contains(&GuiCommand::MoveDown));
        assert!(!global_commands.contains(&GuiCommand::SelectAll));
    }

    #[test]
    fn global_shortcut_contract_matches_win32_ctrl_alt_tolerance() {
        for (command, key) in [
            (GuiCommand::SaveDocument, "s"),
            (GuiCommand::NewDocument, "n"),
            (GuiCommand::CloseTab, "w"),
            (GuiCommand::FindText, "f"),
            (GuiCommand::ReplaceText, "h"),
        ] {
            let accelerators = shortcut_accelerators(command, GuiShortcutScope::Global);
            assert!(accelerators.contains(format!("<Primary>{key}").as_str()));
            assert!(accelerators.contains(format!("<Primary><Alt>{key}").as_str()));
        }

        let editor_select_all =
            shortcut_accelerators(GuiCommand::SelectAll, GuiShortcutScope::Editor);
        assert!(editor_select_all.contains("<Primary>a"));
        assert!(!editor_select_all.contains("<Primary><Alt>a"));

        let tree_new_child =
            shortcut_accelerators(GuiCommand::NewChildDocument, GuiShortcutScope::Tree);
        assert!(tree_new_child.contains("<Primary>Return"));
        assert!(!tree_new_child.contains("<Primary><Alt>Return"));
    }

    #[test]
    fn shortcut_contract_has_no_duplicate_scope_accelerators() {
        let mut accelerators = HashSet::new();
        for shortcut in GUI_SHORTCUTS {
            assert!(
                accelerators.insert((shortcut.scope, shortcut.accelerator)),
                "duplicate shortcut {:?} {}",
                shortcut.scope,
                shortcut.accelerator
            );
        }
    }

    #[test]
    fn active_tree_command_availability_tracks_selection_and_root() {
        assert_eq!(
            GuiCommandAvailability::for_context(
                GuiTreeMode::Active,
                false,
                None,
                false,
                false,
                false
            ),
            GuiCommandAvailability {
                save_enabled: false,
                close_tab_enabled: false,
                new_child_document_enabled: false,
                new_document_enabled: true,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );

        assert_eq!(
            GuiCommandAvailability::for_context(
                GuiTreeMode::Active,
                false,
                Some(ROOT_NODE_ID),
                false,
                false,
                true,
            ),
            GuiCommandAvailability {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: true,
                new_document_enabled: true,
                rename_enabled: true,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );

        assert_eq!(
            GuiCommandAvailability::for_context(
                GuiTreeMode::Active,
                false,
                Some(2),
                true,
                true,
                true
            ),
            GuiCommandAvailability {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: true,
                new_document_enabled: true,
                rename_enabled: true,
                move_up_enabled: true,
                move_down_enabled: true,
                delete_enabled: true,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );
    }

    #[test]
    fn search_results_disable_node_commands_and_keep_active_tree_checked() {
        assert_eq!(
            GuiCommandAvailability::for_context(
                GuiTreeMode::Active,
                true,
                Some(2),
                true,
                true,
                true
            ),
            GuiCommandAvailability {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );
    }

    #[test]
    fn trash_command_availability_enables_restore_and_permanent_delete_for_selection() {
        assert_eq!(
            GuiCommandAvailability::for_context(GuiTreeMode::Trash, false, None, true, true, false),
            GuiCommandAvailability {
                save_enabled: false,
                close_tab_enabled: false,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: false,
                trash_checked: true,
            }
        );

        assert_eq!(
            GuiCommandAvailability::for_context(
                GuiTreeMode::Trash,
                false,
                Some(42),
                true,
                true,
                true
            ),
            GuiCommandAvailability {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: true,
                delete_permanently_enabled: true,
                active_tree_checked: false,
                trash_checked: true,
            }
        );
    }

    #[test]
    fn editor_availability_respects_editability_and_selection() {
        assert_eq!(
            GuiEditorAvailability::for_context(true, true, true, true, true),
            GuiEditorAvailability {
                undo_enabled: true,
                cut_enabled: true,
                copy_enabled: true,
                paste_enabled: true,
                delete_enabled: true,
                select_all_enabled: true,
                find_replace_enabled: true,
            }
        );

        assert_eq!(
            GuiEditorAvailability::for_context(true, false, true, true, true),
            GuiEditorAvailability {
                undo_enabled: false,
                cut_enabled: false,
                copy_enabled: true,
                paste_enabled: false,
                delete_enabled: false,
                select_all_enabled: true,
                find_replace_enabled: false,
            }
        );

        assert_eq!(
            GuiEditorAvailability::for_context(false, false, true, true, true),
            GuiEditorAvailability {
                undo_enabled: false,
                cut_enabled: false,
                copy_enabled: false,
                paste_enabled: false,
                delete_enabled: false,
                select_all_enabled: false,
                find_replace_enabled: false,
            }
        );
    }
}
