use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use gtk4 as gtk;

use crate::app::App;
use crate::domain::{
    find_next_literal, replace_all_literal, toggle_editor_word_wrap, AppearanceTheme,
    DirtyTabDecision, Document, DocumentSearchResult, DocumentTabSource, DocumentTabViewState,
    DomainError, EditorFontSettings, LoadedTabMetadataUpdate, Node, OpenDocumentTabInput, OpenTabs,
    ReplaceAllError, SiblingMoveDirection, SplitterSettings, TextEncoding, TextMatch, UiLanguage,
    UiSettings, WindowSettings, APP_AUTHOR_URL, APP_DISPLAY_NAME, APP_ICON_PNG_FILE_NAME,
    APP_ICON_SVG_FILE_NAME, APP_LINUX_APPLICATION_ID, MAX_EDITOR_FONT_SIZE_PT,
    MIN_EDITOR_FONT_SIZE_PT, ROOT_NODE_ID, SEARCH_RESULT_LIMIT,
};
use crate::error::{
    AppError, IoUserMessage, PlatformUserMessage, SqliteUserMessage, TextEncodingUserMessage,
    TextFileTooLargeUserMessage,
};
use crate::infra::text_file::TEXT_FILE_BYTE_LIMIT;

use super::command_contract::{
    GuiCommand, GuiCommandAvailability, GuiEditorAvailability, GuiMenuEntry, GuiMenuKind,
    GuiOptionMenu, GuiShortcutScope, GuiTreeMode, EDITOR_CONTEXT_MENU_ENTRIES, GUI_SHORTCUTS,
    MAIN_MENU_SPECS, TREE_CONTEXT_MENU_ENTRIES,
};

const SEARCH_DEBOUNCE_MS: u64 = 180;
const UI_SETTINGS_SAVE_DEBOUNCE_MS: u64 = 250;
const SELECTION_UI_SETTINGS_SAVE_DEBOUNCE_MS: u64 = 250;
const MIN_SPLIT_WIDTH_PX: i32 = 80;
const MIN_EDITOR_WIDTH_PX: i32 = 160;
const SPLITTER_WIDTH_PX: i32 = 4;
const SEARCH_BOX_HEIGHT_PX: i32 = 24;
const SEARCH_PANEL_PADDING_PX: i32 = 6;
const TAB_BAR_HEIGHT_PX: i32 = 28;
const TAB_CLOSE_HIT_WIDTH_PX: i32 = 22;
const TAB_CLOSE_HIT_RIGHT_PADDING_PX: i32 = 2;
const CARET_STATUS_HEIGHT_PX: i32 = 22;
const CARET_STATUS_HORIZONTAL_PADDING_PX: i32 = 8;
const TREE_INDENT_PX: i32 = 18;
const TREE_ROW_HORIZONTAL_PADDING_PX: i32 = 6;
const TREE_ROW_VERTICAL_PADDING_PX: i32 = 3;
const TREE_EXPANDER_HIT_SIZE_PX: i32 = 20;
const WIN32_QUESTION_DEFAULT_RESPONSE: gtk::ResponseType = gtk::ResponseType::Yes;
const TEXT_FILE_MIB_LIMIT: usize = TEXT_FILE_BYTE_LIMIT / 1024 / 1024;
const REPLACE_ALL_OUTPUT_BYTE_LIMIT: usize = TEXT_FILE_BYTE_LIMIT;
const FIND_REPLACE_DIALOG_TEXT_CAPACITY: usize = 1024;
const FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS: usize = FIND_REPLACE_DIALOG_TEXT_CAPACITY - 1;
const DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB: usize = 64;
const DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS: usize =
    DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB * 1024 * 1024 / 2;
const THEME_CSS_CLASS_PREFIX: &str = "j3-theme-";
const FIND_MATCH_HIGHLIGHT_TAG: &str = "j3-current-find-match";
type TabContentByNodeId = HashMap<i64, (String, String)>;

pub fn run_message_loop(app: App) -> Result<(), AppError> {
    let gtk_app = gtk::Application::builder()
        .application_id(APP_LINUX_APPLICATION_ID)
        .build();
    let app_slot = Rc::new(RefCell::new(Some(app)));
    let startup_error = Rc::new(RefCell::new(None::<AppError>));

    {
        let app_slot = Rc::clone(&app_slot);
        let startup_error = Rc::clone(&startup_error);
        gtk_app.connect_activate(move |gtk_app| {
            let Some(app) = app_slot.borrow_mut().take() else {
                return;
            };

            match build_main_window(gtk_app, app) {
                Ok(window) => window.present(),
                Err(error) => {
                    *startup_error.borrow_mut() = Some(error);
                    gtk_app.quit();
                }
            }
        });
    }

    gtk_app.run();

    let result = if let Some(error) = startup_error.borrow_mut().take() {
        Err(error)
    } else {
        Ok(())
    };
    result
}

pub fn show_error_message(title: &str, message: &str) {
    if gtk::init().is_ok() {
        show_gtk_error_message(None, title, message);
    } else {
        eprintln!("{title}: {message}");
    }
}

fn configure_application_icon_name() {
    gtk::Window::set_default_icon_name(APP_LINUX_APPLICATION_ID);
}

fn install_runtime_window_icon(window: &gtk::ApplicationWindow, icon_path: Option<PathBuf>) {
    let Some(icon_path) = icon_path else {
        return;
    };
    window.connect_realize(move |window| {
        let file = gio::File::for_path(&icon_path);
        let Ok(texture) = gdk::Texture::from_file(&file) else {
            return;
        };
        let Some(surface) = window.surface() else {
            return;
        };
        let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() else {
            return;
        };
        toplevel.set_icon_list(&[texture]);
    });
}

fn install_compact_titlebar(window: &gtk::ApplicationWindow, icon_path: Option<&Path>) {
    let titlebar = gtk::WindowHandle::new();
    titlebar.add_css_class("j3-compact-titlebar");

    let layout = gtk::CenterBox::new();
    layout.add_css_class("j3-titlebar-layout");

    let start = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    start.add_css_class("j3-titlebar-start");
    let icon = if let Some(icon_path) = icon_path {
        gtk::Image::from_file(icon_path)
    } else {
        gtk::Image::from_icon_name(APP_LINUX_APPLICATION_ID)
    };
    icon.add_css_class("j3-titlebar-icon");
    icon.set_pixel_size(12);
    start.append(&icon);

    let title = gtk::Label::new(Some(APP_DISPLAY_NAME));
    title.add_css_class("j3-titlebar-label");
    title.set_single_line_mode(true);

    let controls = gtk::WindowControls::new(gtk::PackType::End);
    controls.add_css_class("j3-titlebar-controls");

    layout.set_start_widget(Some(&start));
    layout.set_center_widget(Some(&title));
    layout.set_end_widget(Some(&controls));
    titlebar.set_child(Some(&layout));
    window.set_titlebar(Some(&titlebar));
}

fn find_application_icon_path() -> Option<PathBuf> {
    find_icon_path(APP_ICON_SVG_FILE_NAME).or_else(|| find_icon_path(APP_ICON_PNG_FILE_NAME))
}

fn find_icon_path(file_name: &str) -> Option<PathBuf> {
    let executable_icon = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(file_name)));
    executable_icon.filter(|path| path.is_file()).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|path| path.join(file_name))
            .filter(|path| path.is_file())
    })
}

fn build_main_window(
    gtk_app: &gtk::Application,
    app: App,
) -> Result<gtk::ApplicationWindow, AppError> {
    let initial_settings = app.ui_settings();
    configure_application_icon_name();
    let runtime_icon_path = find_application_icon_path();
    let window_builder = gtk::ApplicationWindow::builder()
        .application(gtk_app)
        .title(app.window_title())
        .icon_name(APP_LINUX_APPLICATION_ID)
        .default_width(initial_settings.window.width)
        .default_height(initial_settings.window.height);
    let window = window_builder.build();
    install_compact_titlebar(&window, runtime_icon_path.as_deref());
    install_runtime_window_icon(&window, runtime_icon_path);
    window.add_css_class("j3-window");

    let css_provider = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let actions = Rc::new(AppActions::new(&window));
    register_global_accelerators(gtk_app);

    let menu_model = build_menu_model(initial_settings.language);
    let menu_bar = gtk::PopoverMenuBar::from_model(Some(&menu_model));
    menu_bar.add_css_class("j3-menu-bar");

    let search = gtk::SearchEntry::new();
    search.add_css_class("j3-search");
    search.set_placeholder_text(Some(ui_text(initial_settings.language).search_cue()));
    search.set_size_request(-1, SEARCH_BOX_HEIGHT_PX);
    search.set_margin_top(SEARCH_PANEL_PADDING_PX);
    search.set_margin_bottom(SEARCH_PANEL_PADDING_PX);
    search.set_margin_start(SEARCH_PANEL_PADDING_PX);
    search.set_margin_end(SEARCH_PANEL_PADDING_PX);

    let tree = gtk::ListBox::new();
    tree.set_selection_mode(gtk::SelectionMode::Single);
    tree.set_focusable(true);
    tree.add_css_class("j3-tree");

    let tree_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&tree)
        .build();
    tree_scroller.add_css_class("j3-tree-scroller");
    tree_scroller.set_hexpand(true);
    tree_scroller.set_vexpand(true);

    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left.add_css_class("j3-left-pane");
    left.set_width_request(MIN_SPLIT_WIDTH_PX);
    left.append(&search);
    left.append(&tree_scroller);

    let notebook = gtk::Notebook::new();
    notebook.set_scrollable(true);
    notebook.set_hexpand(true);
    notebook.set_vexpand(true);
    notebook.add_css_class("j3-tabs");

    let caret_status = gtk::Label::new(None);
    caret_status.add_css_class("j3-caret-status");
    caret_status.set_xalign(1.0);
    caret_status.set_size_request(-1, CARET_STATUS_HEIGHT_PX);
    caret_status.set_margin_start(CARET_STATUS_HORIZONTAL_PADDING_PX);
    caret_status.set_margin_end(CARET_STATUS_HORIZONTAL_PADDING_PX);

    let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right.add_css_class("j3-right-pane");
    right.set_hexpand(true);
    right.set_vexpand(true);
    right.set_width_request(MIN_EDITOR_WIDTH_PX);
    right.append(&notebook);
    right.append(&caret_status);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_start_child(Some(&left));
    paned.set_end_child(Some(&right));
    paned.set_resize_start_child(false);
    paned.set_shrink_start_child(false);
    paned.set_position(clamp_split_width_for_window(
        initial_settings.splitter.left_width,
        initial_settings.window.width,
    ));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("j3-root");
    root.append(&menu_bar);
    root.append(&paned);
    window.set_child(Some(&root));

    let document = UiDocument::from_active_document(app.document(), initial_settings.language)?;
    let widgets = Widgets {
        window: window.clone(),
        menu_bar,
        search,
        tree,
        notebook,
        paned,
        caret_status,
        css_provider,
    };
    let state = Rc::new(RefCell::new(LinuxState {
        app,
        document,
        tabs: OpenTabs::new(),
        tree_mode: TreeMode::Active,
        search_query: String::new(),
        selected_node_id: None,
        editing_node_id: None,
        dragging_node_id: None,
        expanded_node_ids: HashSet::new(),
        visible_node_ids: Vec::new(),
        visible_tree_row_specs: Vec::new(),
        tab_pages: Vec::new(),
        last_tree_context_point: None,
        last_editor_context_point: None,
        suppress_tree_selection: false,
        suppress_tab_change: false,
        suppress_editor_change: false,
        editor_content_pending_sync: false,
        suppress_search_change: false,
        search_generation: 0,
        ui_settings_generation: 0,
        selection_ui_settings_generation: 0,
        pending_selection_node_id: None,
        surface_size_signals_attached: false,
        find_replace_dialog: None,
        find_replace_dialog_generation: 0,
        widgets,
        actions: Rc::clone(&actions),
    }));

    connect_actions(&state);
    connect_window_signals(&state);
    apply_theme(&state)?;
    rebuild_menu_bar(&state);
    rebuild_tree_list(&state, initial_settings.selection.node_id)?;
    let initial_node_id = state.borrow().selected_node_id;
    if let Some(node_id) = initial_node_id {
        open_or_activate_tab_from_node(&state, node_id)?;
    } else {
        refresh_tabs(&state)?;
    }
    save_current_selection_ui_setting(&state)?;
    update_actions(&state);
    update_window_title(&state);

    Ok(window)
}

fn register_global_accelerators(gtk_app: &gtk::Application) {
    for (action, accelerators) in global_accelerator_bindings() {
        gtk_app.set_accels_for_action(&action, &accelerators);
    }
}

fn global_accelerator_bindings() -> Vec<(String, Vec<&'static str>)> {
    let mut bindings: Vec<(String, Vec<&'static str>)> = Vec::new();
    for shortcut in GUI_SHORTCUTS
        .iter()
        .filter(|shortcut| shortcut.scope == GuiShortcutScope::Global)
    {
        let action = shortcut.command.gtk_detailed_action().to_owned();
        if let Some((_, accelerators)) = bindings
            .iter_mut()
            .find(|(registered_action, _)| registered_action == &action)
        {
            accelerators.push(shortcut.accelerator);
        } else {
            bindings.push((action, vec![shortcut.accelerator]));
        }
    }
    bindings
}

#[derive(Clone)]
struct Widgets {
    window: gtk::ApplicationWindow,
    menu_bar: gtk::PopoverMenuBar,
    search: gtk::SearchEntry,
    tree: gtk::ListBox,
    notebook: gtk::Notebook,
    paned: gtk::Paned,
    caret_status: gtk::Label,
    css_provider: gtk::CssProvider,
}

#[derive(Clone)]
struct TabPage {
    page: gtk::ScrolledWindow,
    view: gtk::TextView,
    buffer: gtk::TextBuffer,
    node_id: Rc<Cell<i64>>,
    source: Rc<Cell<DocumentTabSource>>,
    content_revision: Rc<Cell<u64>>,
    caret_status_cache: Rc<RefCell<Option<CaretStatusCache>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaretStatusCache {
    view_width: i32,
    line_start_offset: i32,
    display_line_number: usize,
}

struct ExistingTabPageUpdate {
    page: TabPage,
    title: String,
    node_id: i64,
    source: DocumentTabSource,
    editable: bool,
    view_state: DocumentTabViewState,
    content_revision: u64,
    content: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ContextMenuPoint {
    x: f64,
    y: f64,
}

impl ContextMenuPoint {
    fn as_tuple(self) -> (f64, f64) {
        (self.x, self.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct EditorContextMenuPoint {
    node_id: i64,
    point: ContextMenuPoint,
}

struct LinuxState {
    app: App,
    document: UiDocument,
    tabs: OpenTabs,
    tree_mode: TreeMode,
    search_query: String,
    selected_node_id: Option<i64>,
    editing_node_id: Option<i64>,
    dragging_node_id: Option<i64>,
    expanded_node_ids: HashSet<i64>,
    visible_node_ids: Vec<i64>,
    visible_tree_row_specs: Vec<TreeRowSpec>,
    tab_pages: Vec<TabPage>,
    last_tree_context_point: Option<ContextMenuPoint>,
    last_editor_context_point: Option<EditorContextMenuPoint>,
    suppress_tree_selection: bool,
    suppress_tab_change: bool,
    suppress_editor_change: bool,
    editor_content_pending_sync: bool,
    suppress_search_change: bool,
    search_generation: u64,
    ui_settings_generation: u64,
    selection_ui_settings_generation: u64,
    pending_selection_node_id: Option<i64>,
    surface_size_signals_attached: bool,
    find_replace_dialog: Option<FindReplaceDialogState>,
    find_replace_dialog_generation: u64,
    widgets: Widgets,
    actions: Rc<AppActions>,
}

#[derive(Clone)]
struct AppActions {
    save: gio::SimpleAction,
    import_text: gio::SimpleAction,
    export_text: gio::SimpleAction,
    export_all_text: gio::SimpleAction,
    close_tab: gio::SimpleAction,
    close_window: gio::SimpleAction,
    undo: gio::SimpleAction,
    cut: gio::SimpleAction,
    copy: gio::SimpleAction,
    paste: gio::SimpleAction,
    delete_selection: gio::SimpleAction,
    select_all: gio::SimpleAction,
    find: gio::SimpleAction,
    replace: gio::SimpleAction,
    new_document: gio::SimpleAction,
    new_child_document: gio::SimpleAction,
    rename: gio::SimpleAction,
    move_up: gio::SimpleAction,
    move_down: gio::SimpleAction,
    delete: gio::SimpleAction,
    restore: gio::SimpleAction,
    delete_permanently: gio::SimpleAction,
    tree_mode: gio::SimpleAction,
    import_encoding: gio::SimpleAction,
    export_encoding: gio::SimpleAction,
    theme: gio::SimpleAction,
    language: gio::SimpleAction,
    word_wrap: gio::SimpleAction,
    editor_font: gio::SimpleAction,
    about: gio::SimpleAction,
}

impl AppActions {
    fn new(window: &gtk::ApplicationWindow) -> Self {
        let actions = Self::build();

        for action in actions.all() {
            window.add_action(action);
        }

        actions
    }

    fn build() -> Self {
        Self {
            save: simple_action(GuiCommand::SaveDocument.gtk_action_name()),
            import_text: simple_action(GuiCommand::ImportText.gtk_action_name()),
            export_text: simple_action(GuiCommand::ExportText.gtk_action_name()),
            export_all_text: simple_action(GuiCommand::ExportAllText.gtk_action_name()),
            close_tab: simple_action(GuiCommand::CloseTab.gtk_action_name()),
            close_window: simple_action(GuiCommand::CloseWindow.gtk_action_name()),
            undo: simple_action(GuiCommand::Undo.gtk_action_name()),
            cut: simple_action(GuiCommand::Cut.gtk_action_name()),
            copy: simple_action(GuiCommand::Copy.gtk_action_name()),
            paste: simple_action(GuiCommand::Paste.gtk_action_name()),
            delete_selection: simple_action(GuiCommand::DeleteSelection.gtk_action_name()),
            select_all: simple_action(GuiCommand::SelectAll.gtk_action_name()),
            find: simple_action(GuiCommand::FindText.gtk_action_name()),
            replace: simple_action(GuiCommand::ReplaceText.gtk_action_name()),
            new_document: simple_action(GuiCommand::NewDocument.gtk_action_name()),
            new_child_document: simple_action(GuiCommand::NewChildDocument.gtk_action_name()),
            rename: simple_action(GuiCommand::Rename.gtk_action_name()),
            move_up: simple_action(GuiCommand::MoveUp.gtk_action_name()),
            move_down: simple_action(GuiCommand::MoveDown.gtk_action_name()),
            delete: simple_action(GuiCommand::MoveToTrash.gtk_action_name()),
            restore: simple_action(GuiCommand::Restore.gtk_action_name()),
            delete_permanently: simple_action(GuiCommand::DeletePermanently.gtk_action_name()),
            tree_mode: stateful_string_action(
                GuiCommand::ShowActiveTree.gtk_action_name(),
                "active",
            ),
            import_encoding: stateful_string_action(
                GuiOptionMenu::ImportEncoding.gtk_action_name(),
                TextEncoding::default_import().storage_value(),
            ),
            export_encoding: stateful_string_action(
                GuiOptionMenu::ExportEncoding.gtk_action_name(),
                TextEncoding::default_export().storage_value(),
            ),
            theme: stateful_string_action(
                GuiOptionMenu::Theme.gtk_action_name(),
                AppearanceTheme::Light.storage_value(),
            ),
            language: stateful_string_action(
                GuiOptionMenu::Language.gtk_action_name(),
                UiLanguage::English.storage_value(),
            ),
            word_wrap: gio::SimpleAction::new_stateful(
                GuiCommand::WordWrap.gtk_action_name(),
                None,
                &true.to_variant(),
            ),
            editor_font: simple_action(GuiCommand::EditorFont.gtk_action_name()),
            about: simple_action(GuiCommand::About.gtk_action_name()),
        }
    }

    fn all(&self) -> [&gio::SimpleAction; 30] {
        [
            &self.save,
            &self.import_text,
            &self.export_text,
            &self.export_all_text,
            &self.close_tab,
            &self.close_window,
            &self.undo,
            &self.cut,
            &self.copy,
            &self.paste,
            &self.delete_selection,
            &self.select_all,
            &self.find,
            &self.replace,
            &self.new_document,
            &self.new_child_document,
            &self.rename,
            &self.move_up,
            &self.move_down,
            &self.delete,
            &self.restore,
            &self.delete_permanently,
            &self.tree_mode,
            &self.import_encoding,
            &self.export_encoding,
            &self.theme,
            &self.language,
            &self.word_wrap,
            &self.editor_font,
            &self.about,
        ]
    }
}

fn simple_action(name: &str) -> gio::SimpleAction {
    gio::SimpleAction::new(name, None)
}

fn stateful_string_action(name: &str, initial: &str) -> gio::SimpleAction {
    gio::SimpleAction::new_stateful(
        name,
        Some(&String::static_variant_type()),
        &initial.to_variant(),
    )
}

fn build_menu_model(language: UiLanguage) -> gio::Menu {
    let text = ui_text(language);
    let root = gio::Menu::new();

    for spec in MAIN_MENU_SPECS {
        let menu = build_menu_entries(spec.entries, text);
        append_submenu(&root, menu_label(text, spec.kind), menu);
    }

    root
}

fn build_menu_entries(entries: &[GuiMenuEntry], text: UiText) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();

    for entry in entries {
        match *entry {
            GuiMenuEntry::Command(command) => {
                append_item(
                    &section,
                    command_label(text, command),
                    command.gtk_detailed_action(),
                );
            }
            GuiMenuEntry::Separator => {
                append_section_if_not_empty(&menu, &section);
                section = gio::Menu::new();
            }
            GuiMenuEntry::OptionMenu(option_menu) => {
                append_submenu(
                    &section,
                    option_menu_label(text, option_menu),
                    build_option_menu(option_menu, text),
                );
            }
        }
    }

    append_section_if_not_empty(&menu, &section);
    menu
}

fn append_section_if_not_empty(menu: &gio::Menu, section: &gio::Menu) {
    if section.n_items() > 0 {
        menu.append_section(None, section);
    }
}

fn build_option_menu(option_menu: GuiOptionMenu, text: UiText) -> gio::Menu {
    match option_menu {
        GuiOptionMenu::ImportEncoding => build_encoding_menu(GuiOptionMenu::ImportEncoding, text),
        GuiOptionMenu::ExportEncoding => build_encoding_menu(GuiOptionMenu::ExportEncoding, text),
        GuiOptionMenu::Theme => build_theme_menu(text),
        GuiOptionMenu::Language => build_language_menu(),
    }
}

fn build_encoding_menu(option_menu: GuiOptionMenu, text: UiText) -> gio::Menu {
    let menu = gio::Menu::new();
    let encodings = match option_menu {
        GuiOptionMenu::ImportEncoding => TextEncoding::import_options(),
        GuiOptionMenu::ExportEncoding => TextEncoding::export_options(),
        GuiOptionMenu::Theme | GuiOptionMenu::Language => &[],
    };

    for encoding in encodings {
        if let Some(action) = option_menu.gtk_detailed_action_for_encoding(*encoding) {
            append_item(&menu, text.text_encoding_name(*encoding), &action);
        }
    }
    menu
}

fn build_theme_menu(text: UiText) -> gio::Menu {
    let menu = gio::Menu::new();
    for theme in AppearanceTheme::options() {
        if let Some(action) = GuiOptionMenu::Theme.gtk_detailed_action_for_theme(*theme) {
            append_item(&menu, text.theme_name(*theme), &action);
        }
    }
    menu
}

fn build_language_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for language in UiLanguage::options() {
        if let Some(action) = GuiOptionMenu::Language.gtk_detailed_action_for_language(*language) {
            append_item(&menu, language.display_name(), &action);
        }
    }
    menu
}

fn menu_label(text: UiText, menu: GuiMenuKind) -> &'static str {
    match menu {
        GuiMenuKind::File => text.menu_file(),
        GuiMenuKind::Edit => text.menu_edit(),
        GuiMenuKind::Document => text.menu_document(),
        GuiMenuKind::View => text.menu_view(),
        GuiMenuKind::Help => text.menu_help(),
    }
}

fn option_menu_label(text: UiText, option_menu: GuiOptionMenu) -> &'static str {
    match option_menu {
        GuiOptionMenu::ImportEncoding => text.import_encoding(),
        GuiOptionMenu::ExportEncoding => text.export_encoding(),
        GuiOptionMenu::Theme => text.theme(),
        GuiOptionMenu::Language => text.language(),
    }
}

fn command_label(text: UiText, command: GuiCommand) -> &'static str {
    match command {
        GuiCommand::SaveDocument => text.save_document(),
        GuiCommand::ImportText => text.import_text(),
        GuiCommand::ExportText => text.export_text(),
        GuiCommand::ExportAllText => text.export_all_text(),
        GuiCommand::CloseTab => text.close_tab(),
        GuiCommand::CloseWindow => text.close_window(),
        GuiCommand::Undo => text.undo(),
        GuiCommand::Cut => text.cut(),
        GuiCommand::Copy => text.copy(),
        GuiCommand::Paste => text.paste(),
        GuiCommand::DeleteSelection => text.delete_selection(),
        GuiCommand::SelectAll => text.select_all(),
        GuiCommand::FindText => text.find_text(),
        GuiCommand::ReplaceText => text.replace_text(),
        GuiCommand::NewDocument => text.new_document(),
        GuiCommand::NewChildDocument => text.new_child_document(),
        GuiCommand::Rename => text.rename(),
        GuiCommand::MoveUp => text.move_up(),
        GuiCommand::MoveDown => text.move_down(),
        GuiCommand::MoveToTrash => text.move_to_trash(),
        GuiCommand::Restore => text.restore(),
        GuiCommand::DeletePermanently => text.delete_permanently(),
        GuiCommand::ShowActiveTree => text.document_tree(),
        GuiCommand::ShowTrash => text.trash(),
        GuiCommand::WordWrap => text.word_wrap(),
        GuiCommand::EditorFont => text.editor_font(),
        GuiCommand::About => text.about_menu(),
    }
}

fn append_item(menu: &gio::Menu, label: &str, detailed_action: &str) {
    menu.append(Some(label), Some(detailed_action));
}

fn append_submenu(menu: &gio::Menu, label: &str, submenu: gio::Menu) {
    menu.append_submenu(Some(label), &submenu);
}

macro_rules! for_each_simple_action_binding {
    ($visit:ident) => {
        $visit!(save, GuiCommand::SaveDocument, save_current_document_action);
        $visit!(import_text, GuiCommand::ImportText, import_text_action);
        $visit!(export_text, GuiCommand::ExportText, export_text_action);
        $visit!(
            export_all_text,
            GuiCommand::ExportAllText,
            export_all_text_action
        );
        $visit!(close_tab, GuiCommand::CloseTab, close_active_tab_action);
        $visit!(close_window, GuiCommand::CloseWindow, close_window_action);
        $visit!(undo, GuiCommand::Undo, editor_undo_action);
        $visit!(cut, GuiCommand::Cut, editor_cut_action);
        $visit!(copy, GuiCommand::Copy, editor_copy_action);
        $visit!(paste, GuiCommand::Paste, editor_paste_action);
        $visit!(
            delete_selection,
            GuiCommand::DeleteSelection,
            editor_delete_selection_action
        );
        $visit!(select_all, GuiCommand::SelectAll, editor_select_all_action);
        $visit!(find, GuiCommand::FindText, find_action);
        $visit!(replace, GuiCommand::ReplaceText, replace_action);
        $visit!(new_document, GuiCommand::NewDocument, new_document_action);
        $visit!(
            new_child_document,
            GuiCommand::NewChildDocument,
            new_child_document_action
        );
        $visit!(rename, GuiCommand::Rename, rename_action);
        $visit!(move_up, GuiCommand::MoveUp, move_up_action);
        $visit!(move_down, GuiCommand::MoveDown, move_down_action);
        $visit!(delete, GuiCommand::MoveToTrash, delete_node_action);
        $visit!(restore, GuiCommand::Restore, restore_node_action);
        $visit!(
            delete_permanently,
            GuiCommand::DeletePermanently,
            delete_permanently_action
        );
        $visit!(editor_font, GuiCommand::EditorFont, editor_font_action);
        $visit!(about, GuiCommand::About, about_action);
    };
}

macro_rules! for_each_stateful_string_action_binding {
    ($visit:ident) => {
        $visit!(
            tree_mode,
            GuiCommand::ShowActiveTree.gtk_action_name(),
            tree_mode_action
        );
        $visit!(
            import_encoding,
            GuiOptionMenu::ImportEncoding.gtk_action_name(),
            import_encoding_action
        );
        $visit!(
            export_encoding,
            GuiOptionMenu::ExportEncoding.gtk_action_name(),
            export_encoding_action
        );
        $visit!(theme, GuiOptionMenu::Theme.gtk_action_name(), theme_action);
        $visit!(
            language,
            GuiOptionMenu::Language.gtk_action_name(),
            language_action
        );
    };
}

macro_rules! for_each_bool_stateful_action_binding {
    ($visit:ident) => {
        $visit!(word_wrap, GuiCommand::WordWrap, toggle_word_wrap_action);
    };
}

fn strip_menu_accelerator(label: &str) -> String {
    label.split('\t').next().unwrap_or(label).to_owned()
}

fn connect_actions(state: &Rc<RefCell<LinuxState>>) {
    macro_rules! connect_simple_binding {
        ($field:ident, $command:expr, $handler:path) => {{
            let action = state.borrow().actions.$field.clone();
            debug_assert_eq!(action.name().as_str(), $command.gtk_action_name());
            connect_simple_action(state, &action, $handler);
        }};
    }
    for_each_simple_action_binding!(connect_simple_binding);

    macro_rules! connect_stateful_string_binding {
        ($field:ident, $action_name:expr, $handler:path) => {{
            let action = state.borrow().actions.$field.clone();
            debug_assert_eq!(action.name().as_str(), $action_name);
            connect_stateful_string_action(state, &action, $handler);
        }};
    }
    for_each_stateful_string_action_binding!(connect_stateful_string_binding);

    macro_rules! connect_bool_stateful_binding {
        ($field:ident, $command:expr, $handler:path) => {{
            let state = Rc::clone(state);
            let action = state.borrow().actions.$field.clone();
            debug_assert_eq!(action.name().as_str(), $command.gtk_action_name());
            action.connect_activate(move |_, _| {
                run_and_report(&state, $handler(&state));
            });
        }};
    }
    for_each_bool_stateful_action_binding!(connect_bool_stateful_binding);
}

fn connect_simple_action(
    state: &Rc<RefCell<LinuxState>>,
    action: &gio::SimpleAction,
    handler: fn(&Rc<RefCell<LinuxState>>) -> Result<(), AppError>,
) {
    let state = Rc::clone(state);
    action.connect_activate(move |_, _| run_and_report(&state, handler(&state)));
}

fn connect_stateful_string_action(
    state: &Rc<RefCell<LinuxState>>,
    action: &gio::SimpleAction,
    handler: fn(&Rc<RefCell<LinuxState>>, &str) -> Result<(), AppError>,
) {
    let state = Rc::clone(state);
    action.connect_activate(move |_, target| {
        let value = target.and_then(|target| target.str()).unwrap_or_default();
        run_and_report(&state, handler(&state, value));
    });
}

fn connect_window_signals(state: &Rc<RefCell<LinuxState>>) {
    {
        let state = Rc::clone(state);
        let tree = state.borrow().widgets.tree.clone();
        tree.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                run_and_report(&state, handle_tree_row_selected(&state, row.index()));
            }
        });
    }

    {
        let state = Rc::clone(state);
        let search = state.borrow().widgets.search.clone();
        search.connect_search_changed(move |entry| {
            if state.borrow().suppress_search_change {
                return;
            }
            handle_search_changed(&state, entry.text().to_string());
        });
    }

    {
        let state = Rc::clone(state);
        let notebook = state.borrow().widgets.notebook.clone();
        notebook.connect_switch_page(move |_, page, page_num| {
            run_and_report(&state, handle_tab_switched(&state, page, page_num));
        });
    }

    {
        let state = Rc::clone(state);
        let notebook = state.borrow().widgets.notebook.clone();
        notebook.connect_page_reordered(move |_, page, page_num| {
            run_and_report(&state, handle_tab_reordered(&state, page, page_num));
        });
    }

    {
        let state = Rc::clone(state);
        let window = state.borrow().widgets.window.clone();
        window.connect_close_request(move |_| match handle_window_close(&state) {
            Ok(true) => glib::Propagation::Proceed,
            Ok(false) => glib::Propagation::Stop,
            Err(error) => {
                report_error(&state, &error);
                glib::Propagation::Stop
            }
        });
    }

    {
        let state = Rc::clone(state);
        let window = state.borrow().widgets.window.clone();
        let controller = gtk::EventControllerLegacy::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        controller.connect_event(move |_, event| {
            let Some((x, y)) = secondary_button_press_position(event) else {
                return glib::Propagation::Proceed;
            };
            if show_editor_context_menu_from_window_position(&state, x, y) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(controller);
    }

    {
        let state = Rc::clone(state);
        let window = state.borrow().widgets.window.clone();
        window.connect_realize(move |window| {
            attach_window_surface_ui_settings_signals(&state, window);
        });
    }

    {
        let state = Rc::clone(state);
        let paned = state.borrow().widgets.paned.clone();
        paned.connect_position_notify(move |_| {
            schedule_ui_settings_save(&state);
        });
    }

    {
        let state = Rc::clone(state);
        let tree = state.borrow().widgets.tree.clone();
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            let handled = handle_tree_key_pressed(&state, key, modifiers);
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        tree.add_controller(controller);
    }

    {
        let state = Rc::clone(state);
        let tree = state.borrow().widgets.tree.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| {
            remember_tree_context_menu_point(&state, x, y);
        });
        tree.add_controller(motion);
    }

    {
        let state = Rc::clone(state);
        let tree = state.borrow().widgets.tree.clone();
        let gesture = context_menu_click_gesture();
        let tree_for_focus = tree.clone();
        gesture.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            tree_for_focus.grab_focus();
            remember_tree_context_menu_point(&state, x, y);
            run_and_report(&state, show_tree_context_menu_at_position(&state, x, y));
        });
        tree.add_controller(gesture);
    }

    {
        let state = Rc::clone(state);
        let tree = state.borrow().widgets.tree.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        let tree_for_focus = tree.clone();
        gesture.connect_pressed(move |_, press_count, _, y| {
            tree_for_focus.grab_focus();
            if press_count == 2 {
                run_and_report(&state, toggle_tree_row_at_y(&state, y));
            }
        });
        tree.add_controller(gesture);
    }
}

fn run_and_report(state: &Rc<RefCell<LinuxState>>, result: Result<(), AppError>) {
    if let Err(error) = result {
        report_error(state, &error);
    }
}

fn attach_window_surface_ui_settings_signals(
    state: &Rc<RefCell<LinuxState>>,
    window: &gtk::ApplicationWindow,
) {
    {
        let mut state = state.borrow_mut();
        if state.surface_size_signals_attached {
            return;
        }
        state.surface_size_signals_attached = true;
    }

    let Some(native) = window.native() else {
        state.borrow_mut().surface_size_signals_attached = false;
        return;
    };
    let Some(surface) = native.surface() else {
        state.borrow_mut().surface_size_signals_attached = false;
        return;
    };

    {
        let state = Rc::clone(state);
        surface.connect_width_notify(move |_| {
            schedule_ui_settings_save(&state);
        });
    }
    {
        let state = Rc::clone(state);
        surface.connect_height_notify(move |_| {
            schedule_ui_settings_save(&state);
        });
    }
}

fn schedule_ui_settings_save(state: &Rc<RefCell<LinuxState>>) {
    let generation = {
        let mut state = state.borrow_mut();
        state.ui_settings_generation = state.ui_settings_generation.saturating_add(1);
        state.ui_settings_generation
    };
    let state = Rc::clone(state);
    glib::timeout_add_local_once(
        Duration::from_millis(UI_SETTINGS_SAVE_DEBOUNCE_MS),
        move || {
            let is_current = state.borrow().ui_settings_generation == generation;
            if !is_current {
                return;
            }
            let result = persist_ui_settings(&state);
            run_and_report(&state, result);
        },
    );
}

fn report_error(state: &Rc<RefCell<LinuxState>>, error: &AppError) {
    let (window, language) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
        )
    };
    eprintln!("detail: {error}");
    show_gtk_error_message(
        Some(window.upcast_ref()),
        "j3TreeText",
        &app_error_user_message(error, language),
    );
}

fn app_error_user_message(error: &AppError, language: UiLanguage) -> String {
    match error {
        AppError::DatabaseOpen { path, .. } => match language {
            UiLanguage::Korean => format!(
                "문서 DB를 열 수 없습니다.\n경로: {}\n쓰기 권한, 파일 잠금, 디스크 용량을 확인하세요.",
                path.display()
            ),
            UiLanguage::English => format!(
                "Cannot open the document database.\nPath: {}\nCheck write permissions, file locks, and disk space.",
                path.display()
            ),
        },
        AppError::Domain(error) => domain_error_user_message(error, language),
        AppError::Io {
            user_message: IoUserMessage::ReadTextFile,
            ..
        } => match language {
            UiLanguage::Korean => {
                "텍스트 파일을 읽을 수 없습니다. 경로와 권한을 확인하세요.".to_owned()
            }
            UiLanguage::English => {
                "Cannot read the text file. Check the path and permissions.".to_owned()
            }
        },
        AppError::Io {
            user_message: IoUserMessage::WriteTextFile,
            ..
        } => match language {
            UiLanguage::Korean => {
                "텍스트 파일을 저장할 수 없습니다. 경로, 권한, 디스크 용량을 확인하세요."
                    .to_owned()
            }
            UiLanguage::English => {
                "Cannot save the text file. Check the path, permissions, and disk space.".to_owned()
            }
        },
        AppError::Io { .. } => match language {
            UiLanguage::Korean => "파일 작업에 실패했습니다. 쓰기 권한을 확인하세요.".to_owned(),
            UiLanguage::English => {
                "The file operation failed. Check write permissions.".to_owned()
            }
        },
        AppError::Platform { user_message, .. } => {
            platform_error_user_message(*user_message, language)
        }
        AppError::Sqlite {
            user_message: SqliteUserMessage::SaveDocumentContent,
            ..
        } => match language {
            UiLanguage::Korean => {
                "문서를 저장할 수 없습니다. DB 파일 권한과 디스크 용량을 확인하세요.".to_owned()
            }
            UiLanguage::English => {
                "Cannot save the document. Check DB file permissions and disk space.".to_owned()
            }
        },
        AppError::Sqlite { .. } => match language {
            UiLanguage::Korean => "문서 DB를 열거나 초기화할 수 없습니다.".to_owned(),
            UiLanguage::English => "Cannot open or initialize the document database.".to_owned(),
        },
        AppError::TextEncoding {
            user_message: TextEncodingUserMessage::Encode,
            encoding,
            ..
        } => match language {
            UiLanguage::Korean => format!(
                "선택한 인코딩({})으로 저장할 수 없는 문자가 있습니다.\nUTF-8 또는 UTF-16으로 내보내세요.",
                ui_text(language).text_encoding_name(*encoding)
            ),
            UiLanguage::English => format!(
                "Some characters cannot be saved with the selected encoding ({}).\nExport with UTF-8 or UTF-16.",
                ui_text(language).text_encoding_name(*encoding)
            ),
        },
        AppError::TextEncoding { encoding, .. } => match language {
            UiLanguage::Korean => format!(
                "선택한 인코딩({})으로 읽을 수 없습니다.\n다른 인코딩을 선택하세요.",
                ui_text(language).text_encoding_name(*encoding)
            ),
            UiLanguage::English => format!(
                "Cannot read this file with the selected encoding ({}).\nChoose a different encoding.",
                ui_text(language).text_encoding_name(*encoding)
            ),
        },
        AppError::TextFileTooLarge {
            user_message,
            limit_mib,
        } => text_file_too_large_user_message(*user_message, *limit_mib, language),
        AppError::User { message } => message.clone(),
    }
}

fn domain_error_user_message(error: &DomainError, language: UiLanguage) -> String {
    match error {
        DomainError::CannotDeleteRoot => match language {
            UiLanguage::Korean => "루트 문서는 삭제할 수 없습니다.".to_owned(),
            UiLanguage::English => "The root document cannot be deleted.".to_owned(),
        },
        DomainError::CannotMoveNodeIntoDescendant { .. }
        | DomainError::CannotMoveNodeIntoItself { .. } => match language {
            UiLanguage::Korean => "자기 자신이나 하위 문서로 이동할 수 없습니다.".to_owned(),
            UiLanguage::English => {
                "Cannot move a document into itself or its descendants.".to_owned()
            }
        },
        DomainError::CannotMoveRoot => match language {
            UiLanguage::Korean => "루트 문서는 이동할 수 없습니다.".to_owned(),
            UiLanguage::English => "The root document cannot be moved.".to_owned(),
        },
        DomainError::DocumentSaveConflict { .. } => match language {
            UiLanguage::Korean => {
                "다른 곳에서 먼저 저장되었습니다. 다시 불러오거나 새 문서로 저장하세요."
                    .to_owned()
            }
            UiLanguage::English => {
                "This document was saved elsewhere first. Reload it or save your content as a new document."
                    .to_owned()
            }
        },
        DomainError::DuplicateSiblingTitle { .. } => match language {
            UiLanguage::Korean => "같은 위치에 같은 이름이 있습니다.".to_owned(),
            UiLanguage::English => {
                "A document with the same name already exists in this location.".to_owned()
            }
        },
        DomainError::EmptyTitle { .. } | DomainError::EmptyTitleInput => match language {
            UiLanguage::Korean => "이름을 입력하세요.".to_owned(),
            UiLanguage::English => "Enter a name.".to_owned(),
        },
        DomainError::NodeNotFound { .. } => match language {
            UiLanguage::Korean => "선택한 문서를 찾을 수 없습니다. 트리를 새로 고치세요.".to_owned(),
            UiLanguage::English => "The selected document was not found. Refresh the tree.".to_owned(),
        },
        DomainError::NodeNotDeleted { .. } => match language {
            UiLanguage::Korean => "선택한 문서가 휴지통에 없습니다.".to_owned(),
            UiLanguage::English => "The selected document is not in the trash.".to_owned(),
        },
        _ => match language {
            UiLanguage::Korean => "문서 데이터가 올바르지 않습니다.".to_owned(),
            UiLanguage::English => "The document data is not valid.".to_owned(),
        },
    }
}

fn platform_error_user_message(user_message: PlatformUserMessage, language: UiLanguage) -> String {
    match user_message {
        PlatformUserMessage::DesktopUiUnsupported => match language {
            UiLanguage::Korean => "현재 플랫폼에서 데스크톱 UI를 사용할 수 없습니다.".to_owned(),
            UiLanguage::English => {
                "The desktop UI is not available on the current platform.".to_owned()
            }
        },
        PlatformUserMessage::Font => match language {
            UiLanguage::Korean => "글꼴을 적용할 수 없습니다. 다른 글꼴을 선택하세요.".to_owned(),
            UiLanguage::English => "Cannot apply the font. Choose a different font.".to_owned(),
        },
        PlatformUserMessage::Win32Startup | PlatformUserMessage::RichEditStartup => {
            match language {
                UiLanguage::Korean => "데스크톱 UI를 시작하지 못했습니다.".to_owned(),
                UiLanguage::English => "Could not start the desktop UI.".to_owned(),
            }
        }
        PlatformUserMessage::Generic => match language {
            UiLanguage::Korean => "Linux UI 오류입니다. 다시 시도하세요.".to_owned(),
            UiLanguage::English => "Linux UI error. Try again.".to_owned(),
        },
    }
}

fn text_file_too_large_user_message(
    user_message: TextFileTooLargeUserMessage,
    limit_mib: usize,
    language: UiLanguage,
) -> String {
    match (language, user_message) {
        (UiLanguage::Korean, TextFileTooLargeUserMessage::Import) => {
            format!("가져올 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요.")
        }
        (UiLanguage::Korean, TextFileTooLargeUserMessage::Export) => {
            format!("내보낼 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요.")
        }
        (UiLanguage::Korean, TextFileTooLargeUserMessage::Generic) => {
            format!("텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요.")
        }
        (UiLanguage::English, TextFileTooLargeUserMessage::Import) => format!(
            "The text to import is too large. Split it into {limit_mib}MiB or smaller chunks and try again."
        ),
        (UiLanguage::English, TextFileTooLargeUserMessage::Export) => format!(
            "The text to export is too large. Split it into {limit_mib}MiB or smaller chunks and try again."
        ),
        (UiLanguage::English, TextFileTooLargeUserMessage::Generic) => format!(
            "The text is too large. Split it into {limit_mib}MiB or smaller chunks and try again."
        ),
    }
}

fn rebuild_menu_bar(state: &Rc<RefCell<LinuxState>>) {
    let (menu_bar, language) = {
        let state = state.borrow();
        (
            state.widgets.menu_bar.clone(),
            state.app.ui_settings().language,
        )
    };
    menu_bar.set_menu_model(Some(&build_menu_model(language)));
}

fn update_actions(state: &Rc<RefCell<LinuxState>>) {
    let state = state.borrow();
    let actions = &state.actions;
    let active_tab = state.tabs.active();
    let active_tab_exists = active_tab.is_some();
    let editable = active_tab.is_some_and(|tab| tab.editable);
    let active_page = active_page(&state);
    let has_text = editor_has_text_for_menu_state(
        active_tab.map(|tab| tab.content.as_str()),
        active_page.map(|page| page.buffer.end_iter().offset()),
    );
    let (has_selection, can_undo) = active_page
        .map(|page| {
            (
                page.buffer
                    .selection_bounds()
                    .is_some_and(|(start, end)| start.offset() != end.offset()),
                page.buffer.can_undo(),
            )
        })
        .unwrap_or((false, false));
    let search_active =
        state.tree_mode == TreeMode::Active && !state.search_query.trim().is_empty();
    let selected_node_id = existing_selected_node_id(&state.document, state.selected_node_id);
    let (move_up_enabled, move_down_enabled) =
        if state.tree_mode == TreeMode::Active && !search_active {
            sibling_move_availability(&state.document, selected_node_id)
        } else {
            (false, false)
        };

    let command_availability = GuiCommandAvailability::for_context(
        match state.tree_mode {
            TreeMode::Active => GuiTreeMode::Active,
            TreeMode::Trash => GuiTreeMode::Trash,
        },
        search_active,
        selected_node_id,
        move_up_enabled,
        move_down_enabled,
        active_tab_exists,
    );
    let editor_availability = GuiEditorAvailability::for_context(
        active_tab_exists,
        editable,
        has_selection,
        can_undo,
        has_text,
    );

    apply_action_enabled_states(actions, command_availability, editor_availability);

    actions.suppress_state_updates(|| {
        actions.tree_mode.set_state(
            &match state.tree_mode {
                TreeMode::Active => "active",
                TreeMode::Trash => "trash",
            }
            .to_variant(),
        );
        actions.import_encoding.set_state(
            &state
                .app
                .ui_settings()
                .text_encoding
                .import_encoding
                .storage_value()
                .to_variant(),
        );
        actions.export_encoding.set_state(
            &state
                .app
                .ui_settings()
                .text_encoding
                .export_encoding
                .storage_value()
                .to_variant(),
        );
        actions.theme.set_state(
            &state
                .app
                .ui_settings()
                .appearance
                .theme
                .storage_value()
                .to_variant(),
        );
        actions.language.set_state(
            &state
                .app
                .ui_settings()
                .language
                .storage_value()
                .to_variant(),
        );
        actions
            .word_wrap
            .set_state(&state.app.ui_settings().editor.word_wrap.to_variant());
    });
}

fn apply_action_enabled_states(
    actions: &AppActions,
    command_availability: GuiCommandAvailability,
    editor_availability: GuiEditorAvailability,
) {
    actions.save.set_enabled(command_availability.save_enabled);
    actions.import_text.set_enabled(true);
    actions.export_text.set_enabled(true);
    actions.export_all_text.set_enabled(true);
    actions
        .close_tab
        .set_enabled(command_availability.close_tab_enabled);
    actions.close_window.set_enabled(true);
    actions.undo.set_enabled(editor_availability.undo_enabled);
    actions.cut.set_enabled(editor_availability.cut_enabled);
    actions.copy.set_enabled(editor_availability.copy_enabled);
    actions.paste.set_enabled(editor_availability.paste_enabled);
    actions
        .delete_selection
        .set_enabled(editor_availability.delete_enabled);
    actions
        .select_all
        .set_enabled(editor_availability.select_all_enabled);
    actions
        .find
        .set_enabled(editor_availability.find_replace_enabled);
    actions
        .replace
        .set_enabled(editor_availability.find_replace_enabled);

    actions
        .new_document
        .set_enabled(command_availability.new_document_enabled);
    actions
        .new_child_document
        .set_enabled(command_availability.new_child_document_enabled);
    actions
        .rename
        .set_enabled(command_availability.rename_enabled);
    actions
        .move_up
        .set_enabled(command_availability.move_up_enabled);
    actions
        .move_down
        .set_enabled(command_availability.move_down_enabled);
    actions
        .delete
        .set_enabled(command_availability.delete_enabled);
    actions
        .restore
        .set_enabled(command_availability.restore_enabled);
    actions
        .delete_permanently
        .set_enabled(command_availability.delete_permanently_enabled);
    actions.tree_mode.set_enabled(true);
    actions.import_encoding.set_enabled(true);
    actions.export_encoding.set_enabled(true);
    actions.theme.set_enabled(true);
    actions.language.set_enabled(true);
    actions.word_wrap.set_enabled(true);
    actions.editor_font.set_enabled(true);
    actions.about.set_enabled(true);
}

fn editor_has_text_for_menu_state(
    tab_content: Option<&str>,
    live_editor_end_offset: Option<i32>,
) -> bool {
    live_editor_end_offset
        .map(|offset| offset > 0)
        .unwrap_or_else(|| tab_content.is_some_and(|content| !content.is_empty()))
}

trait ActionStateBatch {
    fn suppress_state_updates(&self, run: impl FnOnce());
}

impl ActionStateBatch for AppActions {
    fn suppress_state_updates(&self, run: impl FnOnce()) {
        run();
    }
}

fn existing_selected_node_id(document: &UiDocument, selected_node_id: Option<i64>) -> Option<i64> {
    selected_node_id.filter(|node_id| document.node_by_id(*node_id).is_some())
}

fn update_window_title(state: &Rc<RefCell<LinuxState>>) {
    let state = state.borrow();
    let text = ui_text(state.app.ui_settings().language);
    let mut title = state.app.window_title().to_owned();
    match state.tree_mode {
        TreeMode::Trash => {
            title.push_str(" - ");
            title.push_str(text.window_trash_suffix());
        }
        TreeMode::Active if !state.search_query.trim().is_empty() => {
            title.push_str(" - ");
            title.push_str(text.window_search_suffix());
        }
        TreeMode::Active => {}
    }
    if let Some(tab) = state.tabs.active() {
        title.push_str(" - ");
        title.push_str(&tab.display_title());
    }
    state.widgets.window.set_title(Some(&title));
}

fn active_page(state: &LinuxState) -> Option<&TabPage> {
    let index = state.tabs.active_index()?;
    state.tab_pages.get(index)
}

fn active_page_cloned(state: &Rc<RefCell<LinuxState>>) -> Option<TabPage> {
    let state = state.borrow();
    active_page(&state).cloned()
}

fn save_current_document_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    save_current_document(state).map(|_| ())
}

fn import_text_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_active_editable_document_for_import(state)?;
    let (window, language, encoding) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
            state.app.ui_settings().text_encoding.import_encoding,
        )
    };
    let Some(path) = choose_text_file(window.upcast_ref(), language, FileDialogMode::Import)?
    else {
        return Ok(());
    };

    if !resolve_active_dirty_before_import(state)? {
        update_actions(state);
        return Ok(());
    }
    ensure_active_editable_document_for_import(state)?;

    let mut decoded = {
        let state = state.borrow();
        state.app.import_text_file(&path, encoding)?
    };
    prepare_imported_text(&mut decoded.content, language)?;

    {
        let mut state = state.borrow_mut();
        if !state.tabs.import_active_content(decoded.content) {
            return Err(AppError::user(ui_text(language).open_import_document()));
        }
    }

    refresh_tabs(state)?;
    reload_active_editor_page_from_tab_state(state);
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn export_text_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    sync_active_editor_content(state)?;
    let (window, language, encoding) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
            state.app.ui_settings().text_encoding.export_encoding,
        )
    };
    let Some(path) = choose_text_file(window.upcast_ref(), language, FileDialogMode::Export)?
    else {
        return Ok(());
    };
    let result = {
        let state = state.borrow();
        let Some(tab) = state.tabs.active() else {
            return Err(AppError::user(ui_text(language).open_export_document()));
        };
        state.app.export_text_file(&path, encoding, &tab.content)
    };
    result.map_err(|error| match error {
        AppError::TextFileTooLarge { .. } => {
            AppError::user(ui_text(language).export_text_too_large(TEXT_FILE_MIB_LIMIT))
        }
        error => error,
    })
}

fn export_all_text_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    sync_active_editor_content(state)?;
    let (window, language, encoding) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
            state.app.ui_settings().text_encoding.export_encoding,
        )
    };
    let Some(directory) = choose_text_folder(window.upcast_ref(), language)? else {
        return Ok(());
    };
    let count = {
        let state = state.borrow();
        let overrides = dirty_active_tab_content_overrides(&state);
        state
            .app
            .export_all_text_files(&directory, encoding, &overrides)?
    };
    let path = directory.display().to_string();
    let text = ui_text(language);
    show_info_message(
        Some(window.upcast_ref()),
        text.export_all_complete_title(),
        &text.export_all_complete_message(count, &path),
    );
    Ok(())
}

fn dirty_active_tab_content_overrides(state: &LinuxState) -> HashMap<i64, &str> {
    state
        .tabs
        .tabs()
        .iter()
        .filter(|tab| tab.editable && tab.dirty)
        .map(|tab| (tab.node_id, tab.content.as_str()))
        .collect()
}

fn close_active_tab_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    close_active_tab(state)
}

fn close_window_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if handle_window_close(state)? {
        destroy_window_after_close_policy(state);
    }
    Ok(())
}

fn destroy_window_after_close_policy(state: &Rc<RefCell<LinuxState>>) {
    let window = state.borrow().widgets.window.clone();
    window.destroy();
}

fn editor_undo_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if let Some(page) = active_page_cloned_or_refresh(state)? {
        if active_tab_is_editable(state) && page.buffer.can_undo() {
            page.view.grab_focus();
            page.buffer.undo();
            refresh_editor_status_after_command(state);
        }
    }
    Ok(())
}

fn editor_cut_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if !active_tab_is_editable(state) {
        return Ok(());
    }
    if let Some((page, clipboard)) = active_page_clipboard_or_refresh(state)? {
        page.view.grab_focus();
        page.buffer.cut_clipboard(&clipboard, true);
        refresh_editor_status_after_command(state);
    }
    Ok(())
}

fn editor_copy_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if let Some((page, clipboard)) = active_page_clipboard_or_refresh(state)? {
        page.view.grab_focus();
        page.buffer.copy_clipboard(&clipboard);
        refresh_editor_status_after_command(state);
    }
    Ok(())
}

fn editor_paste_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if !active_tab_is_editable(state) {
        return Ok(());
    }
    if let Some((page, clipboard)) = active_page_clipboard_or_refresh(state)? {
        page.view.grab_focus();
        page.buffer.paste_clipboard(&clipboard, None, true);
        refresh_editor_status_after_command(state);
    }
    Ok(())
}

fn editor_delete_selection_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if !active_tab_is_editable(state) {
        return Ok(());
    }
    if let Some(page) = active_page_cloned_or_refresh(state)? {
        page.view.grab_focus();
        page.buffer.delete_selection(true, true);
        refresh_editor_status_after_command(state);
    }
    Ok(())
}

fn editor_select_all_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if let Some(page) = active_page_cloned_or_refresh(state)? {
        page.view.grab_focus();
        drain_pending_main_context_events();
        select_entire_buffer(&page.buffer);
        drain_pending_main_context_events();
        select_entire_buffer(&page.buffer);
        refresh_editor_status_after_command(state);
    }
    Ok(())
}

fn select_entire_buffer(buffer: &gtk::TextBuffer) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.select_range(&start, &end);
}

fn drain_pending_main_context_events() {
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn refresh_editor_status_after_command(state: &Rc<RefCell<LinuxState>>) {
    update_caret_status(state);
    update_actions(state);
}

fn active_page_cloned_or_refresh(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<Option<TabPage>, AppError> {
    if let Some(page) = active_page_cloned(state) {
        return Ok(Some(page));
    }
    if state.borrow().tabs.active().is_some() {
        refresh_tabs(state)?;
    }
    Ok(active_page_cloned(state))
}

fn active_page_clipboard_or_refresh(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<Option<(TabPage, gdk::Clipboard)>, AppError> {
    let Some(page) = active_page_cloned_or_refresh(state)? else {
        return Ok(None);
    };
    let Some(display) = gdk::Display::default() else {
        return Ok(None);
    };
    Ok(Some((page, display.clipboard())))
}

fn active_tab_is_editable(state: &Rc<RefCell<LinuxState>>) -> bool {
    state.borrow().tabs.active().is_some_and(|tab| tab.editable)
}

fn find_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    open_find_replace_dialog(state, FindReplaceDialogKind::Find)
}

fn replace_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    open_find_replace_dialog(state, FindReplaceDialogKind::Replace)
}

fn new_document_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_active_tree_browse_mode(state)?;
    if !resolve_dirty_before_refresh(state)? {
        update_actions(state);
        return Ok(());
    }
    let parent_id = selected_sibling_parent_id(state)?;
    let node_id = state.borrow_mut().app.create_document(parent_id)?;
    reload_visible_document_for_label_edit(state, node_id)?;
    Ok(())
}

fn new_child_document_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_active_tree_browse_mode(state)?;
    if !resolve_dirty_before_refresh(state)? {
        update_actions(state);
        return Ok(());
    }
    let parent_id = selected_child_parent_id(state)?;
    let node_id = state.borrow_mut().app.create_document(parent_id)?;
    reload_visible_document_for_label_edit(state, node_id)?;
    Ok(())
}

fn rename_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_active_tree_browse_mode(state)?;
    let node_id = selected_node_id(state)?;
    start_label_edit(state, node_id);
    Ok(())
}

fn move_up_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    move_selected_node_within_parent(state, SiblingMoveDirection::Up)
}

fn move_down_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    move_selected_node_within_parent(state, SiblingMoveDirection::Down)
}

fn move_selected_node_within_parent(
    state: &Rc<RefCell<LinuxState>>,
    direction: SiblingMoveDirection,
) -> Result<(), AppError> {
    ensure_active_tree_browse_mode(state)?;
    if !resolve_dirty_before_refresh(state)? {
        update_actions(state);
        return Ok(());
    }
    let node_id = selected_node_id(state)?;
    state
        .borrow_mut()
        .app
        .move_node_within_parent(node_id, direction)?;
    sync_tabs_from_active_document_local_metadata(state, true)?;
    reload_visible_document(state, Some(node_id))?;
    Ok(())
}

fn delete_node_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_active_tree_browse_mode(state)?;
    let node = selected_node(state)?;
    ensure_selected_node_can_be_deleted(node.id, node.parent_id)?;

    let language = state.borrow().app.ui_settings().language;
    let parent = state.borrow().widgets.window.clone();
    if !confirm_question(
        Some(parent.upcast_ref()),
        "j3TreeText",
        &ui_text(language).confirm_delete(&node.title),
        language,
    ) {
        return Ok(());
    }

    let affected_node_ids = state
        .borrow_mut()
        .app
        .stage_active_subtree_node_ids_for_delete(node.id)?;
    let affected_set = affected_node_ids.iter().copied().collect::<HashSet<_>>();
    if !resolve_dirty_tabs_for_nodes(state, &affected_set)? {
        update_actions(state);
        return Ok(());
    }

    let removed_node_ids = state
        .borrow_mut()
        .app
        .delete_node_from_staged_active_subtree(node.id, &affected_node_ids)?;
    let removed_set = removed_node_ids.into_iter().collect::<HashSet<_>>();
    {
        let mut state = state.borrow_mut();
        state.tabs.close_tabs_for_node_set(&removed_set);
    }
    sync_tabs_from_active_document_local_metadata(state, true)?;
    reload_visible_document(state, node.parent_id)?;
    Ok(())
}

fn restore_node_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_trash_tree_mode(state)?;
    let node = selected_node(state)?;
    let language = state.borrow().app.ui_settings().language;
    let parent = state.borrow().widgets.window.clone();
    if !confirm_question(
        Some(parent.upcast_ref()),
        "j3TreeText",
        &ui_text(language).confirm_restore(&node.title),
        language,
    ) {
        return Ok(());
    }
    state.borrow_mut().app.restore_node(node.id)?;
    sync_tabs_from_active_document_local_metadata(state, true)?;
    reload_visible_document_with_expansion(state, None, TreeRefreshExpansion::ExpandAll)?;
    Ok(())
}

fn delete_permanently_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    ensure_trash_tree_mode(state)?;
    let node = selected_node(state)?;
    ensure_selected_node_can_be_deleted(node.id, node.parent_id)?;
    let language = state.borrow().app.ui_settings().language;
    let parent = state.borrow().widgets.window.clone();
    if !confirm_question(
        Some(parent.upcast_ref()),
        "j3TreeText",
        &ui_text(language).confirm_permanent_delete(&node.title),
        language,
    ) {
        return Ok(());
    }
    let affected_node_ids = subtree_node_ids(&state.borrow().document, node.id);
    let affected_set = affected_node_ids.iter().copied().collect::<HashSet<_>>();
    if !resolve_dirty_tabs_for_nodes(state, &affected_set)? {
        update_actions(state);
        return Ok(());
    }
    state.borrow_mut().app.permanently_delete_node(node.id)?;
    state
        .borrow_mut()
        .tabs
        .close_tabs_for_node_set(&affected_set);
    refresh_tabs(state)?;
    reload_visible_document_with_expansion(state, None, TreeRefreshExpansion::ExpandAll)?;
    Ok(())
}

fn tree_mode_action(state: &Rc<RefCell<LinuxState>>, mode: &str) -> Result<(), AppError> {
    let next_mode = match mode {
        "active" => TreeMode::Active,
        "trash" => TreeMode::Trash,
        _ => return Ok(()),
    };
    let already_in_target_mode = {
        let state = state.borrow();
        let needs_search_reset =
            next_mode == TreeMode::Active && !state.search_query.trim().is_empty();
        state.tree_mode == next_mode && !needs_search_reset
    };
    if already_in_target_mode {
        update_actions(state);
        return Ok(());
    }
    if !resolve_dirty_before_refresh(state)? {
        update_actions(state);
        return Ok(());
    }
    let active_tab_node_id = state.borrow().tabs.active().map(|tab| tab.node_id);
    let search = {
        let mut state = state.borrow_mut();
        state.tree_mode = next_mode;
        state.search_query.clear();
        state.suppress_search_change = true;
        state.widgets.search.clone()
    };
    search.set_text("");
    state.borrow_mut().suppress_search_change = false;
    if next_mode == TreeMode::Active {
        state.borrow_mut().app.reload_document()?;
        sync_tabs_from_reloaded_active_document_metadata(state, true, active_tab_node_id)?;
    }
    reload_visible_document_with_expansion(state, None, TreeRefreshExpansion::ExpandAll)?;
    update_window_title(state);
    Ok(())
}

fn import_encoding_action(state: &Rc<RefCell<LinuxState>>, value: &str) -> Result<(), AppError> {
    let Some(encoding) = TextEncoding::from_import_storage_value(value) else {
        return Ok(());
    };
    let mut state_mut = state.borrow_mut();
    let mut settings = state_mut.app.ui_settings();
    settings.text_encoding.import_encoding = encoding;
    state_mut.app.save_ui_settings(settings)?;
    drop(state_mut);
    update_actions(state);
    Ok(())
}

fn export_encoding_action(state: &Rc<RefCell<LinuxState>>, value: &str) -> Result<(), AppError> {
    let Some(encoding) = TextEncoding::from_export_storage_value(value) else {
        return Ok(());
    };
    let mut state_mut = state.borrow_mut();
    let mut settings = state_mut.app.ui_settings();
    settings.text_encoding.export_encoding = encoding;
    state_mut.app.save_ui_settings(settings)?;
    drop(state_mut);
    update_actions(state);
    Ok(())
}

fn theme_action(state: &Rc<RefCell<LinuxState>>, value: &str) -> Result<(), AppError> {
    let Some(theme) = AppearanceTheme::from_storage_value(value) else {
        return Ok(());
    };
    if state.borrow().app.ui_settings().appearance.theme == theme {
        update_actions(state);
        return Ok(());
    }
    let current_find_match = active_find_match_selection_byte_range(state);
    sync_active_editor_content(state)?;
    {
        let state_mut = state.borrow_mut();
        let mut settings = state_mut.app.ui_settings();
        settings.appearance.set_theme(theme);
        drop(state_mut);
        save_ui_settings_with_resolved_editor_font(state, settings)?;
    }
    apply_theme(state)?;
    restore_active_find_match_highlight(state, current_find_match);
    update_actions(state);
    Ok(())
}

fn language_action(state: &Rc<RefCell<LinuxState>>, value: &str) -> Result<(), AppError> {
    let Some(language) = UiLanguage::from_storage_value(value) else {
        return Ok(());
    };
    if state.borrow().app.ui_settings().language == language {
        update_actions(state);
        return Ok(());
    }
    sync_active_editor_content(state)?;
    let preferred_node_id = {
        let mut state_mut = state.borrow_mut();
        let preferred_node_id = state_mut.selected_node_id;
        let mut settings = state_mut.app.ui_settings();
        settings.language = language;
        state_mut.app.save_ui_settings(settings)?;
        state_mut
            .widgets
            .search
            .set_placeholder_text(Some(ui_text(language).search_cue()));
        preferred_node_id
    };
    rebuild_menu_bar(state);
    reload_visible_tree_preserving_tabs_with_expansion(
        state,
        preferred_node_id,
        TreeRefreshExpansion::ExpandAll,
    )?;
    update_caret_status(state);
    update_window_title(state);
    Ok(())
}

fn toggle_word_wrap_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    sync_active_editor_content(state)?;
    let previous_word_wrap = state.borrow().app.ui_settings().editor.word_wrap;
    let next_word_wrap = toggle_editor_word_wrap(previous_word_wrap);
    apply_editor_word_wrap(state, next_word_wrap);
    reset_active_editor_undo_stack(state);
    {
        let mut state_mut = state.borrow_mut();
        let mut settings = state_mut.app.ui_settings();
        settings.editor.word_wrap = next_word_wrap;
        if let Err(error) = state_mut.app.save_ui_settings(settings) {
            drop(state_mut);
            apply_editor_word_wrap(state, previous_word_wrap);
            update_actions(state);
            return Err(error);
        }
    }
    update_actions(state);
    Ok(())
}

fn editor_font_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let (window, current, language) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().editor_font,
            state.app.ui_settings().language,
        )
    };
    let Some(selected) = choose_editor_font(window.upcast_ref(), language, &current) else {
        return Ok(());
    };
    sync_active_editor_content(state)?;
    let used_fallback = {
        let state_mut = state.borrow_mut();
        let mut settings = state_mut.app.ui_settings();
        settings.editor_font = selected;
        drop(state_mut);
        save_ui_settings_with_resolved_editor_font(state, settings)?
    };
    apply_theme(state)?;
    reset_active_editor_undo_stack(state);
    update_actions(state);
    if used_fallback {
        show_gtk_error_message(
            Some(window.upcast_ref()),
            "j3TreeText",
            ui_text(language).font_fallback(),
        );
    }
    Ok(())
}

fn about_action(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let (window, language) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
        )
    };
    let text = ui_text(language);
    show_about_dialog(
        Some(window.upcast_ref()),
        text,
        text.about_title(),
        &text.about_message(env!("CARGO_PKG_VERSION")),
    );
    Ok(())
}

fn handle_tree_key_pressed(
    state: &Rc<RefCell<LinuxState>>,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> bool {
    if state.borrow().editing_node_id.is_some() {
        return false;
    }

    if is_context_menu_key(key, modifiers) {
        show_selected_tree_context_menu(state);
        return true;
    }

    let tree_mode = state.borrow().tree_mode;
    let Some(command) = tree_key_command(key, modifiers, tree_mode) else {
        return false;
    };

    let restore_tree_focus = command.restores_tree_focus_after_action();
    run_and_report(state, run_tree_key_command(state, command));
    if restore_tree_focus {
        state.borrow().widgets.tree.grab_focus();
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeKeyCommand {
    NewDocument,
    NewChildDocument,
    Rename,
    MoveToTrash,
    DeletePermanently,
    MoveUp,
    MoveDown,
    ExpandOrSelectChild,
    CollapseOrSelectParent,
    ExpandNode,
    CollapseNode,
    ExpandSubtree,
}

impl TreeKeyCommand {
    fn restores_tree_focus_after_action(self) -> bool {
        matches!(self, Self::MoveToTrash | Self::DeletePermanently)
    }
}

fn tree_key_command(
    key: gdk::Key,
    modifiers: gdk::ModifierType,
    tree_mode: TreeMode,
) -> Option<TreeKeyCommand> {
    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
    match (key, ctrl) {
        (gdk::Key::Return, false) if !alt => Some(TreeKeyCommand::NewDocument),
        (gdk::Key::Return, true) if !alt => Some(TreeKeyCommand::NewChildDocument),
        (gdk::Key::Right | gdk::Key::KP_Right, false) if !alt => {
            Some(TreeKeyCommand::ExpandOrSelectChild)
        }
        (gdk::Key::Left | gdk::Key::KP_Left, false) if !alt => {
            Some(TreeKeyCommand::CollapseOrSelectParent)
        }
        (gdk::Key::plus | gdk::Key::KP_Add, false) if !alt => Some(TreeKeyCommand::ExpandNode),
        (gdk::Key::minus | gdk::Key::KP_Subtract, false) if !alt => {
            Some(TreeKeyCommand::CollapseNode)
        }
        (gdk::Key::asterisk | gdk::Key::KP_Multiply, false) if !alt => {
            Some(TreeKeyCommand::ExpandSubtree)
        }
        (gdk::Key::F2, _) => Some(TreeKeyCommand::Rename),
        (gdk::Key::Delete, _) => match tree_mode {
            TreeMode::Active => Some(TreeKeyCommand::MoveToTrash),
            TreeMode::Trash => Some(TreeKeyCommand::DeletePermanently),
        },
        (gdk::Key::Up, true) => Some(TreeKeyCommand::MoveUp),
        (gdk::Key::Down, true) => Some(TreeKeyCommand::MoveDown),
        _ => None,
    }
}

fn run_tree_key_command(
    state: &Rc<RefCell<LinuxState>>,
    command: TreeKeyCommand,
) -> Result<(), AppError> {
    match command {
        TreeKeyCommand::NewDocument => new_document_action(state),
        TreeKeyCommand::NewChildDocument => new_child_document_action(state),
        TreeKeyCommand::Rename => rename_action(state),
        TreeKeyCommand::MoveToTrash => delete_node_action(state),
        TreeKeyCommand::DeletePermanently => delete_permanently_action(state),
        TreeKeyCommand::MoveUp => move_up_action(state),
        TreeKeyCommand::MoveDown => move_down_action(state),
        TreeKeyCommand::ExpandOrSelectChild => {
            run_tree_navigation_command(state, tree_right_key_action_for_state(state)?)
        }
        TreeKeyCommand::CollapseOrSelectParent => {
            run_tree_navigation_command(state, tree_left_key_action_for_state(state)?)
        }
        TreeKeyCommand::ExpandNode => {
            run_tree_navigation_command(state, tree_expand_key_action_for_state(state)?)
        }
        TreeKeyCommand::CollapseNode => {
            run_tree_navigation_command(state, tree_collapse_key_action_for_state(state)?)
        }
        TreeKeyCommand::ExpandSubtree => {
            run_tree_navigation_command(state, tree_expand_subtree_key_action_for_state(state)?)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeNavigationAction {
    None,
    Select(i64),
    Expand(i64),
    Collapse(i64),
    ExpandSubtree(i64),
}

fn tree_right_key_action_for_state(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<TreeNavigationAction, AppError> {
    let state = state.borrow();
    let selected_node_id = selected_existing_node_id(&state.document, state.selected_node_id)?;
    Ok(tree_right_key_action(
        &state.document,
        &state.visible_node_ids,
        selected_node_id,
    ))
}

fn tree_left_key_action_for_state(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<TreeNavigationAction, AppError> {
    let state = state.borrow();
    let selected_node_id = selected_existing_node_id(&state.document, state.selected_node_id)?;
    Ok(tree_left_key_action(
        &state.document,
        &state.visible_node_ids,
        selected_node_id,
    ))
}

fn tree_expand_key_action_for_state(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<TreeNavigationAction, AppError> {
    let state = state.borrow();
    let selected_node_id = selected_existing_node_id(&state.document, state.selected_node_id)?;
    Ok(tree_expand_key_action(&state.document, selected_node_id))
}

fn tree_collapse_key_action_for_state(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<TreeNavigationAction, AppError> {
    let state = state.borrow();
    let selected_node_id = selected_existing_node_id(&state.document, state.selected_node_id)?;
    Ok(tree_collapse_key_action(&state.document, selected_node_id))
}

fn tree_expand_subtree_key_action_for_state(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<TreeNavigationAction, AppError> {
    let state = state.borrow();
    let selected_node_id = selected_existing_node_id(&state.document, state.selected_node_id)?;
    Ok(tree_expand_subtree_key_action(
        &state.document,
        selected_node_id,
    ))
}

fn run_tree_navigation_command(
    state: &Rc<RefCell<LinuxState>>,
    action: TreeNavigationAction,
) -> Result<(), AppError> {
    match action {
        TreeNavigationAction::None => Ok(()),
        TreeNavigationAction::Select(node_id) => {
            select_tree_node_with_navigation(state, node_id, false)?;
            Ok(())
        }
        TreeNavigationAction::Expand(node_id) => {
            set_tree_node_expanded(state, node_id, true, Some(node_id))
        }
        TreeNavigationAction::Collapse(node_id) => {
            set_tree_node_expanded(state, node_id, false, Some(node_id))
        }
        TreeNavigationAction::ExpandSubtree(node_id) => {
            let expanded_node_ids = {
                let state = state.borrow();
                state.document.expandable_subtree_node_ids(node_id)
            };
            if expanded_node_ids.is_empty() {
                return Ok(());
            }
            state
                .borrow_mut()
                .expanded_node_ids
                .extend(expanded_node_ids);
            refresh_tree_node_descendants(state, node_id, Some(node_id))
        }
    }
}

fn tree_right_key_action(
    document: &UiDocument,
    visible_node_ids: &[i64],
    selected_node_id: i64,
) -> TreeNavigationAction {
    if !document.has_display_children(selected_node_id) {
        return TreeNavigationAction::None;
    }
    match first_visible_display_child_id(document, visible_node_ids, selected_node_id) {
        Some(child_node_id) => TreeNavigationAction::Select(child_node_id),
        None => TreeNavigationAction::Expand(selected_node_id),
    }
}

fn tree_left_key_action(
    document: &UiDocument,
    visible_node_ids: &[i64],
    selected_node_id: i64,
) -> TreeNavigationAction {
    if first_visible_display_child_id(document, visible_node_ids, selected_node_id).is_some() {
        return TreeNavigationAction::Collapse(selected_node_id);
    }
    document
        .node_by_id(selected_node_id)
        .and_then(|node| node.display_parent_id)
        .map(TreeNavigationAction::Select)
        .unwrap_or(TreeNavigationAction::None)
}

fn tree_expand_key_action(document: &UiDocument, selected_node_id: i64) -> TreeNavigationAction {
    if document.has_display_children(selected_node_id) {
        TreeNavigationAction::Expand(selected_node_id)
    } else {
        TreeNavigationAction::None
    }
}

fn tree_collapse_key_action(document: &UiDocument, selected_node_id: i64) -> TreeNavigationAction {
    if document.has_display_children(selected_node_id) {
        TreeNavigationAction::Collapse(selected_node_id)
    } else {
        TreeNavigationAction::None
    }
}

fn tree_expand_subtree_key_action(
    document: &UiDocument,
    selected_node_id: i64,
) -> TreeNavigationAction {
    if document.has_display_children(selected_node_id) {
        TreeNavigationAction::ExpandSubtree(selected_node_id)
    } else {
        TreeNavigationAction::None
    }
}

fn first_visible_display_child_id(
    document: &UiDocument,
    visible_node_ids: &[i64],
    parent_node_id: i64,
) -> Option<i64> {
    let parent_position = visible_node_ids
        .iter()
        .position(|node_id| *node_id == parent_node_id)?;
    let child_node_id = *visible_node_ids.get(parent_position + 1)?;
    document
        .node_by_id(child_node_id)
        .is_some_and(|node| node.display_parent_id == Some(parent_node_id))
        .then_some(child_node_id)
}

fn is_context_menu_key(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    key == gdk::Key::Menu
        || (key == gdk::Key::F10
            && modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            && !modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK))
}

fn handle_tree_row_selected(
    state: &Rc<RefCell<LinuxState>>,
    row_index: i32,
) -> Result<(), AppError> {
    if state.borrow().suppress_tree_selection {
        return Ok(());
    }
    let Some(node_id) = usize::try_from(row_index)
        .ok()
        .and_then(|index| state.borrow().visible_node_ids.get(index).copied())
    else {
        return Ok(());
    };

    select_tree_node_with_navigation(state, node_id, true)?;
    Ok(())
}

fn select_tree_node_with_navigation(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
    force: bool,
) -> Result<bool, AppError> {
    let previous_node_id = state.borrow().selected_node_id;
    if !force && previous_node_id == Some(node_id) {
        return Ok(true);
    }

    if !autosave_active_tab_before_navigation(state)? {
        restore_tree_selection(state, previous_node_id);
        return Ok(false);
    }

    {
        let mut state = state.borrow_mut();
        state.selected_node_id = Some(node_id);
    }
    open_or_activate_tab_from_node(state, node_id)?;
    schedule_selection_ui_setting_save(state, node_id);
    select_visible_tree_row_suppressed(state, node_id);
    update_actions(state);
    Ok(true)
}

fn restore_tree_selection(state: &Rc<RefCell<LinuxState>>, node_id: Option<i64>) {
    let (tree, row) = {
        let state = state.borrow();
        (
            state.widgets.tree.clone(),
            node_id
                .and_then(|node_id| state.visible_node_ids.iter().position(|id| *id == node_id))
                .and_then(|index| state.widgets.tree.row_at_index(index as i32)),
        )
    };
    state.borrow_mut().suppress_tree_selection = true;
    tree.select_row(row.as_ref());
    state.borrow_mut().suppress_tree_selection = false;
}

fn select_visible_tree_row_suppressed(state: &Rc<RefCell<LinuxState>>, node_id: i64) {
    let (tree, row) = {
        let state = state.borrow();
        (
            state.widgets.tree.clone(),
            state
                .visible_node_ids
                .iter()
                .position(|id| *id == node_id)
                .and_then(|index| state.widgets.tree.row_at_index(index as i32)),
        )
    };
    state.borrow_mut().suppress_tree_selection = true;
    tree.select_row(row.as_ref());
    state.borrow_mut().suppress_tree_selection = false;
}

fn open_or_activate_tab_from_node(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<(), AppError> {
    if activate_existing_tab_from_node_without_content_load(state, node_id)? {
        refresh_tabs(state)?;
        reset_active_editor_undo_stack(state);
        update_actions(state);
        update_window_title(state);
        return Ok(());
    }

    let input = tab_input_from_node(state, node_id)?;
    {
        let mut state = state.borrow_mut();
        state.tabs.open_or_activate(input);
    }
    refresh_tabs(state)?;
    reset_active_editor_undo_stack(state);
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn activate_existing_tab_from_node_without_content_load(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<bool, AppError> {
    let node = state
        .borrow()
        .document
        .node_by_id(node_id)
        .cloned()
        .ok_or(DomainError::NodeNotFound { node_id })?;

    let mut state = state.borrow_mut();
    let Some(index) = existing_tab_index_without_content_load(&state.tabs, &node) else {
        return Ok(false);
    };

    state.tabs.set_active(index);
    state.tabs.sync_loaded_tab_metadata_preserving_content_at(
        index,
        LoadedTabMetadataUpdate {
            node_id: node.id,
            parent_id: node.parent_id,
            title: node.title,
            loaded_updated_at: node.updated_at,
            editable: node.editable,
            source: node.source,
            current_content_for_dirty_token: None,
        },
    );
    Ok(true)
}

fn existing_tab_index_without_content_load(tabs: &OpenTabs, node: &UiNode) -> Option<usize> {
    tabs.tabs()
        .iter()
        .position(|tab| tab.node_id == node.id)
        .filter(|index| {
            let tab = &tabs.tabs()[*index];
            tab.dirty || tab.loaded_updated_at == node.updated_at
        })
}

fn tab_input_from_node(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<OpenDocumentTabInput, AppError> {
    let node = state
        .borrow()
        .document
        .node_by_id(node_id)
        .cloned()
        .ok_or(DomainError::NodeNotFound { node_id })?;
    let (content, updated_at) = match node.source {
        DocumentTabSource::Trash => state.borrow().app.load_deleted_node_content(node.id)?,
        DocumentTabSource::ActiveTree | DocumentTabSource::SearchResult => state
            .borrow()
            .app
            .load_active_node_content_if_present(node.id)?
            .ok_or(DomainError::NodeNotFound { node_id: node.id })?,
    };
    Ok(OpenDocumentTabInput {
        node_id: node.id,
        parent_id: node.parent_id,
        title: node.title,
        content,
        loaded_updated_at: updated_at,
        editable: node.editable,
        source: node.source,
    })
}

fn handle_tab_switched(
    state: &Rc<RefCell<LinuxState>>,
    _page: &gtk::Widget,
    page_num: u32,
) -> Result<(), AppError> {
    if state.borrow().suppress_tab_change {
        return Ok(());
    }
    let previous_index = state.borrow().tabs.active_index();
    let next_index = usize::try_from(page_num)
        .map_err(|_| AppError::platform("switch tab", "tab index is too large"))?;
    if previous_index == Some(next_index) {
        update_caret_status(state);
        update_actions(state);
        return Ok(());
    }

    if !autosave_active_tab_before_navigation(state)? {
        if let Some(previous_index) = previous_index {
            set_notebook_current_page_suppressed(state, previous_index);
        }
        return Ok(());
    }

    {
        let mut state = state.borrow_mut();
        state.tabs.set_active(next_index);
    }
    reset_active_editor_undo_stack(state);
    select_active_tab_node_in_tree(state);
    update_caret_status(state);
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn handle_tab_reordered(
    state: &Rc<RefCell<LinuxState>>,
    page: &gtk::Widget,
    page_num: u32,
) -> Result<(), AppError> {
    if state.borrow().suppress_tab_change {
        return Ok(());
    }
    sync_active_editor_content(state)?;
    let to_index = usize::try_from(page_num)
        .map_err(|_| AppError::platform("move tab", "tab index is too large"))?;
    let from_index = {
        let state = state.borrow();
        state.tab_pages.iter().position(|tab_page| {
            tab_page.page.upcast_ref::<gtk::Widget>().as_ptr() == page.as_ptr()
        })
    };
    let Some(from_index) = from_index else {
        return Ok(());
    };
    if from_index == to_index {
        return Ok(());
    }
    {
        let mut state = state.borrow_mut();
        state.tabs.move_tab(from_index, to_index);
        let moved = state.tab_pages.remove(from_index);
        state.tab_pages.insert(to_index, moved);
    }
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn set_notebook_current_page_suppressed(state: &Rc<RefCell<LinuxState>>, index: usize) {
    let notebook = state.borrow().widgets.notebook.clone();
    state.borrow_mut().suppress_tab_change = true;
    notebook.set_current_page(Some(index as u32));
    state.borrow_mut().suppress_tab_change = false;
}

fn select_active_tab_node_in_tree(state: &Rc<RefCell<LinuxState>>) {
    let node_id = state.borrow().tabs.active().map(|tab| tab.node_id);
    let (tree, row) = {
        let state = state.borrow();
        (
            state.widgets.tree.clone(),
            node_id
                .and_then(|node_id| state.visible_node_ids.iter().position(|id| *id == node_id))
                .and_then(|index| state.widgets.tree.row_at_index(index as i32)),
        )
    };
    if let (Some(node_id), None) = (node_id, row.as_ref()) {
        run_and_report(state, rebuild_tree_list(state, Some(node_id)));
        return;
    }
    state.borrow_mut().suppress_tree_selection = true;
    tree.select_row(row.as_ref());
    {
        let mut state = state.borrow_mut();
        state.suppress_tree_selection = false;
        state.selected_node_id = node_id;
    }
}

fn handle_search_changed(state: &Rc<RefCell<LinuxState>>, next_query: String) {
    let previous_query = {
        let mut state_mut = state.borrow_mut();
        if next_query == state_mut.search_query {
            state_mut.search_generation = state_mut.search_generation.saturating_add(1);
            return;
        }

        let previous_query = state_mut.search_query.clone();
        if next_query.trim() == previous_query.trim() {
            state_mut.search_generation = state_mut.search_generation.saturating_add(1);
            state_mut.search_query = next_query;
            return;
        }

        if should_debounce_search_change(&state_mut, &previous_query, &next_query) {
            previous_query
        } else {
            state_mut.search_generation = state_mut.search_generation.saturating_add(1);
            drop(state_mut);
            let result = apply_search_text_change_now(state, next_query, previous_query);
            run_and_report(state, result);
            return;
        }
    };

    let generation = {
        let mut state = state.borrow_mut();
        state.search_generation = state.search_generation.saturating_add(1);
        state.search_generation
    };
    let state_clone = Rc::clone(state);
    glib::timeout_add_local_once(Duration::from_millis(SEARCH_DEBOUNCE_MS), move || {
        let is_current = state_clone.borrow().search_generation == generation;
        if !is_current {
            return;
        }
        let result = apply_search_text_change_now(&state_clone, next_query, previous_query);
        run_and_report(&state_clone, result);
    });
}

fn should_debounce_search_change(
    state: &LinuxState,
    previous_query: &str,
    next_query: &str,
) -> bool {
    state.tree_mode == TreeMode::Active
        && !next_query.trim().is_empty()
        && !can_refine_search_document_from_visible_results(
            &state.document,
            state.tree_mode,
            previous_query,
            next_query,
        )
}

fn apply_search_text_change_now(
    state: &Rc<RefCell<LinuxState>>,
    next_query: String,
    previous_query: String,
) -> Result<(), AppError> {
    if next_query == state.borrow().search_query {
        return Ok(());
    }
    if next_query.trim() == state.borrow().search_query.trim() {
        state.borrow_mut().search_query = next_query;
        return Ok(());
    }

    sync_active_editor_content(state)?;
    let preferred_node_id = {
        let state = state.borrow();
        state
            .selected_node_id
            .or(state.app.ui_settings_ref().selection.node_id)
    };
    {
        let mut state = state.borrow_mut();
        state.tree_mode = TreeMode::Active;
        state.search_query = next_query;
    }
    if !search_is_active(state) {
        state.borrow_mut().app.reload_document()?;
        let active_marked_read_only = sync_tabs_from_active_document_local_metadata(state, true)?;
        reload_visible_tree_preserving_tabs_with_expansion(
            state,
            preferred_node_id,
            TreeRefreshExpansion::ExpandAll,
        )?;
        if active_marked_read_only {
            refresh_tabs(state)?;
        }
        update_window_title(state);
        return Ok(());
    }
    reload_visible_document_after_search_change(state, &previous_query, preferred_node_id)?;
    update_window_title(state);
    Ok(())
}

fn reload_visible_document_after_search_change(
    state: &Rc<RefCell<LinuxState>>,
    previous_query: &str,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let document = {
        let state = state.borrow();
        refined_search_document_from_visible_results(&state, previous_query)
    };
    let document = match document {
        Some(document) => document,
        None => {
            let state = state.borrow();
            load_ui_document(&state)?
        }
    };
    state.borrow_mut().document = document;
    apply_tree_refresh_expansion(state, TreeRefreshExpansion::ExpandAll);
    sync_tabs_from_visible_document_preserving_content(state);
    rebuild_tree_list(state, preferred_node_id)?;
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn search_is_active(state: &Rc<RefCell<LinuxState>>) -> bool {
    let state = state.borrow();
    state.tree_mode == TreeMode::Active && !state.search_query.trim().is_empty()
}

fn refined_search_document_from_visible_results(
    state: &LinuxState,
    previous_query: &str,
) -> Option<UiDocument> {
    refined_search_document_from_visible_document(
        &state.document,
        state.tree_mode,
        previous_query,
        &state.search_query,
    )
}

fn refined_search_document_from_visible_document(
    document: &UiDocument,
    tree_mode: TreeMode,
    previous_query: &str,
    next_query: &str,
) -> Option<UiDocument> {
    let next_query = next_query.trim();
    if !can_refine_search_document_from_visible_results(
        document,
        tree_mode,
        previous_query,
        next_query,
    ) {
        return None;
    }

    let nodes = document
        .nodes
        .iter()
        .filter(|node| search_result_node_matches(node, next_query))
        .cloned()
        .collect();

    Some(UiDocument::from_ui_nodes(nodes))
}

fn can_refine_search_document_from_visible_results(
    document: &UiDocument,
    tree_mode: TreeMode,
    previous_query: &str,
    next_query: &str,
) -> bool {
    let previous_query = previous_query.trim();
    let next_query = next_query.trim();
    tree_mode == TreeMode::Active
        && !previous_query.is_empty()
        && !next_query.is_empty()
        && next_query.starts_with(previous_query)
        && search_refinement_preserves_content_scope(previous_query, next_query)
        && document.nodes.len() < SEARCH_RESULT_LIMIT
        && document
            .nodes
            .iter()
            .all(|node| search_result_node_is_refinable(node, previous_query))
}

fn search_refinement_preserves_content_scope(previous_query: &str, next_query: &str) -> bool {
    fts_trigram_content_search_is_enabled(previous_query)
        || !fts_trigram_content_search_is_enabled(next_query)
}

fn fts_trigram_content_search_is_enabled(query: &str) -> bool {
    query.chars().count() >= 3 && !query.chars().any(|character| character == '\0')
}

fn search_result_node_matches(node: &UiNode, query: &str) -> bool {
    contains_sqlite_like_literal(&node.title, query)
}

fn search_result_node_is_refinable(node: &UiNode, previous_query: &str) -> bool {
    matches!(node.source, DocumentTabSource::SearchResult)
        && !node.search_content_matched
        && contains_sqlite_like_literal(&node.title, previous_query)
}

fn contains_sqlite_like_literal(text: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    let query = query.as_bytes();
    text.as_bytes()
        .windows(query.len())
        .any(|window| window.eq_ignore_ascii_case(query))
}

fn sync_active_editor_content(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let Some(page) = active_page_cloned(state) else {
        state.borrow_mut().editor_content_pending_sync = false;
        return Ok(());
    };
    if state.borrow().suppress_editor_change {
        return Ok(());
    }

    let pending_editor_content_sync = state.borrow().editor_content_pending_sync;
    if pending_editor_content_sync {
        clear_find_match_highlight(&page.buffer);
        let mut content = buffer_text(&page.buffer);
        let view_state =
            gtk_tab_view_state_checked(&page.view, &page.buffer, &content).map_err(|_| {
                let language = state.borrow().app.ui_settings().language;
                AppError::user(
                    ui_text(language).editor_text_too_large(DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB),
                )
            })?;
        let active_content_marker = {
            let mut state = state.borrow_mut();
            state.tabs.update_active_content_reusing(&mut content);
            state.tabs.update_active_view_state(view_state);
            state.editor_content_pending_sync = false;
            state
                .tabs
                .active()
                .map(|tab| (tab.node_id, tab.source, tab.content_revision()))
        };
        if let Some((node_id, source, content_revision)) = active_content_marker {
            page.node_id.set(node_id);
            page.source.set(source);
            page.content_revision.set(content_revision);
        }
    } else {
        let view_state = {
            let state = state.borrow();
            let Some(tab) = state.tabs.active() else {
                return Ok(());
            };
            gtk_tab_view_state(&page.view, &page.buffer, &tab.content)
        };
        state.borrow_mut().tabs.update_active_view_state(view_state);
    }
    update_actions(state);
    Ok(())
}

fn handle_editor_changed_for_page(
    state: &Rc<RefCell<LinuxState>>,
    page: &gtk::ScrolledWindow,
) -> Result<(), AppError> {
    if state.borrow().suppress_editor_change {
        return Ok(());
    }
    let active_index = state.borrow().tabs.active_index();
    let page_index = {
        let state = state.borrow();
        state
            .tab_pages
            .iter()
            .position(|tab_page| tab_page.page == *page)
    };
    if active_index != page_index {
        return Ok(());
    }

    {
        let mut state = state.borrow_mut();
        if state.tabs.active().is_some_and(|tab| tab.editable) {
            state.editor_content_pending_sync = true;
            state.tabs.mark_active_dirty_from_view();
        }
    }
    update_actions(state);
    Ok(())
}

fn save_current_document(state: &Rc<RefCell<LinuxState>>) -> Result<SaveOutcome, AppError> {
    sync_active_editor_content(state)?;
    let (context, save_result) = {
        let mut state = state.borrow_mut();
        let LinuxState { app, tabs, .. } = &mut *state;
        let Some(tab) = tabs.active() else {
            return Ok(SaveOutcome::NoChanges);
        };
        let context = SaveContext {
            node_id: tab.node_id,
            parent_id: tab.parent_id,
            title: tab.title.clone(),
            expected_updated_at: tab.loaded_updated_at.clone(),
            save_target: tab.is_save_target(),
        };
        if !context.save_target {
            return Ok(SaveOutcome::NoChanges);
        }
        let save_result =
            app.save_document_content(context.node_id, &tab.content, &context.expected_updated_at);
        (context, save_result)
    };
    let updated_at = match save_result {
        Ok(updated_at) => updated_at,
        Err(AppError::Domain(DomainError::DocumentSaveConflict { .. })) => {
            let content = {
                let state = state.borrow();
                let Some(tab) = state.tabs.active() else {
                    return Err(AppError::platform(
                        "save document",
                        "active tab content was not available",
                    ));
                };
                tab.content.clone()
            };
            return handle_save_conflict(state, context, content);
        }
        Err(error) => return Err(error),
    };

    {
        let mut state = state.borrow_mut();
        if let Some(node) = state
            .document
            .nodes
            .iter_mut()
            .find(|node| node.id == context.node_id)
        {
            node.updated_at = updated_at.clone();
        }
        state.tabs.mark_active_current_content_saved(updated_at);
    }
    refresh_tab_labels(state);
    update_window_title(state);
    Ok(SaveOutcome::Saved)
}

#[derive(Clone)]
struct SaveContext {
    node_id: i64,
    parent_id: Option<i64>,
    title: String,
    expected_updated_at: String,
    save_target: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveOutcome {
    NoChanges,
    Saved,
    Reloaded,
    SavedAsNewDocument,
    Canceled,
}

fn save_outcome_allows_navigation(outcome: SaveOutcome) -> bool {
    matches!(
        outcome,
        SaveOutcome::NoChanges
            | SaveOutcome::Saved
            | SaveOutcome::Reloaded
            | SaveOutcome::SavedAsNewDocument
    )
}

fn handle_save_conflict(
    state: &Rc<RefCell<LinuxState>>,
    context: SaveContext,
    content: String,
) -> Result<SaveOutcome, AppError> {
    let (window, language) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
        )
    };
    match prompt_save_conflict(window.upcast_ref(), language) {
        ConflictDecision::Reload => {
            state.borrow_mut().app.reload_document()?;
            if let Some(input) = tab_input_from_active_document(state, context.node_id)? {
                state.borrow_mut().tabs.reload_active(input);
            }
            sync_tabs_from_reloaded_active_document_metadata(state, false, Some(context.node_id))?;
            reload_visible_document_with_expansion(
                state,
                Some(context.node_id),
                TreeRefreshExpansion::ExpandAll,
            )?;
            Ok(SaveOutcome::Reloaded)
        }
        ConflictDecision::SaveAsNewDocument => {
            let parent_id = context.parent_id.unwrap_or(ROOT_NODE_ID);
            let base_title = ui_text(language).conflicted_copy_title(&context.title);
            let node_id = state
                .borrow_mut()
                .app
                .save_document_content_as_new_document(parent_id, &base_title, &content)?;
            if let Some(input) = tab_input_from_active_document(state, node_id)? {
                state.borrow_mut().tabs.replace_active(input);
            }
            reload_active_document_and_tree_with_expansion(
                state,
                Some(node_id),
                TreeRefreshExpansion::ExpandAll,
            )?;
            refresh_tabs(state)?;
            Ok(SaveOutcome::SavedAsNewDocument)
        }
        ConflictDecision::Cancel => Ok(SaveOutcome::Canceled),
    }
}

fn tab_input_from_active_document(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<Option<OpenDocumentTabInput>, AppError> {
    let metadata = state
        .borrow()
        .app
        .document()
        .node_by_id(node_id)
        .map(|node| {
            (
                node.id,
                node.parent_id,
                node.title.clone(),
                node.deleted_at.is_none(),
            )
        });
    let Some((node_id, parent_id, title, editable)) = metadata else {
        return Ok(None);
    };
    let (content, updated_at) = state.borrow().app.load_active_node_content(node_id)?;
    Ok(Some(OpenDocumentTabInput {
        node_id,
        parent_id,
        title,
        content,
        loaded_updated_at: updated_at,
        editable,
        source: DocumentTabSource::ActiveTree,
    }))
}

fn autosave_active_tab_before_navigation(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<bool, AppError> {
    sync_active_editor_content(state)?;
    let Some((dirty, is_save_target)) = ({
        let state = state.borrow();
        state
            .tabs
            .active()
            .map(|tab| (tab.dirty, tab.is_save_target()))
    }) else {
        return Ok(true);
    };
    if !dirty {
        return Ok(true);
    }
    if !is_save_target {
        return resolve_unsavable_dirty_active_tab(state);
    }
    Ok(save_outcome_allows_navigation(save_current_document(
        state,
    )?))
}

fn autosave_all_dirty_tabs(state: &Rc<RefCell<LinuxState>>) -> Result<bool, AppError> {
    sync_active_editor_content(state)?;
    loop {
        let Some(index) = ({ state.borrow().tabs.first_dirty_index() }) else {
            break;
        };
        state.borrow_mut().tabs.set_active(index);
        refresh_tabs(state)?;
        update_window_title(state);
        let outcome = autosave_active_tab_from_memory_before_close(state)?;
        if !save_outcome_allows_navigation(outcome) {
            refresh_tabs(state)?;
            reset_active_editor_undo_stack(state);
            update_window_title(state);
            return Ok(false);
        }
    }
    Ok(true)
}

fn autosave_active_tab_from_memory_before_close(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<SaveOutcome, AppError> {
    let Some((dirty, is_save_target)) = ({
        let state = state.borrow();
        state
            .tabs
            .active()
            .map(|tab| (tab.dirty, tab.is_save_target()))
    }) else {
        return Ok(SaveOutcome::NoChanges);
    };
    if !dirty {
        return Ok(SaveOutcome::NoChanges);
    }
    if !is_save_target {
        return if resolve_unsavable_dirty_active_tab(state)? {
            Ok(SaveOutcome::NoChanges)
        } else {
            Ok(SaveOutcome::Canceled)
        };
    }
    save_current_document(state)
}

fn resolve_unsavable_dirty_active_tab(state: &Rc<RefCell<LinuxState>>) -> Result<bool, AppError> {
    let (window, language, tab_title, active_index) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
            state.tabs.active().map(|tab| tab.title.clone()),
            state.tabs.active_index(),
        )
    };
    if !prompt_discard_unsavable_changes(window.upcast_ref(), language, tab_title.as_deref()) {
        return Ok(false);
    }
    if let Some(index) = active_index {
        state.borrow_mut().tabs.discard_tab_changes(index);
        refresh_tabs(state)?;
        update_window_title(state);
    }
    Ok(true)
}

fn resolve_active_dirty_before_import(state: &Rc<RefCell<LinuxState>>) -> Result<bool, AppError> {
    sync_active_editor_content(state)?;
    let Some((tab_title, window, language)) = ({
        let state = state.borrow();
        let Some(tab) = state.tabs.active() else {
            return Ok(false);
        };
        if !tab.dirty {
            return Ok(true);
        }
        Some((
            tab.title.clone(),
            state.widgets.window.clone(),
            state.app.ui_settings().language,
        ))
    }) else {
        return Ok(false);
    };
    match prompt_unsaved_changes(window.upcast_ref(), Some(&tab_title), language) {
        DirtyTabDecision::Save => Ok(save_outcome_allows_navigation(save_current_document(
            state,
        )?)),
        DirtyTabDecision::Discard => {
            if let Some(index) = state.borrow().tabs.active_index() {
                state.borrow_mut().tabs.discard_tab_changes(index);
            }
            Ok(true)
        }
        DirtyTabDecision::Cancel => Ok(false),
    }
}

fn resolve_dirty_tabs_for_nodes(
    state: &Rc<RefCell<LinuxState>>,
    node_ids: &HashSet<i64>,
) -> Result<bool, AppError> {
    sync_active_editor_content(state)?;
    loop {
        let dirty_indices = state.borrow().tabs.dirty_tab_indices_for_node_set(node_ids);
        let Some(index) = dirty_indices.first().copied() else {
            return Ok(true);
        };
        {
            state.borrow_mut().tabs.set_active(index);
        }
        refresh_tabs(state)?;
        reset_active_editor_undo_stack(state);
        update_window_title(state);
        let (has_unsavable_changes, tab_title, language, window) = {
            let state = state.borrow();
            let Some(tab) = state.tabs.active() else {
                return Ok(false);
            };
            (
                tab.has_unsavable_changes(),
                tab.title.clone(),
                state.app.ui_settings().language,
                state.widgets.window.clone(),
            )
        };
        if has_unsavable_changes {
            if !resolve_unsavable_dirty_active_tab(state)? {
                return Ok(false);
            }
            continue;
        }
        match prompt_unsaved_changes(window.upcast_ref(), Some(&tab_title), language) {
            DirtyTabDecision::Save => match save_current_document(state)? {
                SaveOutcome::Saved | SaveOutcome::NoChanges => {}
                SaveOutcome::Reloaded | SaveOutcome::SavedAsNewDocument | SaveOutcome::Canceled => {
                    return Ok(false)
                }
            },
            DirtyTabDecision::Discard => {
                state.borrow_mut().tabs.discard_tab_changes(index);
                refresh_tabs(state)?;
            }
            DirtyTabDecision::Cancel => return Ok(false),
        }
    }
}

fn resolve_dirty_before_refresh(state: &Rc<RefCell<LinuxState>>) -> Result<bool, AppError> {
    sync_active_editor_content(state)?;
    sync_tabs_from_visible_document(state, false)?;
    refresh_tabs(state)?;
    update_window_title(state);
    Ok(true)
}

fn close_active_tab(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if state.borrow().tabs.active().is_none() {
        return Ok(());
    }
    if !autosave_active_tab_before_navigation(state)? {
        return Ok(());
    }
    state.borrow_mut().tabs.close_active();
    refresh_tabs(state)?;
    select_active_tab_node_in_tree(state);
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn close_tab_at_index(state: &Rc<RefCell<LinuxState>>, index: usize) -> Result<(), AppError> {
    let active_index = state.borrow().tabs.active_index();
    let Some(active_index) = active_index else {
        return Ok(());
    };
    if index >= state.borrow().tabs.tabs().len() {
        return Ok(());
    }
    if active_index != index {
        if !autosave_active_tab_before_navigation(state)? {
            return Ok(());
        }
        state.borrow_mut().tabs.set_active(index);
        refresh_tabs(state)?;
        reset_active_editor_undo_stack(state);
        select_active_tab_node_in_tree(state);
        update_actions(state);
        update_window_title(state);
    }
    close_active_tab(state)
}

fn handle_window_close(state: &Rc<RefCell<LinuxState>>) -> Result<bool, AppError> {
    let can_close = autosave_all_dirty_tabs(state)?;
    if can_close {
        if let Err(error) = persist_ui_settings(state) {
            report_error(state, &error);
        }
    }
    Ok(can_close)
}

fn persist_ui_settings(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let mut state = state.borrow_mut();
    let app_settings = state.app.ui_settings();
    let mut current_settings = app_settings.clone();
    if let Some(node_id) = state.pending_selection_node_id {
        current_settings.selection.node_id = Some(node_id);
    }
    let settings = persisted_ui_settings(
        current_settings.clone(),
        state.widgets.window.width(),
        state.widgets.window.height(),
        state.widgets.paned.position(),
        state.tree_mode,
        &state.search_query,
        state.selected_node_id,
    );
    if settings == app_settings {
        state.pending_selection_node_id = None;
        return Ok(());
    }

    state.app.save_ui_settings(settings)?;
    state.pending_selection_node_id = None;
    Ok(())
}

fn persisted_ui_settings(
    mut settings: UiSettings,
    window_width: i32,
    window_height: i32,
    splitter_position: i32,
    tree_mode: TreeMode,
    search_query: &str,
    selected_node_id: Option<i64>,
) -> UiSettings {
    settings.window = persisted_window_settings(settings.window, window_width, window_height);
    settings.splitter = SplitterSettings::new(clamp_split_width_for_window(
        splitter_position,
        window_width,
    ));
    settings.selection.node_id = selection_node_id_for_ui_settings(
        tree_mode,
        search_query,
        selected_node_id,
        settings.selection.node_id,
    );
    settings
}

fn persisted_window_settings(current: WindowSettings, width: i32, height: i32) -> WindowSettings {
    if width <= 0 || height <= 0 {
        return current;
    }

    current.with_size(width, height)
}

fn clamp_split_width_for_window(left_width: i32, window_width: i32) -> i32 {
    if window_width <= 0 {
        return 0;
    }

    let full_width = window_width.saturating_sub(SPLITTER_WIDTH_PX).max(0);
    let max_with_editor = window_width
        .saturating_sub(SPLITTER_WIDTH_PX)
        .saturating_sub(MIN_EDITOR_WIDTH_PX)
        .max(0);
    let max_width = if max_with_editor >= MIN_SPLIT_WIDTH_PX {
        max_with_editor
    } else {
        full_width
    };
    let min_width = MIN_SPLIT_WIDTH_PX.min(max_width);
    left_width.clamp(min_width, max_width)
}

fn schedule_selection_ui_setting_save(state: &Rc<RefCell<LinuxState>>, node_id: i64) {
    let generation = {
        let mut state = state.borrow_mut();
        let saved_selection = state.app.ui_settings_ref().selection.node_id;
        let current_selection = state.pending_selection_node_id.or(saved_selection);
        let next_selection = selection_node_id_for_ui_settings(
            state.tree_mode,
            &state.search_query,
            Some(node_id),
            current_selection,
        );

        state.selection_ui_settings_generation =
            state.selection_ui_settings_generation.saturating_add(1);
        if next_selection == saved_selection {
            state.pending_selection_node_id = None;
            return;
        }

        state.pending_selection_node_id = next_selection;
        state.selection_ui_settings_generation
    };

    let state = Rc::clone(state);
    glib::timeout_add_local_once(
        Duration::from_millis(SELECTION_UI_SETTINGS_SAVE_DEBOUNCE_MS),
        move || {
            let is_current = state.borrow().selection_ui_settings_generation == generation;
            if !is_current {
                return;
            }
            let result = flush_pending_selection_ui_setting(&state);
            run_and_report(&state, result);
        },
    );
}

fn flush_pending_selection_ui_setting(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let mut state = state.borrow_mut();
    let Some(node_id) = state.pending_selection_node_id else {
        return Ok(());
    };

    let mut settings = state.app.ui_settings();
    if settings.selection.node_id == Some(node_id) {
        state.pending_selection_node_id = None;
        return Ok(());
    }

    settings.selection.node_id = Some(node_id);
    state.app.save_ui_settings(settings)?;
    state.pending_selection_node_id = None;
    Ok(())
}

fn save_current_selection_ui_setting(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let mut state = state.borrow_mut();
    let mut settings = state.app.ui_settings();
    let next_selection = selection_node_id_for_ui_settings(
        state.tree_mode,
        &state.search_query,
        state.selected_node_id,
        settings.selection.node_id,
    );
    if settings.selection.node_id != next_selection {
        settings.selection.node_id = next_selection;
        state.app.save_ui_settings(settings)?;
    }
    Ok(())
}

fn selection_node_id_for_ui_settings(
    tree_mode: TreeMode,
    search_query: &str,
    selected_node_id: Option<i64>,
    current_selection_node_id: Option<i64>,
) -> Option<i64> {
    if tree_mode == TreeMode::Active && search_query.trim().is_empty() {
        selected_node_id.or(current_selection_node_id)
    } else {
        current_selection_node_id
    }
}

fn reload_active_document_and_tree_with_expansion(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
    expansion: TreeRefreshExpansion,
) -> Result<(), AppError> {
    state.borrow_mut().app.reload_document()?;
    reload_visible_document_with_expansion(state, preferred_node_id, expansion)
}

fn reload_visible_document(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    reload_visible_document_with_expansion(state, preferred_node_id, TreeRefreshExpansion::Preserve)
}

fn reload_visible_document_for_label_edit(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<(), AppError> {
    state.borrow_mut().editing_node_id = Some(node_id);
    let result = reload_visible_document(state, Some(node_id));
    if result.is_err() {
        state.borrow_mut().editing_node_id = None;
    }
    result
}

fn reload_visible_document_with_expansion(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
    expansion: TreeRefreshExpansion,
) -> Result<(), AppError> {
    let document = {
        let state = state.borrow();
        load_ui_document(&state)?
    };
    {
        let mut state = state.borrow_mut();
        state.document = document;
    }
    apply_tree_refresh_expansion(state, expansion);
    sync_tabs_from_visible_document_preserving_content(state);
    rebuild_tree_list(state, preferred_node_id)?;
    refresh_tabs(state)?;
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn reload_visible_tree_preserving_tabs_with_expansion(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
    expansion: TreeRefreshExpansion,
) -> Result<(), AppError> {
    let document = {
        let state = state.borrow();
        load_ui_document(&state)?
    };
    {
        let mut state = state.borrow_mut();
        state.document = document;
    }
    apply_tree_refresh_expansion(state, expansion);
    sync_tabs_from_visible_document_preserving_content(state);
    rebuild_tree_list(state, preferred_node_id)?;
    update_actions(state);
    update_window_title(state);
    Ok(())
}

fn refresh_visible_document_after_stable_tree_row_rename(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<bool, AppError> {
    let document = {
        let state = state.borrow();
        load_ui_document(&state)?
    };
    let can_refresh_row = {
        let state = state.borrow();
        can_refresh_renamed_tree_row(&state.document, &document, &state.visible_node_ids, node_id)
    };
    if !can_refresh_row {
        return Ok(false);
    }

    state.borrow_mut().document = document;
    sync_tabs_from_visible_document_preserving_content(state);
    if !refresh_visible_tree_row(state, node_id, Some(node_id))? {
        rebuild_tree_list(state, Some(node_id))?;
    }
    refresh_tabs(state)?;
    update_actions(state);
    update_window_title(state);
    Ok(true)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeRefreshExpansion {
    Preserve,
    ExpandAll,
}

fn apply_tree_refresh_expansion(state: &Rc<RefCell<LinuxState>>, expansion: TreeRefreshExpansion) {
    let expanded_node_ids = {
        let state = state.borrow();
        expanded_node_ids_for_tree_refresh(&state.document, &state.expanded_node_ids, expansion)
    };
    state.borrow_mut().expanded_node_ids = expanded_node_ids;
}

fn expanded_node_ids_for_tree_refresh(
    document: &UiDocument,
    current: &HashSet<i64>,
    expansion: TreeRefreshExpansion,
) -> HashSet<i64> {
    match expansion {
        TreeRefreshExpansion::Preserve => current.clone(),
        TreeRefreshExpansion::ExpandAll => document.expandable_node_ids(),
    }
}

fn load_ui_document(state: &LinuxState) -> Result<UiDocument, AppError> {
    let language = state.app.ui_settings().language;
    if state.tree_mode == TreeMode::Trash {
        return UiDocument::from_trash_nodes(&state.app.deleted_nodes()?, language);
    }
    if !state.search_query.trim().is_empty() {
        return UiDocument::from_search_results(
            state.app.search_documents(state.search_query.trim())?,
            language,
        );
    }
    UiDocument::from_active_document(state.app.document(), language)
}

fn sync_tabs_from_visible_document(
    state: &Rc<RefCell<LinuxState>>,
    update_dirty_token: bool,
) -> Result<(), AppError> {
    let targets: Vec<(i64, String, UiNode)> = {
        let state = state.borrow();
        state
            .tabs
            .tabs()
            .iter()
            .filter_map(|tab| {
                state
                    .document
                    .node_by_id(tab.node_id)
                    .cloned()
                    .map(|node| (tab.node_id, tab.loaded_updated_at.clone(), node))
            })
            .collect()
    };

    let mut active_content_node_ids = Vec::new();
    let mut deleted_content_node_ids = Vec::new();
    for (_, loaded_updated_at, node) in &targets {
        if tab_content_is_current(loaded_updated_at, node) {
            state.borrow_mut().tabs.sync_loaded_tab_metadata(
                node.id,
                node.parent_id,
                node.title.clone(),
                node.editable,
                node.source,
            );
            continue;
        }

        match node.source {
            DocumentTabSource::ActiveTree | DocumentTabSource::SearchResult => {
                active_content_node_ids.push(node.id);
            }
            DocumentTabSource::Trash => {
                deleted_content_node_ids.push(node.id);
            }
        }
    }

    let (mut active_contents, mut deleted_contents) =
        load_visible_tab_contents(state, &active_content_node_ids, &deleted_content_node_ids)?;

    for (_, loaded_updated_at, node) in targets {
        if tab_content_is_current(&loaded_updated_at, &node) {
            continue;
        }

        let Some(input) = tab_input_from_ui_node_with_contents(
            node,
            &mut active_contents,
            &mut deleted_contents,
        )?
        else {
            continue;
        };
        state
            .borrow_mut()
            .tabs
            .sync_loaded_tab(input, update_dirty_token);
    }
    Ok(())
}

fn sync_tabs_from_visible_document_preserving_content(state: &Rc<RefCell<LinuxState>>) {
    let tab_count = state.borrow().tabs.tabs().len();
    for index in 0..tab_count {
        let tab_node_id = state.borrow().tabs.tabs()[index].node_id;
        let node = state.borrow().document.node_by_id(tab_node_id).cloned();
        let Some(node) = node else {
            continue;
        };
        state
            .borrow_mut()
            .tabs
            .sync_loaded_tab_metadata_preserving_content_at(
                index,
                LoadedTabMetadataUpdate {
                    node_id: node.id,
                    parent_id: node.parent_id,
                    title: node.title,
                    loaded_updated_at: node.updated_at,
                    editable: node.editable,
                    source: node.source,
                    current_content_for_dirty_token: None,
                },
            );
    }
}

fn sync_tabs_from_active_document_local_metadata(
    state: &Rc<RefCell<LinuxState>>,
    update_dirty_token: bool,
) -> Result<bool, AppError> {
    let state = &mut *state.borrow_mut();
    let app = &state.app;
    let tabs = &mut state.tabs;
    sync_tabs_from_active_document_local_metadata_for_tabs(app, tabs, update_dirty_token)
}

fn sync_tabs_from_active_document_local_metadata_for_tabs(
    app: &App,
    tabs: &mut OpenTabs,
    update_dirty_token: bool,
) -> Result<bool, AppError> {
    let tab_sync_targets: Vec<(usize, i64, bool)> = tabs
        .tabs()
        .iter()
        .enumerate()
        .map(|(index, tab)| (index, tab.node_id, tab.dirty))
        .collect();
    let dirty_tab_node_ids: Vec<i64> = if update_dirty_token {
        tab_sync_targets
            .iter()
            .filter_map(|(_, node_id, dirty)| dirty.then_some(*node_id))
            .collect()
    } else {
        Vec::new()
    };
    let current_contents = app.load_active_node_contents_if_present(&dirty_tab_node_ids)?;
    let mut active_tab_node_ids = Vec::with_capacity(tab_sync_targets.len());

    for (index, node_id, was_dirty) in tab_sync_targets {
        let Some((node_id, parent_id, title, node_updated_at)) = app
            .document()
            .node_by_id(node_id)
            .filter(|node| node.deleted_at.is_none())
            .map(|node| {
                (
                    node.id,
                    node.parent_id,
                    node.title.clone(),
                    node.updated_at.clone(),
                )
            })
        else {
            continue;
        };

        let current_content = if update_dirty_token && was_dirty {
            current_contents.get(&node_id)
        } else {
            None
        };
        let current_content_for_dirty_token = current_content.map(|(content, _)| content.as_str());
        let loaded_updated_at =
            current_content.map_or(node_updated_at, |(_, updated_at)| updated_at.clone());

        active_tab_node_ids.push(node_id);
        tabs.sync_loaded_tab_metadata_preserving_content_at(
            index,
            LoadedTabMetadataUpdate {
                node_id,
                parent_id,
                title,
                loaded_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
                current_content_for_dirty_token,
            },
        );
    }

    Ok(tabs.mark_tabs_missing_from_active_document_read_only(&active_tab_node_ids))
}

fn sync_tabs_from_reloaded_active_document_metadata(
    state: &Rc<RefCell<LinuxState>>,
    update_dirty_token: bool,
    reloaded_node_id: Option<i64>,
) -> Result<bool, AppError> {
    let state = &mut *state.borrow_mut();
    let app = &state.app;
    let tabs = &mut state.tabs;
    sync_tabs_from_reloaded_active_document_metadata_for_tabs(
        app,
        tabs,
        update_dirty_token,
        reloaded_node_id,
    )
}

fn sync_tabs_from_reloaded_active_document_metadata_for_tabs(
    app: &App,
    tabs: &mut OpenTabs,
    update_dirty_token: bool,
    reloaded_node_id: Option<i64>,
) -> Result<bool, AppError> {
    let tab_sync_targets: Vec<(usize, i64, String)> = tabs
        .tabs()
        .iter()
        .enumerate()
        .map(|(index, tab)| (index, tab.node_id, tab.loaded_updated_at.clone()))
        .collect();
    let mut tab_metadata = Vec::with_capacity(tab_sync_targets.len());

    for (index, node_id, loaded_updated_at) in tab_sync_targets {
        let Some(node) = app
            .document()
            .node_by_id(node_id)
            .filter(|node| node.deleted_at.is_none())
        else {
            continue;
        };
        tab_metadata.push((
            index,
            node.id,
            node.parent_id,
            node.title.clone(),
            node.updated_at.clone(),
            loaded_updated_at,
        ));
    }

    let content_node_ids: Vec<i64> = tab_metadata
        .iter()
        .filter_map(|(_, node_id, _, _, updated_at, loaded_updated_at)| {
            (Some(*node_id) != reloaded_node_id && loaded_updated_at != updated_at)
                .then_some(*node_id)
        })
        .collect();
    let mut contents = app.load_active_node_contents_if_present(&content_node_ids)?;
    let mut active_tab_node_ids = Vec::with_capacity(tab_metadata.len());

    for (index, node_id, parent_id, title, updated_at, loaded_updated_at) in tab_metadata {
        if Some(node_id) == reloaded_node_id || loaded_updated_at == updated_at {
            active_tab_node_ids.push(node_id);
            tabs.sync_loaded_tab_metadata_preserving_content_at(
                index,
                LoadedTabMetadataUpdate {
                    node_id,
                    parent_id,
                    title,
                    loaded_updated_at: updated_at,
                    editable: true,
                    source: DocumentTabSource::ActiveTree,
                    current_content_for_dirty_token: None,
                },
            );
            continue;
        }

        let Some((content, loaded_updated_at)) = contents.remove(&node_id) else {
            continue;
        };
        active_tab_node_ids.push(node_id);
        tabs.sync_loaded_tab(
            OpenDocumentTabInput {
                node_id,
                parent_id,
                title,
                content,
                loaded_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            },
            update_dirty_token,
        );
    }

    Ok(tabs.mark_tabs_missing_from_active_document_read_only(&active_tab_node_ids))
}

fn load_visible_tab_contents(
    state: &Rc<RefCell<LinuxState>>,
    active_node_ids: &[i64],
    deleted_node_ids: &[i64],
) -> Result<(TabContentByNodeId, TabContentByNodeId), AppError> {
    if active_node_ids.is_empty() && deleted_node_ids.is_empty() {
        return Ok((HashMap::new(), HashMap::new()));
    }

    let active_contents = state
        .borrow()
        .app
        .load_active_node_contents_if_present(active_node_ids)?;
    let deleted_contents = state
        .borrow()
        .app
        .load_deleted_node_contents(deleted_node_ids)?;

    for node_id in deleted_node_ids {
        if !deleted_contents.contains_key(node_id) {
            return Err(DomainError::NodeNotFound { node_id: *node_id }.into());
        }
    }

    Ok((active_contents, deleted_contents))
}

fn tab_input_from_ui_node_with_contents(
    node: UiNode,
    active_contents: &mut TabContentByNodeId,
    deleted_contents: &mut TabContentByNodeId,
) -> Result<Option<OpenDocumentTabInput>, AppError> {
    let content = match node.source {
        DocumentTabSource::ActiveTree | DocumentTabSource::SearchResult => {
            let Some(content) = active_contents.remove(&node.id) else {
                return Ok(None);
            };
            content
        }
        DocumentTabSource::Trash => {
            let Some(content) = deleted_contents.remove(&node.id) else {
                return Err(DomainError::NodeNotFound { node_id: node.id }.into());
            };
            content
        }
    };

    Ok(Some(OpenDocumentTabInput {
        node_id: node.id,
        parent_id: node.parent_id,
        title: node.title,
        content: content.0,
        loaded_updated_at: content.1,
        editable: node.editable,
        source: node.source,
    }))
}

fn tab_content_is_current(loaded_updated_at: &str, node: &UiNode) -> bool {
    loaded_updated_at == node.updated_at.as_str()
}

fn refresh_tabs(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    if state.borrow().editor_content_pending_sync {
        sync_active_editor_content(state)?;
    }

    let (notebook, settings, desired_tab_count, active_index, update_in_place) = {
        let state = state.borrow();
        let desired_tab_count = state.tabs.tabs().len();
        (
            state.widgets.notebook.clone(),
            state.app.ui_settings(),
            desired_tab_count,
            state.tabs.active_index(),
            can_refresh_tabs_in_place(
                state.widgets.notebook.n_pages(),
                state.tab_pages.len(),
                desired_tab_count,
            ),
        )
    };

    if update_in_place {
        refresh_existing_tab_pages(state, &notebook, &settings);
        return Ok(());
    }

    {
        let mut state = state.borrow_mut();
        state.suppress_tab_change = true;
        state.tab_pages.clear();
    }
    while notebook.n_pages() > 0 {
        notebook.remove_page(Some(0));
    }

    let mut tab_pages = Vec::with_capacity(desired_tab_count);
    for index in 0..desired_tab_count {
        let Some((page, title)) = build_tab_page_for_index(state, index, &settings) else {
            continue;
        };
        let tab_label = build_tab_label(state, &page.page, index, &title);
        notebook.append_page(&page.page, Some(&tab_label));
        notebook.set_tab_reorderable(&page.page, true);
        connect_text_page_signals(state, &page);
        tab_pages.push(page);
    }
    state.borrow_mut().tab_pages = tab_pages;

    if let Some(active_index) = active_index {
        notebook.set_current_page(Some(active_index as u32));
    }
    state.borrow_mut().suppress_tab_change = false;
    update_caret_status(state);
    update_actions(state);
    Ok(())
}

fn can_refresh_tabs_in_place(
    notebook_page_count: u32,
    tab_page_count: usize,
    desired_tab_count: usize,
) -> bool {
    usize::try_from(notebook_page_count).ok() == Some(desired_tab_count)
        && tab_page_count == desired_tab_count
}

fn refresh_existing_tab_pages(
    state: &Rc<RefCell<LinuxState>>,
    notebook: &gtk::Notebook,
    settings: &crate::domain::UiSettings,
) {
    let (updates, active_index) = existing_tab_page_updates(state);
    state.borrow_mut().suppress_tab_change = true;
    for (index, update) in updates.iter().enumerate() {
        update_existing_tab_page(state, update, settings);
        let tab_label = build_tab_label(state, &update.page.page, index, &update.title);
        notebook.set_tab_label(&update.page.page, Some(&tab_label));
        notebook.set_tab_reorderable(&update.page.page, true);
    }
    if let Some(active_index) = active_index {
        notebook.set_current_page(Some(active_index as u32));
    }
    state.borrow_mut().suppress_tab_change = false;

    update_caret_status(state);
    update_actions(state);
}

fn existing_tab_page_updates(
    state: &Rc<RefCell<LinuxState>>,
) -> (Vec<ExistingTabPageUpdate>, Option<usize>) {
    let state = state.borrow();
    let updates = state
        .tab_pages
        .iter()
        .zip(state.tabs.tabs().iter())
        .map(|(page, tab)| {
            let node_id = tab.node_id;
            let source = tab.source;
            let content_revision = tab.content_revision();
            let content_is_current = page.node_id.get() == node_id
                && page.source.get() == source
                && page.content_revision.get() == content_revision;
            ExistingTabPageUpdate {
                page: page.clone(),
                title: tab.display_title(),
                node_id,
                source,
                editable: tab.editable,
                view_state: tab.view_state,
                content_revision,
                content: (!content_is_current).then(|| tab.content.clone()),
            }
        })
        .collect();
    (updates, state.tabs.active_index())
}

fn build_tab_page_for_index(
    state: &Rc<RefCell<LinuxState>>,
    index: usize,
    settings: &crate::domain::UiSettings,
) -> Option<(TabPage, String)> {
    let state = state.borrow();
    let tab = state.tabs.tabs().get(index)?;
    let buffer = gtk::TextBuffer::builder()
        .text(&tab.content)
        .enable_undo(true)
        .build();
    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .monospace(false)
        .editable(tab.editable)
        .cursor_visible(tab.editable)
        .wrap_mode(if settings.editor.word_wrap {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        })
        .vexpand(true)
        .hexpand(true)
        .build();
    view.add_css_class("j3-editor");

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(if settings.editor.word_wrap {
            gtk::PolicyType::Never
        } else {
            gtk::PolicyType::Automatic
        })
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&view)
        .build();
    scroller.add_css_class("j3-editor-scroller");

    restore_buffer_view_state(&view, &buffer, &tab.content, tab.view_state);
    Some((
        TabPage {
            page: scroller,
            view,
            buffer,
            node_id: Rc::new(Cell::new(tab.node_id)),
            source: Rc::new(Cell::new(tab.source)),
            content_revision: Rc::new(Cell::new(tab.content_revision())),
            caret_status_cache: Rc::new(RefCell::new(None)),
        },
        tab.display_title(),
    ))
}

fn update_existing_tab_page(
    state: &Rc<RefCell<LinuxState>>,
    update: &ExistingTabPageUpdate,
    settings: &crate::domain::UiSettings,
) {
    let page = &update.page;
    let editor_access_changed = page.view.is_editable() != update.editable
        || page.view.is_cursor_visible() != update.editable;
    page.view.set_editable(update.editable);
    page.view.set_cursor_visible(update.editable);
    page.view.set_wrap_mode(if settings.editor.word_wrap {
        gtk::WrapMode::WordChar
    } else {
        gtk::WrapMode::None
    });
    page.page
        .set_hscrollbar_policy(if settings.editor.word_wrap {
            gtk::PolicyType::Never
        } else {
            gtk::PolicyType::Automatic
        });
    invalidate_caret_status_cache(page);

    let Some(content) = update.content.as_deref() else {
        if editor_access_changed {
            reset_text_buffer_undo_stack(&page.buffer);
        }
        return;
    };

    state.borrow_mut().suppress_editor_change = true;
    page.buffer.set_text(content);
    restore_buffer_view_state(&page.view, &page.buffer, content, update.view_state);
    reset_text_buffer_undo_stack(&page.buffer);
    invalidate_caret_status_cache(page);
    page.node_id.set(update.node_id);
    page.source.set(update.source);
    page.content_revision.set(update.content_revision);
    state.borrow_mut().suppress_editor_change = false;
}

fn reset_active_editor_undo_stack(state: &Rc<RefCell<LinuxState>>) {
    if let Some(page) = active_page_cloned(state) {
        reset_text_buffer_undo_stack(&page.buffer);
    }
}

fn reload_active_editor_page_from_tab_state(state: &Rc<RefCell<LinuxState>>) {
    let (page, content, view_state) = {
        let state = state.borrow();
        let Some(page) = active_page(&state) else {
            return;
        };
        let Some(tab) = state.tabs.active() else {
            return;
        };
        (page.clone(), tab.content.clone(), tab.view_state)
    };
    clear_find_match_highlight(&page.buffer);
    restore_buffer_view_state(&page.view, &page.buffer, &content, view_state);
    reset_text_buffer_undo_stack(&page.buffer);
    state.borrow_mut().editor_content_pending_sync = false;
}

fn reset_text_buffer_undo_stack(buffer: &gtk::TextBuffer) {
    if buffer.enables_undo() {
        buffer.set_enable_undo(false);
        buffer.set_enable_undo(true);
    }
}

fn build_tab_label(
    state: &Rc<RefCell<LinuxState>>,
    page: &gtk::ScrolledWindow,
    index: usize,
    title: &str,
) -> gtk::Box {
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    let close = gtk::Button::builder()
        .label("x")
        .has_frame(false)
        .focusable(false)
        .build();
    close.add_css_class("flat");
    close.add_css_class("j3-tab-close");
    close.set_size_request(TAB_CLOSE_HIT_WIDTH_PX, -1);
    close.set_margin_end(TAB_CLOSE_HIT_RIGHT_PADDING_PX);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("j3-tab-label");
    row.set_size_request(-1, TAB_BAR_HEIGHT_PX);
    row.append(&label);
    row.append(&close);

    let state_clone = Rc::clone(state);
    let page = page.clone();
    close.connect_clicked(move |_| {
        let index = state_clone
            .borrow()
            .widgets
            .notebook
            .page_num(&page)
            .and_then(|index| usize::try_from(index).ok())
            .unwrap_or(index);
        run_and_report(&state_clone, close_tab_at_index(&state_clone, index));
    });

    row
}

fn refresh_tab_labels(state: &Rc<RefCell<LinuxState>>) {
    let (notebook, pages, titles, active_index) = {
        let state = state.borrow();
        (
            state.widgets.notebook.clone(),
            state.tab_pages.clone(),
            state
                .tabs
                .tabs()
                .iter()
                .map(|tab| tab.display_title())
                .collect::<Vec<_>>(),
            state.tabs.active_index(),
        )
    };

    state.borrow_mut().suppress_tab_change = true;
    for (index, (page, title)) in pages.iter().zip(titles.iter()).enumerate() {
        let tab_label = build_tab_label(state, &page.page, index, title);
        notebook.set_tab_label(&page.page, Some(&tab_label));
        notebook.set_tab_reorderable(&page.page, true);
    }
    if let Some(active_index) = active_index {
        notebook.set_current_page(Some(active_index as u32));
    }
    state.borrow_mut().suppress_tab_change = false;

    update_caret_status(state);
    update_actions(state);
}

fn connect_text_page_signals(state: &Rc<RefCell<LinuxState>>, tab_page: &TabPage) {
    let page = tab_page.page.clone();
    let view = tab_page.view.clone();
    let buffer = tab_page.buffer.clone();
    {
        let state = Rc::clone(state);
        let page = page.clone();
        let caret_status_cache = Rc::clone(&tab_page.caret_status_cache);
        buffer.connect_changed(move |_| {
            caret_status_cache.borrow_mut().take();
            run_and_report(&state, handle_editor_changed_for_page(&state, &page));
        });
    }
    {
        let state = Rc::clone(state);
        buffer.connect_mark_set(move |buffer, _, mark| {
            if is_insert_mark(buffer, mark) {
                update_caret_status(&state);
                update_actions(&state);
            } else if is_selection_bound_mark(buffer, mark) {
                update_actions(&state);
            }
        });
    }
    {
        let state = Rc::clone(state);
        let page = page.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| {
            remember_editor_context_menu_point(&state, &page, x, y);
        });
        view.add_controller(motion);
    }
    {
        let state = Rc::clone(state);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |controller, key, _, modifiers| {
            let primary = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
            if primary && !alt && matches!(key, gdk::Key::a | gdk::Key::A) {
                run_and_report(&state, editor_select_all_action(&state));
                glib::Propagation::Stop
            } else if is_context_menu_key(key, modifiers) {
                if let Some(widget) = controller.widget() {
                    show_editor_context_menu(&state, &widget);
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        view.add_controller(controller);
    }
}

fn update_caret_status(state: &Rc<RefCell<LinuxState>>) {
    let (label, language, line, column) = {
        let state = state.borrow();
        let language = state.app.ui_settings().language;
        match active_page(&state) {
            Some(page) => {
                let (line, column) = caret_line_column_for_page(page);
                (state.widgets.caret_status.clone(), language, line, column)
            }
            None => (state.widgets.caret_status.clone(), language, 1, 1),
        }
    };
    label.set_text(&ui_text(language).caret_position_status(line, column));
}

fn is_insert_mark(buffer: &gtk::TextBuffer, mark: &gtk::TextMark) -> bool {
    mark == &buffer.get_insert()
}

fn is_selection_bound_mark(buffer: &gtk::TextBuffer, mark: &gtk::TextMark) -> bool {
    mark == &buffer.selection_bound()
}

fn invalidate_caret_status_cache(page: &TabPage) {
    page.caret_status_cache.borrow_mut().take();
}

fn invalidate_all_caret_status_caches(state: &Rc<RefCell<LinuxState>>) {
    for page in &state.borrow().tab_pages {
        invalidate_caret_status_cache(page);
    }
}

fn caret_line_column_for_page(page: &TabPage) -> (usize, usize) {
    let caret = page.buffer.iter_at_offset(page.buffer.cursor_position());
    let mut line_start = caret;
    if !page.view.starts_display_line(&line_start) {
        page.view.backward_display_line_start(&mut line_start);
    }

    let line = display_line_number_for_page(page, line_start.offset())
        .unwrap_or_else(|| usize::try_from(caret.line()).unwrap_or(0) + 1);
    let column = caret_column_utf16_for_range(&line_start, &caret);

    (line, column)
}

fn display_line_number_for_page(page: &TabPage, target_offset: i32) -> Option<usize> {
    if target_offset <= 0 {
        store_caret_status_cache(page, 1, 0);
        return Some(1);
    }

    let view_width = page.view.width();
    let cached = *page.caret_status_cache.borrow();
    if let Some(cache) = cached.filter(|cache| cache.view_width == view_width) {
        if cache.line_start_offset == target_offset {
            return Some(cache.display_line_number);
        }
        let cached_line = display_line_number_between(
            &page.view,
            &page.buffer,
            cache.line_start_offset,
            cache.display_line_number,
            target_offset,
        );
        if let Some(line) = cached_line {
            store_caret_status_cache(page, line, target_offset);
            return Some(line);
        }
    }

    let line = display_line_number_between(&page.view, &page.buffer, 0, 1, target_offset)?;
    store_caret_status_cache(page, line, target_offset);
    Some(line)
}

fn store_caret_status_cache(page: &TabPage, display_line_number: usize, line_start_offset: i32) {
    *page.caret_status_cache.borrow_mut() = Some(CaretStatusCache {
        view_width: page.view.width(),
        line_start_offset,
        display_line_number,
    });
}

fn display_line_number_between(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    start_offset: i32,
    start_line: usize,
    target_offset: i32,
) -> Option<usize> {
    if start_offset <= target_offset {
        return display_line_number_forward_between(
            view,
            buffer,
            start_offset,
            start_line,
            target_offset,
        );
    }
    display_line_number_backward_between(view, buffer, start_offset, start_line, target_offset)
}

fn display_line_number_forward_between(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    start_offset: i32,
    start_line: usize,
    target_offset: i32,
) -> Option<usize> {
    let mut iter = buffer.iter_at_offset(start_offset);
    let mut line = start_line;
    while iter.offset() < target_offset {
        let previous_offset = iter.offset();
        if !view.forward_display_line(&mut iter) || iter.offset() <= previous_offset {
            return None;
        }
        line = line.saturating_add(1);
    }
    (iter.offset() == target_offset).then_some(line)
}

fn display_line_number_backward_between(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    start_offset: i32,
    start_line: usize,
    target_offset: i32,
) -> Option<usize> {
    let mut iter = buffer.iter_at_offset(start_offset);
    let mut line = start_line;
    while iter.offset() > target_offset {
        let previous_offset = iter.offset();
        if !view.backward_display_line(&mut iter) || iter.offset() >= previous_offset {
            return None;
        }
        if !view.starts_display_line(&iter) {
            view.backward_display_line_start(&mut iter);
        }
        line = line.checked_sub(1)?;
    }
    (iter.offset() == target_offset).then_some(line)
}

fn caret_column_utf16_for_range(line_start: &gtk::TextIter, caret: &gtk::TextIter) -> usize {
    if caret.offset() <= line_start.offset() {
        return 1;
    }

    let mut iter = *line_start;
    let mut column = 1usize;
    while iter.offset() < caret.offset() {
        column = column.saturating_add(iter.char().len_utf16());
        if !iter.forward_char() {
            break;
        }
    }
    column
}

#[cfg(test)]
fn caret_column_utf16_for_line_prefix(line_prefix: &str) -> usize {
    line_prefix.encode_utf16().count().saturating_add(1)
}

fn apply_editor_word_wrap(state: &Rc<RefCell<LinuxState>>, word_wrap: bool) {
    let pages = state.borrow().tab_pages.clone();
    apply_editor_word_wrap_to_pages(&pages, word_wrap);
    update_caret_status(state);
}

fn apply_editor_word_wrap_to_pages(pages: &[TabPage], word_wrap: bool) {
    for page in pages {
        page.view.set_wrap_mode(if word_wrap {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        });
        page.page.set_hscrollbar_policy(if word_wrap {
            gtk::PolicyType::Never
        } else {
            gtk::PolicyType::Automatic
        });
        invalidate_caret_status_cache(page);
    }
}

#[derive(Clone, PartialEq, Eq)]
struct TreeRowSpec {
    node_id: i64,
    depth: usize,
    title: String,
    display_title: String,
    has_children: bool,
    expanded: bool,
    editing: bool,
}

fn tree_row_specs_for_order(
    document: &UiDocument,
    order: &[(usize, usize)],
    expanded_node_ids: &HashSet<i64>,
    editing_node_id: Option<i64>,
) -> Vec<TreeRowSpec> {
    order
        .iter()
        .map(|(node_index, depth)| {
            let node = &document.nodes[*node_index];
            TreeRowSpec {
                node_id: node.id,
                depth: *depth,
                title: node.title.clone(),
                display_title: node.display_title.clone(),
                has_children: document.has_display_children(node.id),
                expanded: expanded_node_ids.contains(&node.id),
                editing: editing_node_id == Some(node.id),
            }
        })
        .collect()
}

fn tree_row_spec_for_node(
    document: &UiDocument,
    node_id: i64,
    expanded_node_ids: &HashSet<i64>,
    editing_node_id: Option<i64>,
) -> Option<TreeRowSpec> {
    let node = document.node_by_id(node_id)?;
    Some(TreeRowSpec {
        node_id: node.id,
        depth: document.display_depth(node.id),
        title: node.title.clone(),
        display_title: node.display_title.clone(),
        has_children: document.has_display_children(node.id),
        expanded: expanded_node_ids.contains(&node.id),
        editing: editing_node_id == Some(node.id),
    })
}

fn build_tree_row(
    state: &Rc<RefCell<LinuxState>>,
    spec: &TreeRowSpec,
) -> (gtk::ListBoxRow, Option<gtk::Entry>) {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("j3-tree-row");
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    container.set_margin_top(TREE_ROW_VERTICAL_PADDING_PX);
    container.set_margin_bottom(TREE_ROW_VERTICAL_PADDING_PX);
    container
        .set_margin_start(TREE_ROW_HORIZONTAL_PADDING_PX + (spec.depth as i32 * TREE_INDENT_PX));
    container.set_margin_end(TREE_ROW_HORIZONTAL_PADDING_PX);

    if spec.has_children {
        let toggle = gtk::Button::builder()
            .label(if spec.expanded { "▾" } else { "▸" })
            .has_frame(false)
            .focusable(false)
            .build();
        toggle.add_css_class("flat");
        toggle.add_css_class("j3-tree-expander");
        let state_for_toggle = Rc::clone(state);
        let node_id = spec.node_id;
        toggle.connect_clicked(move |_| {
            run_and_report(
                &state_for_toggle,
                toggle_tree_node_expanded(&state_for_toggle, node_id),
            );
        });
        container.append(&toggle);
    } else {
        let spacer = gtk::Label::new(None);
        spacer.set_width_chars(2);
        container.append(&spacer);
    }

    let mut focus_entry = None;
    if spec.editing {
        let entry = gtk::Entry::new();
        entry.add_css_class("j3-tree-entry");
        entry.set_text(&spec.title);
        entry.set_hexpand(true);
        let entry_for_activate = entry.clone();
        let state_for_activate = Rc::clone(state);
        entry.connect_activate(move |_| {
            run_and_report(
                &state_for_activate,
                commit_label_edit(&state_for_activate, entry_for_activate.text().as_str()),
            );
        });
        let focus = gtk::EventControllerFocus::new();
        let state_for_focus = Rc::clone(state);
        let entry_for_focus = entry.clone();
        let node_id = spec.node_id;
        focus.connect_leave(move |_| {
            if state_for_focus.borrow().editing_node_id == Some(node_id) {
                run_and_report(
                    &state_for_focus,
                    commit_label_edit(&state_for_focus, entry_for_focus.text().as_str()),
                );
            }
        });
        entry.add_controller(focus);
        let key = gtk::EventControllerKey::new();
        let state_for_key = Rc::clone(state);
        key.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                state_for_key.borrow_mut().editing_node_id = None;
                run_and_report(
                    &state_for_key,
                    refresh_visible_tree_row_or_rebuild(&state_for_key, node_id, Some(node_id)),
                );
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        entry.add_controller(key);
        container.append(&entry);
        focus_entry = Some(entry);
    } else {
        let label = gtk::Label::new(Some(&spec.display_title));
        label.set_xalign(0.0);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_hexpand(true);
        container.append(&label);
    }

    row.set_child(Some(&container));
    connect_tree_row_drag_drop(state, &row, spec.node_id);
    (row, focus_entry)
}

fn tree_row_specs_have_same_order(left: &[TreeRowSpec], right: &[TreeRowSpec]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.node_id == right.node_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TreeRowOrderChange {
    prefix_len: usize,
    previous_changed_len: usize,
    next_changed_len: usize,
    suffix_len: usize,
}

fn tree_row_order_change(
    previous: &[TreeRowSpec],
    next: &[TreeRowSpec],
) -> Option<TreeRowOrderChange> {
    if previous.is_empty() || next.is_empty() {
        return None;
    }

    let prefix_len = previous
        .iter()
        .zip(next)
        .take_while(|(previous, next)| previous.node_id == next.node_id)
        .count();
    if prefix_len == previous.len() && prefix_len == next.len() {
        return None;
    }

    let remaining_previous = previous.len().saturating_sub(prefix_len);
    let remaining_next = next.len().saturating_sub(prefix_len);
    let suffix_len = previous
        .iter()
        .rev()
        .take(remaining_previous)
        .zip(next.iter().rev().take(remaining_next))
        .take_while(|(previous, next)| previous.node_id == next.node_id)
        .count();

    if prefix_len == 0 && suffix_len == 0 {
        return None;
    }

    Some(TreeRowOrderChange {
        prefix_len,
        previous_changed_len: previous.len().saturating_sub(prefix_len + suffix_len),
        next_changed_len: next.len().saturating_sub(prefix_len + suffix_len),
        suffix_len,
    })
}

fn tree_has_no_extra_rows(tree: &gtk::ListBox, spec_count: usize) -> bool {
    i32::try_from(spec_count)
        .ok()
        .is_some_and(|index| tree.row_at_index(index).is_none())
}

fn tree_row_count_matches(tree: &gtk::ListBox, row_count: usize) -> bool {
    let Some(row_count_i32) = i32::try_from(row_count).ok() else {
        return false;
    };
    (row_count == 0 || tree.row_at_index(row_count_i32.saturating_sub(1)).is_some())
        && tree.row_at_index(row_count_i32).is_none()
}

fn refresh_tree_list_for_stable_order(
    state: &Rc<RefCell<LinuxState>>,
    tree: &gtk::ListBox,
    row_specs: Vec<TreeRowSpec>,
    preferred_node_id: Option<i64>,
) -> Result<(), Vec<TreeRowSpec>> {
    if !tree_has_no_extra_rows(tree, row_specs.len()) {
        return Err(row_specs);
    }

    let changed_indices = {
        let state = state.borrow();
        if !tree_row_specs_have_same_order(&state.visible_tree_row_specs, &row_specs) {
            return Err(row_specs);
        }
        state
            .visible_tree_row_specs
            .iter()
            .zip(&row_specs)
            .enumerate()
            .filter_map(|(index, (previous, next))| (previous != next).then_some(index))
            .collect::<Vec<_>>()
    };

    state.borrow_mut().suppress_tree_selection = true;
    let mut focus_entry = None;
    for index in changed_indices {
        let Some(row_index) = i32::try_from(index).ok() else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        let Some(row) = tree.row_at_index(row_index) else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        tree.remove(&row);
        let (row, row_focus_entry) = build_tree_row(state, &row_specs[index]);
        tree.insert(&row, row_index);
        if row_focus_entry.is_some() {
            focus_entry = row_focus_entry;
        }
    }

    {
        let mut state = state.borrow_mut();
        state.visible_tree_row_specs = row_specs;
    }
    select_preferred_visible_tree_row(state, preferred_node_id);
    state.borrow_mut().suppress_tree_selection = false;

    if let Some(entry) = focus_entry {
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    update_actions(state);
    Ok(())
}

fn refresh_tree_list_for_changed_order(
    state: &Rc<RefCell<LinuxState>>,
    tree: &gtk::ListBox,
    row_specs: Vec<TreeRowSpec>,
    preferred_node_id: Option<i64>,
) -> Result<(), Vec<TreeRowSpec>> {
    let (previous_len, change, prefix_changed_indices, suffix_changed_indices) = {
        let state = state.borrow();
        let Some(change) = tree_row_order_change(&state.visible_tree_row_specs, &row_specs) else {
            return Err(row_specs);
        };
        if !tree_row_count_matches(tree, state.visible_tree_row_specs.len()) {
            return Err(row_specs);
        }

        let mut prefix_changed_indices = Vec::new();
        for (index, (previous, next)) in state
            .visible_tree_row_specs
            .iter()
            .zip(&row_specs)
            .take(change.prefix_len)
            .enumerate()
        {
            if previous != next {
                prefix_changed_indices.push(index);
            }
        }

        let mut suffix_changed_indices = Vec::new();
        let previous_suffix_start = change
            .prefix_len
            .saturating_add(change.previous_changed_len);
        let next_suffix_start = change.prefix_len.saturating_add(change.next_changed_len);
        for offset in 0..change.suffix_len {
            let previous_index = previous_suffix_start.saturating_add(offset);
            let next_index = next_suffix_start.saturating_add(offset);
            if state.visible_tree_row_specs[previous_index] != row_specs[next_index] {
                suffix_changed_indices.push(next_index);
            }
        }

        (
            state.visible_tree_row_specs.len(),
            change,
            prefix_changed_indices,
            suffix_changed_indices,
        )
    };

    if !tree_row_count_matches(tree, previous_len) {
        return Err(row_specs);
    }

    state.borrow_mut().suppress_tree_selection = true;
    let mut focus_entry = None;
    for index in prefix_changed_indices {
        let Some(row_index) = i32::try_from(index).ok() else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        let Some(row) = tree.row_at_index(row_index) else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        tree.remove(&row);
        let (row, row_focus_entry) = build_tree_row(state, &row_specs[index]);
        tree.insert(&row, row_index);
        if row_focus_entry.is_some() {
            focus_entry = row_focus_entry;
        }
    }

    let Some(changed_start) = i32::try_from(change.prefix_len).ok() else {
        state.borrow_mut().suppress_tree_selection = false;
        return Err(row_specs);
    };
    for _ in 0..change.previous_changed_len {
        let Some(row) = tree.row_at_index(changed_start) else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        tree.remove(&row);
    }
    for offset in 0..change.next_changed_len {
        let insert_index = change.prefix_len.saturating_add(offset);
        let Some(row_index) = i32::try_from(insert_index).ok() else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        let (row, row_focus_entry) = build_tree_row(state, &row_specs[insert_index]);
        tree.insert(&row, row_index);
        if row_focus_entry.is_some() {
            focus_entry = row_focus_entry;
        }
    }

    for index in suffix_changed_indices {
        let Some(row_index) = i32::try_from(index).ok() else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        let Some(row) = tree.row_at_index(row_index) else {
            state.borrow_mut().suppress_tree_selection = false;
            return Err(row_specs);
        };
        tree.remove(&row);
        let (row, row_focus_entry) = build_tree_row(state, &row_specs[index]);
        tree.insert(&row, row_index);
        if row_focus_entry.is_some() {
            focus_entry = row_focus_entry;
        }
    }

    {
        let mut state = state.borrow_mut();
        state.visible_node_ids.clear();
        state.visible_node_ids.reserve(row_specs.len());
        state
            .visible_node_ids
            .extend(row_specs.iter().map(|spec| spec.node_id));
        state.visible_tree_row_specs = row_specs;
    }
    select_preferred_visible_tree_row(state, preferred_node_id);
    state.borrow_mut().suppress_tree_selection = false;

    if let Some(entry) = focus_entry {
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    update_actions(state);
    Ok(())
}

fn rebuild_tree_list(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let (tree, row_specs) = {
        if let Some(node_id) = preferred_node_id {
            let ancestors = {
                let state = state.borrow();
                state.document.display_ancestor_node_ids(node_id)
            };
            state.borrow_mut().expanded_node_ids.extend(ancestors);
        }
        let state = state.borrow();
        (
            state.widgets.tree.clone(),
            tree_row_specs_for_order(
                &state.document,
                &state
                    .document
                    .visible_order(&state.expanded_node_ids, false),
                &state.expanded_node_ids,
                state.editing_node_id,
            ),
        )
    };

    let row_specs_have_same_order = {
        let state = state.borrow();
        tree_row_specs_have_same_order(&state.visible_tree_row_specs, &row_specs)
    };
    let row_specs = if row_specs_have_same_order {
        match refresh_tree_list_for_stable_order(state, &tree, row_specs, preferred_node_id) {
            Ok(()) => return Ok(()),
            Err(row_specs) => row_specs,
        }
    } else {
        row_specs
    };

    let row_specs =
        match refresh_tree_list_for_changed_order(state, &tree, row_specs, preferred_node_id) {
            Ok(()) => return Ok(()),
            Err(row_specs) => row_specs,
        };

    {
        let mut state = state.borrow_mut();
        state.suppress_tree_selection = true;
    }
    while let Some(child) = tree.first_child() {
        tree.remove(&child);
    }
    {
        let mut state = state.borrow_mut();
        state.visible_node_ids.clear();
        state.visible_node_ids.reserve(row_specs.len());
        state.visible_tree_row_specs.clear();
        state.visible_tree_row_specs.reserve(row_specs.len());
    }

    let mut focus_entry = None;
    let mut preferred_row = None;
    for (visible_index, spec) in row_specs.iter().enumerate() {
        {
            let mut state = state.borrow_mut();
            state.visible_node_ids.push(spec.node_id);
            state.visible_tree_row_specs.push(spec.clone());
        }
        let (row, row_focus_entry) = build_tree_row(state, spec);
        tree.append(&row);
        if row_focus_entry.is_some() {
            focus_entry = row_focus_entry;
        }

        if preferred_node_id == Some(spec.node_id) {
            preferred_row = Some(visible_index as i32);
        }
    }

    let selected_row = {
        let mut state = state.borrow_mut();
        let selected_row = preferred_row
            .or_else(|| {
                state
                    .selected_node_id
                    .and_then(|node_id| state.visible_node_ids.iter().position(|id| *id == node_id))
                    .map(|index| index as i32)
            })
            .or_else(|| (!state.visible_node_ids.is_empty()).then_some(0));
        state.selected_node_id = selected_row
            .and_then(|row| usize::try_from(row).ok())
            .and_then(|index| state.visible_node_ids.get(index).copied());
        selected_row
    };
    if let Some(row_index) = selected_row {
        if let Some(row) = tree.row_at_index(row_index) {
            tree.select_row(Some(&row));
        }
    } else {
        tree.unselect_all();
    }
    {
        let mut state = state.borrow_mut();
        state.suppress_tree_selection = false;
    }

    if let Some(entry) = focus_entry {
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    update_actions(state);
    Ok(())
}

fn refresh_visible_tree_row(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
    preferred_node_id: Option<i64>,
) -> Result<bool, AppError> {
    let (tree, row_index, row_index_i32, spec) = {
        let state = state.borrow();
        let Some(row_index) = state
            .visible_node_ids
            .iter()
            .position(|visible_node_id| *visible_node_id == node_id)
        else {
            return Ok(false);
        };
        let Some(row_index_i32) = i32::try_from(row_index).ok() else {
            return Ok(false);
        };
        let Some(spec) = tree_row_spec_for_node(
            &state.document,
            node_id,
            &state.expanded_node_ids,
            state.editing_node_id,
        ) else {
            return Ok(false);
        };
        (state.widgets.tree.clone(), row_index, row_index_i32, spec)
    };
    let Some(row) = tree.row_at_index(row_index_i32) else {
        return Ok(false);
    };

    state.borrow_mut().suppress_tree_selection = true;
    tree.remove(&row);
    let (row, focus_entry) = build_tree_row(state, &spec);
    tree.insert(&row, row_index_i32);
    {
        let mut state = state.borrow_mut();
        if let Some(visible_node_id) = state.visible_node_ids.get_mut(row_index) {
            *visible_node_id = spec.node_id;
        }
        if let Some(row_spec) = state.visible_tree_row_specs.get_mut(row_index) {
            *row_spec = spec.clone();
        }
    }
    select_preferred_visible_tree_row(state, preferred_node_id);
    state.borrow_mut().suppress_tree_selection = false;

    if let Some(entry) = focus_entry {
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    update_actions(state);
    Ok(true)
}

fn refresh_visible_tree_row_or_rebuild(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    if refresh_visible_tree_row(state, node_id, preferred_node_id)? {
        Ok(())
    } else {
        rebuild_tree_list(state, preferred_node_id)
    }
}

fn select_preferred_visible_tree_row(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
) {
    let (tree, selected_row) = {
        let mut state = state.borrow_mut();
        let selected_row = preferred_node_id
            .and_then(|node_id| state.visible_node_ids.iter().position(|id| *id == node_id))
            .or_else(|| {
                state
                    .selected_node_id
                    .and_then(|node_id| state.visible_node_ids.iter().position(|id| *id == node_id))
            })
            .or_else(|| (!state.visible_node_ids.is_empty()).then_some(0));
        state.selected_node_id =
            selected_row.and_then(|index| state.visible_node_ids.get(index).copied());
        (
            state.widgets.tree.clone(),
            selected_row.map(|index| index as i32),
        )
    };
    if let Some(row_index) = selected_row {
        if let Some(row) = tree.row_at_index(row_index) {
            tree.select_row(Some(&row));
        }
    } else {
        tree.unselect_all();
    }
}

fn select_preferred_visible_tree_row_suppressed(
    state: &Rc<RefCell<LinuxState>>,
    preferred_node_id: Option<i64>,
) {
    state.borrow_mut().suppress_tree_selection = true;
    select_preferred_visible_tree_row(state, preferred_node_id);
    state.borrow_mut().suppress_tree_selection = false;
}

fn set_tree_expander_label(row: &gtk::ListBoxRow, expanded: bool) {
    let Some(button) = row
        .child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
        .and_then(|container| container.first_child())
        .and_then(|child| child.downcast::<gtk::Button>().ok())
    else {
        return;
    };
    button.set_label(if expanded { "▾" } else { "▸" });
}

fn visible_descendant_count(visible_tree_row_specs: &[TreeRowSpec], parent_index: usize) -> usize {
    let Some(parent_depth) = visible_tree_row_specs
        .get(parent_index)
        .map(|spec| spec.depth)
    else {
        return 0;
    };

    visible_tree_row_specs
        .iter()
        .skip(parent_index.saturating_add(1))
        .take_while(|spec| spec.depth > parent_depth)
        .count()
}

fn refresh_tree_node_descendants(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let (tree, parent_index, remove_count, row_specs, expanded) = {
        let state_ref = state.borrow();
        let Some(parent_index) = state_ref
            .visible_node_ids
            .iter()
            .position(|visible_node_id| *visible_node_id == node_id)
        else {
            drop(state_ref);
            return rebuild_tree_list(state, preferred_node_id);
        };
        let root_depth = state_ref.document.display_depth(node_id);
        let order = state_ref.document.visible_descendant_order(
            node_id,
            root_depth,
            &state_ref.expanded_node_ids,
        );
        (
            state_ref.widgets.tree.clone(),
            parent_index,
            visible_descendant_count(&state_ref.visible_tree_row_specs, parent_index),
            tree_row_specs_for_order(
                &state_ref.document,
                &order,
                &state_ref.expanded_node_ids,
                state_ref.editing_node_id,
            ),
            state_ref.expanded_node_ids.contains(&node_id),
        )
    };

    state.borrow_mut().suppress_tree_selection = true;
    let first_child_index = parent_index.saturating_add(1);
    for _ in 0..remove_count {
        if let Some(row) = tree.row_at_index(first_child_index as i32) {
            tree.remove(&row);
        }
    }
    {
        let mut state = state.borrow_mut();
        let drain_end = first_child_index.saturating_add(remove_count);
        if first_child_index < drain_end && drain_end <= state.visible_node_ids.len() {
            state.visible_node_ids.drain(first_child_index..drain_end);
        }
        if first_child_index < drain_end && drain_end <= state.visible_tree_row_specs.len() {
            state
                .visible_tree_row_specs
                .drain(first_child_index..drain_end);
        }
    }

    let mut focus_entry = None;
    for (offset, spec) in row_specs.iter().enumerate() {
        let insert_index = first_child_index.saturating_add(offset);
        let (row, row_focus_entry) = build_tree_row(state, spec);
        tree.insert(&row, insert_index as i32);
        state
            .borrow_mut()
            .visible_node_ids
            .insert(insert_index, spec.node_id);
        state
            .borrow_mut()
            .visible_tree_row_specs
            .insert(insert_index, spec.clone());
        if row_focus_entry.is_some() {
            focus_entry = row_focus_entry;
        }
    }
    if let Some(row) = tree.row_at_index(parent_index as i32) {
        set_tree_expander_label(&row, expanded);
    }
    if let Some(parent_spec) = state
        .borrow_mut()
        .visible_tree_row_specs
        .get_mut(parent_index)
    {
        parent_spec.expanded = expanded;
    }
    select_preferred_visible_tree_row(state, preferred_node_id);
    state.borrow_mut().suppress_tree_selection = false;

    if let Some(entry) = focus_entry {
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    update_actions(state);
    Ok(())
}

fn set_tree_node_expanded(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
    expanded: bool,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let changed = {
        let mut state = state.borrow_mut();
        if expanded {
            state.expanded_node_ids.insert(node_id)
        } else {
            state.expanded_node_ids.remove(&node_id)
        }
    };
    if changed {
        refresh_tree_node_descendants(state, node_id, preferred_node_id)
    } else {
        select_preferred_visible_tree_row_suppressed(state, preferred_node_id);
        update_actions(state);
        Ok(())
    }
}

fn toggle_tree_node_expanded(
    state: &Rc<RefCell<LinuxState>>,
    node_id: i64,
) -> Result<(), AppError> {
    let (expanded, preferred_node_id) = {
        let mut state = state.borrow_mut();
        if state.expanded_node_ids.contains(&node_id) {
            state.selected_node_id = Some(node_id);
            (false, Some(node_id))
        } else {
            (true, state.selected_node_id.or(Some(node_id)))
        }
    };
    set_tree_node_expanded(state, node_id, expanded, preferred_node_id)
}

fn connect_tree_row_drag_drop(
    state: &Rc<RefCell<LinuxState>>,
    row: &gtk::ListBoxRow,
    node_id: i64,
) {
    if !tree_drag_drop_enabled(state) {
        return;
    }

    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    drag.connect_prepare(move |_, _, _| {
        Some(gdk::ContentProvider::for_value(
            &node_id.to_string().to_value(),
        ))
    });
    let drag_begin_state = Rc::clone(state);
    drag.connect_drag_begin(move |_, _| {
        drag_begin_state.borrow_mut().dragging_node_id = Some(node_id);
    });
    let drag_cancel_state = Rc::clone(state);
    drag.connect_drag_cancel(move |_, _, _| {
        drag_cancel_state.borrow_mut().dragging_node_id = None;
        false
    });
    let drag_end_state = Rc::clone(state);
    drag.connect_drag_end(move |_, _, _| {
        drag_end_state.borrow_mut().dragging_node_id = None;
    });
    row.add_controller(drag);

    let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    let state = Rc::clone(state);
    drop.connect_drop(move |_, value, _, _| {
        let dropped_text = value.get::<String>().ok();
        let (dragged_node_id, internal_drag_active) = {
            let state = state.borrow();
            let dragged_node_id = dropped_text
                .as_deref()
                .and_then(|text| accepted_internal_dragged_node(state.dragging_node_id, text));
            (dragged_node_id, state.dragging_node_id.is_some())
        };
        let Some(dragged_node_id) = dragged_node_id else {
            if internal_drag_active {
                state.borrow_mut().dragging_node_id = None;
            }
            return false;
        };
        let result = move_node_by_drop(&state, dragged_node_id, node_id);
        state.borrow_mut().dragging_node_id = None;
        let success = result.is_ok();
        run_and_report(&state, result);
        success
    });
    row.add_controller(drop);
}

fn tree_drag_drop_enabled(state: &Rc<RefCell<LinuxState>>) -> bool {
    let state = state.borrow();
    state.tree_mode == TreeMode::Active && state.search_query.trim().is_empty()
}

fn accepted_internal_dragged_node(
    dragging_node_id: Option<i64>,
    dropped_text: &str,
) -> Option<i64> {
    let dropped_node_id = dropped_text.parse::<i64>().ok()?;
    (dragging_node_id == Some(dropped_node_id)).then_some(dropped_node_id)
}

fn move_node_by_drop(
    state: &Rc<RefCell<LinuxState>>,
    dragged_node_id: i64,
    target_node_id: i64,
) -> Result<(), AppError> {
    ensure_active_tree_browse_mode(state)?;
    if dragged_node_id == target_node_id {
        return Ok(());
    }
    if !resolve_dirty_before_refresh(state)? {
        return Ok(());
    }
    state
        .borrow_mut()
        .app
        .move_node_to_parent_end(dragged_node_id, target_node_id)?;
    sync_tabs_from_active_document_local_metadata(state, true)?;
    reload_visible_document(state, Some(dragged_node_id))
}

fn show_tree_context_menu_at_position(
    state: &Rc<RefCell<LinuxState>>,
    x: f64,
    y: f64,
) -> Result<(), AppError> {
    remember_tree_context_menu_point(state, x, y);
    let node_id = tree_node_id_at_y(state, y);
    if let Some(node_id) = node_id {
        if !select_tree_node_with_navigation(state, node_id, false)? {
            update_actions(state);
            return Ok(());
        }
    }
    update_actions(state);
    let (parent, point) = {
        let state = state.borrow();
        let tree = state.widgets.tree.clone().upcast::<gtk::Widget>();
        let window = state.widgets.window.clone().upcast::<gtk::Widget>();
        let point = context_menu_point_in_window(&tree, &window, Some((x, y)));
        (window, point)
    };
    show_tree_context_menu(state, &parent, point);
    Ok(())
}

fn context_menu_click_gesture() -> gtk::GestureClick {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    gesture.set_exclusive(true);
    gesture
}

fn secondary_button_press_position(event: &gdk::Event) -> Option<(f64, f64)> {
    let button = event.downcast_ref::<gdk::ButtonEvent>()?;
    (event.event_type() == gdk::EventType::ButtonPress && button.button() == 3)
        .then(|| event.position())
        .flatten()
}

fn toggle_tree_row_at_y(state: &Rc<RefCell<LinuxState>>, y: f64) -> Result<(), AppError> {
    let Some(node_id) = tree_node_id_at_y(state, y) else {
        return Ok(());
    };
    if state.borrow().document.has_display_children(node_id) {
        toggle_tree_node_expanded(state, node_id)?;
    }
    Ok(())
}

fn tree_node_id_at_y(state: &Rc<RefCell<LinuxState>>, y: f64) -> Option<i64> {
    let state = state.borrow();
    let row = state.widgets.tree.row_at_y(y as i32)?;
    usize::try_from(row.index())
        .ok()
        .and_then(|index| state.visible_node_ids.get(index).copied())
}

fn show_tree_context_menu(
    state: &Rc<RefCell<LinuxState>>,
    parent: &gtk::Widget,
    point: Option<(f64, f64)>,
) {
    let language = state.borrow().app.ui_settings().language;
    let text = ui_text(language);
    show_context_menu(TREE_CONTEXT_MENU_ENTRIES, text, parent, point);
}

fn show_selected_tree_context_menu(state: &Rc<RefCell<LinuxState>>) {
    update_actions(state);
    let (parent, point): (gtk::Widget, Option<(f64, f64)>) = {
        let state = state.borrow();
        let tree = state.widgets.tree.clone().upcast::<gtk::Widget>();
        let window = state.widgets.window.clone().upcast::<gtk::Widget>();
        let point = state
            .last_tree_context_point
            .and_then(|point| context_menu_point_in_window(&tree, &window, Some(point.as_tuple())));
        (window, point)
    };
    show_tree_context_menu(state, &parent, point);
}

fn show_editor_context_menu(state: &Rc<RefCell<LinuxState>>, parent: &gtk::Widget) {
    parent.grab_focus();
    update_actions(state);
    let text = ui_text(state.borrow().app.ui_settings().language);
    let (window, point) = {
        let state = state.borrow();
        let window = state.widgets.window.clone().upcast::<gtk::Widget>();
        let point = active_editor_context_menu_point(
            state.tabs.active().map(|tab| tab.node_id),
            state.last_editor_context_point,
        )
        .and_then(|point| context_menu_point_in_window(parent, &window, Some(point)));
        (window, point)
    };
    show_context_menu(EDITOR_CONTEXT_MENU_ENTRIES, text, &window, point);
}

fn show_editor_context_menu_from_window_position(
    state: &Rc<RefCell<LinuxState>>,
    x: f64,
    y: f64,
) -> bool {
    let Some(page) = active_page_cloned(state) else {
        return false;
    };
    let (window, view) = {
        let state = state.borrow();
        (
            state.widgets.window.clone().upcast::<gtk::Widget>(),
            page.view.clone().upcast::<gtk::Widget>(),
        )
    };
    let Some((view_origin_x, view_origin_y)) =
        context_menu_point_in_widget(&view, &window, 0.0, 0.0)
    else {
        return false;
    };
    let view_x = x - view_origin_x;
    let view_y = y - view_origin_y;
    if !point_inside_widget(&view, view_x, view_y) {
        return false;
    }

    remember_editor_context_menu_point(state, &page.page, view_x, view_y);
    page.view.grab_focus();
    update_actions(state);
    let text = ui_text(state.borrow().app.ui_settings().language);
    show_context_menu(EDITOR_CONTEXT_MENU_ENTRIES, text, &window, Some((x, y)));
    true
}

fn remember_tree_context_menu_point(state: &Rc<RefCell<LinuxState>>, x: f64, y: f64) {
    if let Some(point) = context_menu_point_from_pointer(x, y) {
        state.borrow_mut().last_tree_context_point = Some(point);
    }
}

fn remember_editor_context_menu_point(
    state: &Rc<RefCell<LinuxState>>,
    page: &gtk::ScrolledWindow,
    x: f64,
    y: f64,
) {
    let Some(point) = context_menu_point_from_pointer(x, y) else {
        return;
    };
    let node_id = {
        let state = state.borrow();
        let page_index = state
            .widgets
            .notebook
            .page_num(page)
            .and_then(|index| usize::try_from(index).ok());
        page_index.and_then(|index| state.tabs.tabs().get(index).map(|tab| tab.node_id))
    };
    if let Some(node_id) = node_id {
        state.borrow_mut().last_editor_context_point =
            Some(EditorContextMenuPoint { node_id, point });
    }
}

fn active_editor_context_menu_point(
    active_node_id: Option<i64>,
    remembered: Option<EditorContextMenuPoint>,
) -> Option<(f64, f64)> {
    let active_node_id = active_node_id?;
    let remembered = remembered?;
    (remembered.node_id == active_node_id).then_some(remembered.point.as_tuple())
}

fn context_menu_point_in_window(
    source: &gtk::Widget,
    window: &gtk::Widget,
    point: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    let (x, y) = point?;
    context_menu_point_in_widget(source, window, x, y)
}

fn context_menu_point_in_widget(
    source: &gtk::Widget,
    target: &gtk::Widget,
    x: f64,
    y: f64,
) -> Option<(f64, f64)> {
    source.translate_coordinates(target, x, y)
}

fn point_inside_widget(widget: &gtk::Widget, x: f64, y: f64) -> bool {
    x >= 0.0 && y >= 0.0 && x < f64::from(widget.width()) && y < f64::from(widget.height())
}

fn context_menu_point_from_pointer(x: f64, y: f64) -> Option<ContextMenuPoint> {
    (x.is_finite() && y.is_finite()).then_some(ContextMenuPoint { x, y })
}

fn show_context_menu(
    entries: &[GuiMenuEntry],
    text: UiText,
    parent: &gtk::Widget,
    point: Option<(f64, f64)>,
) {
    let menu = build_menu_entries(entries, text);
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(parent);
    if let Some((x, y)) = point {
        let rect = gdk::Rectangle::new(pointing_coordinate(x), pointing_coordinate(y), 1, 1);
        popover.set_pointing_to(Some(&rect));
    }
    popover.popup();
}

fn pointing_coordinate(value: f64) -> i32 {
    if value.is_finite() {
        value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
    } else {
        0
    }
}

fn start_label_edit(state: &Rc<RefCell<LinuxState>>, node_id: i64) {
    state.borrow_mut().editing_node_id = Some(node_id);
    run_and_report(
        state,
        refresh_visible_tree_row_or_rebuild(state, node_id, Some(node_id)),
    );
}

fn commit_label_edit(state: &Rc<RefCell<LinuxState>>, title: &str) -> Result<(), AppError> {
    let node_id = {
        let mut state = state.borrow_mut();
        let Some(node_id) = state.editing_node_id.take() else {
            return Ok(());
        };
        node_id
    };
    let result = (|| -> Result<(), AppError> {
        if !resolve_dirty_before_refresh(state)? {
            refresh_visible_tree_row_or_rebuild(state, node_id, Some(node_id))?;
            return Ok(());
        }

        state.borrow_mut().app.rename_node(node_id, title)?;
        sync_tabs_from_active_document_local_metadata(state, true)?;
        if refresh_visible_document_after_stable_tree_row_rename(state, node_id)? {
            Ok(())
        } else {
            reload_visible_document(state, Some(node_id))
        }
    })();

    if let Err(error) = result {
        refresh_visible_tree_row_or_rebuild(state, node_id, Some(node_id))?;
        return Err(error);
    }

    Ok(())
}

fn ensure_active_editable_document_for_import(
    state: &Rc<RefCell<LinuxState>>,
) -> Result<(), AppError> {
    let state = state.borrow();
    match state.tabs.active() {
        Some(tab) if tab.editable => Ok(()),
        _ => Err(AppError::user(
            ui_text(state.app.ui_settings().language).open_import_document(),
        )),
    }
}

fn ensure_active_replace_target(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let state_ref = state.borrow();
    let text = ui_text(state_ref.app.ui_settings().language);
    let Some(tab) = state_ref.tabs.active() else {
        return Err(AppError::user(text.open_editable_document()));
    };
    if !tab.editable {
        return Err(AppError::user(text.read_only_find_replace()));
    }
    Ok(())
}

fn ensure_active_tree_browse_mode(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let state = state.borrow();
    if state.tree_mode != TreeMode::Active {
        return Err(AppError::user(
            ui_text(state.app.ui_settings().language).active_tree_only(),
        ));
    }
    if !state.search_query.trim().is_empty() {
        return Err(AppError::user(
            ui_text(state.app.ui_settings().language).search_not_allowed(),
        ));
    }
    Ok(())
}

fn ensure_trash_tree_mode(state: &Rc<RefCell<LinuxState>>) -> Result<(), AppError> {
    let state = state.borrow();
    if state.tree_mode == TreeMode::Trash {
        Ok(())
    } else {
        Err(AppError::user(
            ui_text(state.app.ui_settings().language).trash_only(),
        ))
    }
}

fn selected_node_id(state: &Rc<RefCell<LinuxState>>) -> Result<i64, AppError> {
    let state = state.borrow();
    selected_existing_node_id(&state.document, state.selected_node_id)
}

fn selected_existing_node_id(
    document: &UiDocument,
    selected_node_id: Option<i64>,
) -> Result<i64, AppError> {
    let node_id = selected_node_id.ok_or(DomainError::NodeNotFound { node_id: 0 })?;
    if document.node_by_id(node_id).is_some() {
        Ok(node_id)
    } else {
        Err(DomainError::NodeNotFound { node_id: 0 }.into())
    }
}

fn selected_node(state: &Rc<RefCell<LinuxState>>) -> Result<UiNode, AppError> {
    let node_id = selected_node_id(state)?;
    state
        .borrow()
        .document
        .node_by_id(node_id)
        .cloned()
        .ok_or_else(|| DomainError::NodeNotFound { node_id }.into())
}

fn selected_sibling_parent_id(state: &Rc<RefCell<LinuxState>>) -> Result<i64, AppError> {
    let node = selected_node(state)?;
    if node.id == ROOT_NODE_ID {
        return Ok(ROOT_NODE_ID);
    }
    node.parent_id
        .ok_or_else(|| DomainError::NodeNotFound { node_id: node.id }.into())
}

fn selected_child_parent_id(state: &Rc<RefCell<LinuxState>>) -> Result<i64, AppError> {
    selected_node(state).map(|node| node.id)
}

fn ensure_selected_node_can_be_deleted(
    node_id: i64,
    parent_id: Option<i64>,
) -> Result<(), AppError> {
    if node_id == ROOT_NODE_ID || parent_id.is_none() {
        Err(DomainError::CannotDeleteRoot.into())
    } else {
        Ok(())
    }
}

fn prepare_imported_text(content: &mut String, language: UiLanguage) -> Result<(), AppError> {
    let scan = scan_imported_text(content);
    if scan.contains_nul {
        return Err(AppError::user(ui_text(language).imported_text_nul()));
    }
    let Some(normalized_utf16_len) = scan.normalized_utf16_len else {
        return Err(AppError::platform(
            "prepare editor text",
            "editor text is too large",
        ));
    };
    if normalized_utf16_len > DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS {
        return Err(AppError::user(ui_text(language).imported_text_too_large()));
    }
    if scan.needs_normalization {
        *content = normalize_editor_plain_text(content);
    }
    Ok(())
}

struct ImportedTextScan {
    contains_nul: bool,
    needs_normalization: bool,
    normalized_utf16_len: Option<usize>,
}

fn scan_imported_text(value: &str) -> ImportedTextScan {
    let mut contains_nul = false;
    let mut needs_normalization = false;
    let mut normalized_utf16_len = Some(0usize);
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\0' {
            contains_nul = true;
        }
        let units = match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                } else {
                    needs_normalization = true;
                }
                2
            }
            '\n' => {
                needs_normalization = true;
                2
            }
            _ => ch.len_utf16(),
        };
        if let Some(len) = normalized_utf16_len {
            normalized_utf16_len = len.checked_add(units);
        }
    }
    ImportedTextScan {
        contains_nul,
        needs_normalization,
        normalized_utf16_len,
    }
}

#[cfg(test)]
fn document_editor_text_len_utf16(value: &str) -> Result<usize, AppError> {
    value.chars().try_fold(0usize, |len, ch| {
        len.checked_add(ch.len_utf16())
            .ok_or_else(|| AppError::platform("prepare editor text", "editor text is too large"))
    })
}

#[cfg(test)]
fn utf16_limit_truncation_byte_index(value: &str, limit: usize) -> Option<usize> {
    let mut len = 0usize;
    for (index, ch) in value.char_indices() {
        let Some(next_len) = len.checked_add(ch.len_utf16()) else {
            return Some(index);
        };
        if next_len > limit {
            return Some(index);
        }
        len = next_len;
    }
    None
}

fn normalize_editor_plain_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                output.push_str("\r\n");
            }
            '\n' => output.push_str("\r\n"),
            _ => output.push(ch),
        }
    }
    output
}

fn apply_theme(state: &Rc<RefCell<LinuxState>>) -> Result<bool, AppError> {
    let (provider, window, theme, requested_font) = {
        let state = state.borrow();
        (
            state.widgets.css_provider.clone(),
            state.widgets.window.clone(),
            state.app.ui_settings().appearance.theme,
            state.app.ui_settings().editor_font,
        )
    };
    let resolved_font = resolve_editor_font_settings(&requested_font, |family| {
        editor_font_family_available(&window, family)
    });
    if resolved_font.settings != requested_font {
        let mut state = state.borrow_mut();
        let mut settings = state.app.ui_settings();
        if settings.editor_font == requested_font {
            settings.editor_font = resolved_font.settings.clone();
            state.app.save_ui_settings(settings)?;
        }
    }
    for option in AppearanceTheme::options() {
        window.remove_css_class(&theme_css_class(*option));
    }
    window.add_css_class(&theme_css_class(theme));
    provider.load_from_data(&theme_css(theme, &resolved_font.settings));
    invalidate_all_caret_status_caches(state);
    refresh_find_match_highlight_tags(state, theme);
    Ok(resolved_font.used_fallback)
}

fn save_ui_settings_with_resolved_editor_font(
    state: &Rc<RefCell<LinuxState>>,
    settings: UiSettings,
) -> Result<bool, AppError> {
    let window = state.borrow().widgets.window.clone();
    let (settings, used_fallback) = ui_settings_with_resolved_editor_font(settings, |family| {
        editor_font_family_available(&window, family)
    });
    state.borrow_mut().app.save_ui_settings(settings)?;
    Ok(used_fallback)
}

fn ui_settings_with_resolved_editor_font(
    mut settings: UiSettings,
    family_available: impl Fn(&str) -> bool,
) -> (UiSettings, bool) {
    let resolved_font = resolve_editor_font_settings(&settings.editor_font, family_available);
    settings.editor_font = resolved_font.settings;
    (settings, resolved_font.used_fallback)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedEditorFontSettings {
    settings: EditorFontSettings,
    used_fallback: bool,
}

fn resolve_editor_font_settings(
    requested: &EditorFontSettings,
    family_available: impl Fn(&str) -> bool,
) -> ResolvedEditorFontSettings {
    let default_settings = EditorFontSettings::default();
    let settings = if family_available(&requested.family) {
        requested.clone()
    } else {
        default_settings
    };
    let used_fallback = settings != *requested;
    ResolvedEditorFontSettings {
        settings,
        used_fallback,
    }
}

fn editor_font_family_available(widget: &gtk::ApplicationWindow, family: &str) -> bool {
    widget
        .pango_context()
        .list_families()
        .iter()
        .any(|available| font_family_names_match(available.name().as_str(), family))
}

fn font_family_names_match(left: &str, right: &str) -> bool {
    left == right || left.to_lowercase() == right.to_lowercase()
}

fn theme_css_class(theme: AppearanceTheme) -> String {
    format!("{THEME_CSS_CLASS_PREFIX}{}", theme.storage_value())
}

fn theme_css(theme: AppearanceTheme, font: &EditorFontSettings) -> String {
    let palette = ThemePalette::for_theme(theme);
    format!(
        r#"
.j3-window {{
  background: {bg};
  color: {fg};
}}
.j3-window .j3-root,
.j3-window .j3-left-pane,
.j3-window .j3-right-pane,
.j3-window .j3-menu-bar,
.j3-window .j3-tree-scroller,
.j3-window .j3-tree,
.j3-window .j3-tabs,
.j3-window .j3-tabs header,
.j3-window .j3-tabs tabs,
.j3-window .j3-caret-status {{
  background: {bg};
  color: {fg};
}}
.j3-window .j3-compact-titlebar,
.j3-window .j3-titlebar-layout {{
  background: {bg};
  color: {fg};
  min-height: 22px;
  padding-top: 0;
  padding-bottom: 0;
}}
.j3-window .j3-titlebar-start {{
  min-width: 42px;
  min-height: 22px;
}}
.j3-window .j3-titlebar-icon {{
  min-width: 12px;
  min-height: 12px;
  padding: 0;
  margin: 0;
}}
.j3-window .j3-titlebar-controls {{
  min-height: 22px;
  padding: 0;
  margin: 0;
}}
.j3-window .j3-titlebar-controls button {{
  min-width: 20px;
  min-height: 20px;
  padding: 0;
  margin: 0;
}}
.j3-window .j3-titlebar-controls image {{
  -gtk-icon-size: 12px;
  min-width: 12px;
  min-height: 12px;
}}
.j3-window .j3-titlebar-label {{
  color: {fg};
  min-height: 20px;
  padding-top: 0;
  padding-bottom: 0;
}}
.j3-window .j3-menu-bar menubutton,
.j3-window .j3-menu-bar menubutton > button,
.j3-window .j3-menu-bar button {{
  background: transparent;
  color: {fg};
  box-shadow: none;
}}
.j3-window .j3-menu-bar label {{
  color: {fg};
}}
.j3-window .j3-menu-bar menubutton:hover,
.j3-window .j3-menu-bar menubutton:hover > button,
.j3-window .j3-menu-bar button:hover {{
  background: {hover};
  color: {fg};
}}
.j3-window popover,
.j3-window popover contents,
.j3-window menu {{
  background: {panel};
  color: {fg};
}}
.j3-window popover modelbutton,
.j3-window popover button,
.j3-window menuitem {{
  background: transparent;
  color: {fg};
}}
.j3-window popover label,
.j3-window menuitem label {{
  color: {fg};
}}
.j3-window popover modelbutton:hover,
.j3-window popover button:hover,
.j3-window menuitem:hover {{
  background: {hover};
  color: {fg};
}}
.j3-window .j3-search,
.j3-window .j3-search text,
.j3-window .j3-tree-entry,
.j3-window .j3-tree-entry text {{
  background: {panel};
  color: {fg};
}}
.j3-window .j3-search image {{
  color: {fg};
}}
.j3-window .j3-search:focus,
.j3-window .j3-tree-entry:focus {{
  border-color: {accent};
}}
.j3-tree row {{
  background: {panel};
  color: {fg};
}}
.j3-tree row:hover {{
  background: {hover};
  color: {fg};
}}
.j3-tree row:selected {{
  background: {accent};
  color: {accent_fg};
}}
.j3-tree row:selected label,
.j3-tree row:selected button {{
  color: {accent_fg};
}}
.j3-tree-expander {{
  background: transparent;
  color: {fg};
  box-shadow: none;
}}
.j3-tree-expander:hover {{
  background: {hover};
  color: {fg};
}}
.j3-tree-scroller viewport,
.j3-editor-scroller viewport {{
  background: {panel};
  color: {fg};
}}
.j3-editor-scroller {{
  background: {editor_bg};
  color: {fg};
}}
.j3-editor-scroller viewport {{
  background: {editor_bg};
}}
.j3-editor {{
  font-family: "{font_family}";
  font-size: {font_size}pt;
  color: {fg};
}}
.j3-editor text {{
  background: {editor_bg};
  color: {fg};
}}
.j3-tabs stack {{
  background: {editor_bg};
}}
.j3-tabs tab:checked {{
  background: {panel};
  color: {fg};
}}
.j3-tabs tab {{
  background: {bg};
  color: {fg};
  border-color: {border};
}}
.j3-tabs tab {{
  min-height: {tab_bar_height}px;
  padding-top: 0;
  padding-bottom: 0;
}}
.j3-tab-label {{
  min-height: {tab_bar_height}px;
  color: {fg};
}}
.j3-tree-expander {{
  min-width: {tree_expander_hit_size}px;
  min-height: {tree_expander_hit_size}px;
  padding: 0;
}}
.j3-tab-close {{
  background: transparent;
  color: {fg};
  min-width: {tab_close_width}px;
  min-height: 20px;
  padding: 0;
}}
.j3-tab-close:hover {{
  background: {hover};
  color: {fg};
}}
"#,
        bg = palette.bg,
        panel = palette.panel,
        editor_bg = palette.editor_bg,
        fg = palette.fg,
        accent = palette.accent,
        accent_fg = palette.accent_fg,
        hover = palette.hover,
        border = palette.border,
        font_family = css_escape_string(&font.family),
        font_size = font.size_pt,
        tab_bar_height = TAB_BAR_HEIGHT_PX,
        tab_close_width = TAB_CLOSE_HIT_WIDTH_PX,
        tree_expander_hit_size = TREE_EXPANDER_HIT_SIZE_PX,
    )
}

fn css_escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

struct ThemePalette {
    bg: &'static str,
    panel: &'static str,
    editor_bg: &'static str,
    fg: &'static str,
    accent: &'static str,
    accent_fg: &'static str,
    hover: &'static str,
    border: &'static str,
    find_match_bg: &'static str,
}

impl ThemePalette {
    fn for_theme(theme: AppearanceTheme) -> Self {
        match theme {
            AppearanceTheme::Light => Self {
                bg: "#f0f0f0",
                panel: "#ffffff",
                editor_bg: "#ffffff",
                fg: "#000000",
                accent: "#0078d7",
                accent_fg: "#ffffff",
                hover: "#e5f1fb",
                border: "#c8c8c8",
                find_match_bg: "#ffe680",
            },
            AppearanceTheme::ClassicDark => Self {
                bg: "#1f2124",
                panel: "#181a1d",
                editor_bg: "#181a1d",
                fg: "#e6e8eb",
                accent: "#5c6169",
                accent_fg: "#e6e8eb",
                hover: "#2f3338",
                border: "#4f565f",
                find_match_bg: "#5b4a19",
            },
            AppearanceTheme::SepiaTeal => Self {
                bg: "#181918",
                panel: "#1f3438",
                editor_bg: "#1f3438",
                fg: "#ece8db",
                accent: "#b29a7c",
                accent_fg: "#181918",
                hover: "#2d474b",
                border: "#6a7774",
                find_match_bg: "#53521f",
            },
            AppearanceTheme::Graphite => Self {
                bg: "#18191a",
                panel: "#32373f",
                editor_bg: "#32373f",
                fg: "#efece5",
                accent: "#7e7769",
                accent_fg: "#18191a",
                hover: "#3c424c",
                border: "#6c7078",
                find_match_bg: "#59501f",
            },
            AppearanceTheme::Forest => Self {
                bg: "#161917",
                panel: "#273b3f",
                editor_bg: "#273b3f",
                fg: "#ecefe5",
                accent: "#689675",
                accent_fg: "#ffffff",
                hover: "#304b4f",
                border: "#5d716a",
                find_match_bg: "#3e582a",
            },
            AppearanceTheme::SteelBlue => Self {
                bg: "#18191b",
                panel: "#364050",
                editor_bg: "#364050",
                fg: "#eff0f2",
                accent: "#688bab",
                accent_fg: "#ffffff",
                hover: "#435169",
                border: "#70809a",
                find_match_bg: "#425871",
            },
        }
    }
}

fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn gtk_tab_view_state_checked(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    content: &str,
) -> Result<crate::domain::DocumentTabViewState, ()> {
    let selection = buffer.selection_bounds();
    let caret = buffer.cursor_position();
    let offsets = scan_gtk_tab_view_state_utf16_offsets(
        content,
        caret,
        selection
            .as_ref()
            .map(|(start, end)| (start.offset(), end.offset())),
        Some(DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS),
    );
    if offsets.utf16_limit_truncation_byte_index.is_some() {
        return Err(());
    }
    Ok(crate::domain::DocumentTabViewState {
        first_visible_line: gtk_first_visible_line(view),
        caret_position_utf16: offsets.caret_position_utf16,
        selection_start_utf16: offsets.selection_start_utf16,
        selection_end_utf16: offsets.selection_end_utf16,
    })
}

fn gtk_tab_view_state(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    content: &str,
) -> crate::domain::DocumentTabViewState {
    let selection = buffer.selection_bounds();
    let caret = buffer.cursor_position();
    let offsets = scan_gtk_tab_view_state_utf16_offsets(
        content,
        caret,
        selection
            .as_ref()
            .map(|(start, end)| (start.offset(), end.offset())),
        None,
    );
    crate::domain::DocumentTabViewState {
        first_visible_line: gtk_first_visible_line(view),
        caret_position_utf16: offsets.caret_position_utf16,
        selection_start_utf16: offsets.selection_start_utf16,
        selection_end_utf16: offsets.selection_end_utf16,
    }
}

struct GtkTabViewStateUtf16Offsets {
    caret_position_utf16: usize,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
    utf16_limit_truncation_byte_index: Option<usize>,
}

fn scan_gtk_tab_view_state_utf16_offsets(
    content: &str,
    caret: i32,
    selection: Option<(i32, i32)>,
    utf16_limit: Option<usize>,
) -> GtkTabViewStateUtf16Offsets {
    let caret_target = text_iter_char_offset_target(caret);
    let (selection_start_target, selection_end_target) = selection
        .map(|(start, end)| {
            (
                text_iter_char_offset_target(start),
                text_iter_char_offset_target(end),
            )
        })
        .unwrap_or((caret_target, caret_target));
    let mut caret_position_utf16 = (caret_target == 0).then_some(0usize);
    let mut selection_start_utf16 = (selection_start_target == 0).then_some(0usize);
    let mut selection_end_utf16 = (selection_end_target == 0).then_some(0usize);
    let mut char_offset = 0usize;
    let mut utf16_offset = 0usize;

    for (byte_index, ch) in content.char_indices() {
        if caret_position_utf16.is_none() && char_offset == caret_target {
            caret_position_utf16 = Some(utf16_offset);
        }
        if selection_start_utf16.is_none() && char_offset == selection_start_target {
            selection_start_utf16 = Some(utf16_offset);
        }
        if selection_end_utf16.is_none() && char_offset == selection_end_target {
            selection_end_utf16 = Some(utf16_offset);
        }
        if caret_position_utf16.is_some()
            && selection_start_utf16.is_some()
            && selection_end_utf16.is_some()
            && utf16_limit.is_none()
        {
            break;
        }

        let next_utf16_offset = utf16_offset.saturating_add(ch.len_utf16());
        if utf16_limit.is_some_and(|limit| next_utf16_offset > limit) {
            return GtkTabViewStateUtf16Offsets {
                caret_position_utf16: caret_position_utf16.unwrap_or(utf16_offset),
                selection_start_utf16: selection_start_utf16.unwrap_or(utf16_offset),
                selection_end_utf16: selection_end_utf16.unwrap_or(utf16_offset),
                utf16_limit_truncation_byte_index: Some(byte_index),
            };
        }
        utf16_offset = next_utf16_offset;
        char_offset = char_offset.saturating_add(1);
    }

    GtkTabViewStateUtf16Offsets {
        caret_position_utf16: caret_position_utf16.unwrap_or(utf16_offset),
        selection_start_utf16: selection_start_utf16.unwrap_or(utf16_offset),
        selection_end_utf16: selection_end_utf16.unwrap_or(utf16_offset),
        utf16_limit_truncation_byte_index: None,
    }
}

fn text_iter_char_offset_target(char_offset: i32) -> usize {
    usize::try_from(char_offset.max(0)).unwrap_or(0)
}

fn gtk_first_visible_line(view: &gtk::TextView) -> usize {
    let rect = view.visible_rect();
    view.iter_at_location(rect.x(), rect.y())
        .and_then(|iter| usize::try_from(iter.line()).ok())
        .unwrap_or(0)
}

fn select_find_result_byte_range(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    content: &str,
    start: usize,
    end: usize,
    theme: AppearanceTheme,
) {
    let (start_offset, end_offset) = char_offsets_for_byte_range(content, start, end);
    select_find_result_offsets(view, buffer, start_offset, end_offset, theme);
}

fn select_find_result_offsets(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    start_offset: i32,
    end_offset: i32,
    theme: AppearanceTheme,
) {
    clear_find_match_highlight(buffer);
    let mut start = buffer.iter_at_offset(start_offset);
    let end = buffer.iter_at_offset(end_offset);
    buffer.select_range(&start, &end);
    if start_offset < end_offset {
        let tag = find_match_highlight_tag(buffer, theme);
        buffer.apply_tag(&tag, &start, &end);
    }
    view.scroll_to_iter(&mut start, 0.0, false, 0.0, 0.0);
}

fn clear_find_match_highlight(buffer: &gtk::TextBuffer) {
    let Some(tag) = buffer.tag_table().lookup(FIND_MATCH_HIGHLIGHT_TAG) else {
        return;
    };
    let (start, end) = buffer.bounds();
    buffer.remove_tag(&tag, &start, &end);
}

fn clear_active_find_match_highlight(state: &Rc<RefCell<LinuxState>>) {
    if let Some(page) = active_page_cloned(state) {
        clear_find_match_highlight(&page.buffer);
    }
}

fn active_find_match_selection_byte_range(
    state: &Rc<RefCell<LinuxState>>,
) -> Option<(usize, usize)> {
    let state = state.borrow();
    let page = active_page(&state)?;
    let content = state.tabs.active()?.content.as_str();
    find_match_selection_byte_range(&page.buffer, content)
}

fn find_match_selection_byte_range(
    buffer: &gtk::TextBuffer,
    content: &str,
) -> Option<(usize, usize)> {
    let tag = buffer.tag_table().lookup(FIND_MATCH_HIGHLIGHT_TAG)?;
    let (start, end) = buffer.selection_bounds()?;
    if start.offset() >= end.offset() || !selection_has_find_match_tag(&start, &end, &tag) {
        return None;
    }
    Some((
        byte_index_for_char_offset(content, start.offset()),
        byte_index_for_char_offset(content, end.offset()),
    ))
}

fn selection_has_find_match_tag(
    start: &gtk::TextIter,
    end: &gtk::TextIter,
    tag: &gtk::TextTag,
) -> bool {
    if start.has_tag(tag)
        || start.starts_tag(Some(tag))
        || end.has_tag(tag)
        || end.ends_tag(Some(tag))
    {
        return true;
    }
    let mut probe = *start;
    probe.forward_char() && probe.offset() <= end.offset() && probe.has_tag(tag)
}

fn restore_active_find_match_highlight(
    state: &Rc<RefCell<LinuxState>>,
    range: Option<(usize, usize)>,
) {
    let Some((start, end)) = range else {
        return;
    };
    let (page, content, theme) = {
        let state = state.borrow();
        let Some(page) = active_page(&state) else {
            return;
        };
        let Some(tab) = state.tabs.active() else {
            return;
        };
        (
            page.clone(),
            tab.content.clone(),
            state.app.ui_settings().appearance.theme,
        )
    };
    if start <= end
        && end <= content.len()
        && content.is_char_boundary(start)
        && content.is_char_boundary(end)
    {
        select_find_result_byte_range(&page.view, &page.buffer, &content, start, end, theme);
    }
}

fn find_match_highlight_tag(buffer: &gtk::TextBuffer, theme: AppearanceTheme) -> gtk::TextTag {
    let background = ThemePalette::for_theme(theme).find_match_bg;
    let table = buffer.tag_table();
    if let Some(tag) = table.lookup(FIND_MATCH_HIGHLIGHT_TAG) {
        configure_find_match_highlight_tag(&tag, background);
        return tag;
    }

    buffer
        .create_tag(
            Some(FIND_MATCH_HIGHLIGHT_TAG),
            &[
                ("background", &background),
                ("background-full-height", &true),
            ],
        )
        .unwrap_or_else(|| {
            let tag = table
                .lookup(FIND_MATCH_HIGHLIGHT_TAG)
                .expect("find match tag should exist after create_tag collision");
            configure_find_match_highlight_tag(&tag, background);
            tag
        })
}

fn refresh_find_match_highlight_tags(state: &Rc<RefCell<LinuxState>>, theme: AppearanceTheme) {
    let background = ThemePalette::for_theme(theme).find_match_bg;
    let tab_pages = state.borrow().tab_pages.clone();
    for page in tab_pages {
        if let Some(tag) = page.buffer.tag_table().lookup(FIND_MATCH_HIGHLIGHT_TAG) {
            configure_find_match_highlight_tag(&tag, background);
        }
    }
}

fn configure_find_match_highlight_tag(tag: &gtk::TextTag, background: &str) {
    tag.set_background(Some(background));
    tag.set_background_full_height(true);
}

fn replace_buffer_char_range(
    state: &Rc<RefCell<LinuxState>>,
    buffer: &gtk::TextBuffer,
    start_offset: i32,
    end_offset: i32,
    replacement: &str,
) {
    let mut start = buffer.iter_at_offset(start_offset);
    let mut end = buffer.iter_at_offset(end_offset);
    state.borrow_mut().suppress_editor_change = true;
    buffer.begin_user_action();
    buffer.delete(&mut start, &mut end);
    if !replacement.is_empty() {
        buffer.insert(&mut start, replacement);
    }
    buffer.end_user_action();
    state.borrow_mut().suppress_editor_change = false;
}

fn current_buffer_selection_byte_range(buffer: &gtk::TextBuffer, content: &str) -> TextMatch {
    let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
        let cursor = buffer.iter_at_offset(buffer.cursor_position());
        (cursor, cursor)
    });
    TextMatch {
        start: byte_index_for_char_offset(content, start.offset()),
        end: byte_index_for_char_offset(content, end.offset()),
    }
}

fn restore_buffer_view_state(
    view: &gtk::TextView,
    buffer: &gtk::TextBuffer,
    content: &str,
    view_state: crate::domain::DocumentTabViewState,
) {
    let selection_offsets = scan_restored_selection_char_offsets(
        content,
        view_state.selection_start_utf16,
        view_state.selection_end_utf16,
    );
    let max_first_visible_line =
        usize::try_from(buffer.line_count().saturating_sub(1)).unwrap_or(0);
    let view_state = view_state.clamped(
        selection_offsets.max_text_offset_utf16,
        max_first_visible_line,
    );
    let (start_offset, end_offset) = if restored_selection_offsets_match_clamped_state(
        &selection_offsets,
        view_state.selection_start_utf16,
        view_state.selection_end_utf16,
    ) {
        (
            selection_offsets.selection_start_char_offset,
            selection_offsets.selection_end_char_offset,
        )
    } else {
        let clamped_offsets = scan_restored_selection_char_offsets(
            content,
            view_state.selection_start_utf16,
            view_state.selection_end_utf16,
        );
        (
            clamped_offsets.selection_start_char_offset,
            clamped_offsets.selection_end_char_offset,
        )
    };
    let start = buffer.iter_at_offset(start_offset);
    let end = buffer.iter_at_offset(end_offset);
    buffer.select_range(&start, &end);
    restore_first_visible_line(view, buffer, view_state.first_visible_line);
}

struct RestoredSelectionCharOffsets {
    requested_selection_start_utf16: usize,
    requested_selection_end_utf16: usize,
    max_text_offset_utf16: usize,
    selection_start_char_offset: i32,
    selection_end_char_offset: i32,
}

fn scan_restored_selection_char_offsets(
    content: &str,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
) -> RestoredSelectionCharOffsets {
    let mut selection_start_char_offset = (selection_start_utf16 == 0).then_some(0usize);
    let mut selection_end_char_offset = (selection_end_utf16 == 0).then_some(0usize);
    let mut char_offset = 0usize;
    let mut utf16_offset = 0usize;

    for ch in content.chars() {
        let next_utf16_offset = utf16_offset.saturating_add(ch.len_utf16());
        if selection_start_char_offset.is_none()
            && selection_start_utf16 >= utf16_offset
            && selection_start_utf16 < next_utf16_offset
        {
            selection_start_char_offset = Some(char_offset);
        } else if selection_start_char_offset.is_none()
            && selection_start_utf16 == next_utf16_offset
        {
            selection_start_char_offset = Some(char_offset.saturating_add(1));
        }
        if selection_end_char_offset.is_none()
            && selection_end_utf16 >= utf16_offset
            && selection_end_utf16 < next_utf16_offset
        {
            selection_end_char_offset = Some(char_offset);
        } else if selection_end_char_offset.is_none() && selection_end_utf16 == next_utf16_offset {
            selection_end_char_offset = Some(char_offset.saturating_add(1));
        }
        utf16_offset = next_utf16_offset;
        char_offset = char_offset.saturating_add(1);
    }

    let max_char_offset = char_offset;
    RestoredSelectionCharOffsets {
        requested_selection_start_utf16: selection_start_utf16,
        requested_selection_end_utf16: selection_end_utf16,
        max_text_offset_utf16: utf16_offset,
        selection_start_char_offset: selection_start_char_offset
            .unwrap_or(max_char_offset)
            .min(i32::MAX as usize) as i32,
        selection_end_char_offset: selection_end_char_offset
            .unwrap_or(max_char_offset)
            .min(i32::MAX as usize) as i32,
    }
}

fn restored_selection_offsets_match_clamped_state(
    offsets: &RestoredSelectionCharOffsets,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
) -> bool {
    restored_selection_offset_matches_clamped_offset(
        offsets.requested_selection_start_utf16,
        selection_start_utf16,
        offsets.max_text_offset_utf16,
    ) && restored_selection_offset_matches_clamped_offset(
        offsets.requested_selection_end_utf16,
        selection_end_utf16,
        offsets.max_text_offset_utf16,
    )
}

fn restored_selection_offset_matches_clamped_offset(
    requested_utf16: usize,
    clamped_utf16: usize,
    max_text_offset_utf16: usize,
) -> bool {
    requested_utf16 == clamped_utf16
        || (requested_utf16 >= max_text_offset_utf16 && clamped_utf16 == max_text_offset_utf16)
}

fn restore_first_visible_line(view: &gtk::TextView, buffer: &gtk::TextBuffer, line: usize) {
    let view = view.clone();
    let buffer = buffer.clone();
    let line = line.min(i32::MAX as usize) as i32;
    glib::idle_add_local_once(move || {
        if let Some(mut iter) = buffer.iter_at_line(line) {
            view.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.0);
        }
    });
}

fn char_offset_for_byte_index(content: &str, byte_index: usize) -> i32 {
    char_offsets_for_byte_range(content, byte_index, byte_index).0
}

fn char_offsets_for_byte_range(content: &str, start: usize, end: usize) -> (i32, i32) {
    let start = start.min(content.len());
    let end = end.min(content.len());
    let mut char_offset = 0usize;
    let mut start_offset = (start == 0).then_some(0usize);
    let mut end_offset = (end == 0).then_some(0usize);

    for (byte_index, _) in content.char_indices() {
        if start_offset.is_none() && byte_index == start {
            start_offset = Some(char_offset);
        }
        if end_offset.is_none() && byte_index == end {
            end_offset = Some(char_offset);
        }
        if start_offset.is_some() && end_offset.is_some() {
            break;
        }
        char_offset = char_offset.saturating_add(1);
    }

    let total_chars = char_offset;
    (
        start_offset.unwrap_or(total_chars).min(i32::MAX as usize) as i32,
        end_offset.unwrap_or(total_chars).min(i32::MAX as usize) as i32,
    )
}

fn byte_index_for_char_offset(content: &str, char_offset: i32) -> usize {
    let target = usize::try_from(char_offset.max(0)).unwrap_or(0);
    content
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(content.len()))
        .nth(target)
        .unwrap_or(content.len())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FindReplaceDialogKind {
    Find,
    Replace,
}

#[derive(Clone)]
struct FindReplaceDialogState {
    id: u64,
    kind: FindReplaceDialogKind,
    dialog: gtk::Dialog,
}

enum ExistingFindReplaceDialogAction {
    Focus(gtk::Dialog),
    Close(gtk::Dialog),
}

fn open_find_replace_dialog(
    state: &Rc<RefCell<LinuxState>>,
    kind: FindReplaceDialogKind,
) -> Result<(), AppError> {
    ensure_active_replace_target(state)?;
    sync_active_editor_content(state)?;
    if let Some(action) = existing_find_replace_dialog_action(state, kind) {
        match action {
            ExistingFindReplaceDialogAction::Focus(dialog) => {
                dialog.present();
                return Ok(());
            }
            ExistingFindReplaceDialogAction::Close(dialog) => {
                clear_active_find_match_highlight(state);
                dialog.close();
            }
        }
    }

    let (window, language) = {
        let state = state.borrow();
        (
            state.widgets.window.clone(),
            state.app.ui_settings().language,
        )
    };
    let text = ui_text(language);
    let dialog = gtk::Dialog::builder()
        .title(match kind {
            FindReplaceDialogKind::Find => strip_menu_accelerator(text.find_text()),
            FindReplaceDialogKind::Replace => strip_menu_accelerator(text.replace_text()),
        })
        .transient_for(&window)
        .modal(false)
        .destroy_with_parent(true)
        .build();

    let dialog_id = register_find_replace_dialog(state, kind, &dialog);
    let content = dialog.content_area();
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let find_entry = gtk::Entry::new();
    let replace_entry = gtk::Entry::new();
    install_find_replace_entry_text_limit(&find_entry);
    install_find_replace_entry_text_limit(&replace_entry);
    connect_find_replace_entry_default_response(&find_entry, &dialog);
    connect_find_replace_entry_default_response(&replace_entry, &dialog);
    grid.attach(&gtk::Label::new(Some(text.find_label())), 0, 0, 1, 1);
    grid.attach(&find_entry, 1, 0, 1, 1);
    if kind == FindReplaceDialogKind::Replace {
        grid.attach(&gtk::Label::new(Some(text.replace_label())), 0, 1, 1, 1);
        grid.attach(&replace_entry, 1, 1, 1, 1);
    }
    content.append(&grid);

    dialog.add_button(text.close_button(), gtk::ResponseType::Close);
    let find_next_button = dialog.add_button(text.find_next_button(), gtk::ResponseType::Other(1));
    find_next_button.set_receives_default(true);
    dialog.set_default_widget(Some(&find_next_button));
    dialog.set_default_response(gtk::ResponseType::Other(1));
    if kind == FindReplaceDialogKind::Replace {
        dialog.add_button(text.replace_button(), gtk::ResponseType::Other(2));
        dialog.add_button(text.replace_all_button(), gtk::ResponseType::Other(3));
    }

    let state_for_close = Rc::clone(state);
    dialog.connect_close_request(move |_| {
        clear_find_replace_dialog(&state_for_close, dialog_id);
        glib::Propagation::Proceed
    });

    let state_clone = Rc::clone(state);
    let find_entry_for_response = find_entry.clone();
    let replace_entry_for_response = replace_entry.clone();
    dialog.connect_response(move |dialog, response| match response {
        gtk::ResponseType::Close | gtk::ResponseType::DeleteEvent => {
            clear_find_replace_dialog(&state_clone, dialog_id);
            dialog.close();
        }
        gtk::ResponseType::Other(1) => run_and_report(
            &state_clone,
            find_next_in_active_editor(&state_clone, find_entry_for_response.text().as_str()),
        ),
        gtk::ResponseType::Other(2) => run_and_report(
            &state_clone,
            replace_one_in_active_editor(
                &state_clone,
                find_entry_for_response.text().as_str(),
                replace_entry_for_response.text().as_str(),
            ),
        ),
        gtk::ResponseType::Other(3) => run_and_report(
            &state_clone,
            replace_all_in_active_editor(
                &state_clone,
                find_entry_for_response.text().as_str(),
                replace_entry_for_response.text().as_str(),
            ),
        ),
        _ => {}
    });
    dialog.present();
    find_entry.grab_focus();
    Ok(())
}

fn connect_find_replace_entry_default_response(entry: &gtk::Entry, dialog: &gtk::Dialog) {
    let dialog = dialog.clone();
    entry.connect_activate(move |_| {
        dialog.response(gtk::ResponseType::Other(1));
    });
}

fn existing_find_replace_dialog_action(
    state: &Rc<RefCell<LinuxState>>,
    kind: FindReplaceDialogKind,
) -> Option<ExistingFindReplaceDialogAction> {
    let mut state = state.borrow_mut();
    let active = state.find_replace_dialog.as_ref()?;

    if !active.dialog.is_visible() {
        state.find_replace_dialog = None;
        return None;
    }

    if active.kind == kind {
        return Some(ExistingFindReplaceDialogAction::Focus(
            active.dialog.clone(),
        ));
    }

    let dialog = active.dialog.clone();
    state.find_replace_dialog = None;
    Some(ExistingFindReplaceDialogAction::Close(dialog))
}

fn register_find_replace_dialog(
    state: &Rc<RefCell<LinuxState>>,
    kind: FindReplaceDialogKind,
    dialog: &gtk::Dialog,
) -> u64 {
    let mut state = state.borrow_mut();
    let id = state
        .find_replace_dialog_generation
        .checked_add(1)
        .unwrap_or(1);
    state.find_replace_dialog_generation = id;
    state.find_replace_dialog = Some(FindReplaceDialogState {
        id,
        kind,
        dialog: dialog.clone(),
    });
    id
}

fn clear_find_replace_dialog(state: &Rc<RefCell<LinuxState>>, dialog_id: u64) {
    let should_clear_highlight = {
        let mut state = state.borrow_mut();
        if state
            .find_replace_dialog
            .as_ref()
            .is_some_and(|active| active.id == dialog_id)
        {
            state.find_replace_dialog = None;
            true
        } else {
            false
        }
    };
    if should_clear_highlight {
        clear_active_find_match_highlight(state);
    }
}

fn install_find_replace_entry_text_limit(entry: &gtk::Entry) {
    entry.connect_insert_text(|entry, text, position| {
        let current = entry.text();
        let selection = entry.selection_bounds();
        let Some(limited) = limited_find_replace_insert_text(
            current.as_str(),
            text,
            *position,
            selection,
            FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS,
        ) else {
            return;
        };

        glib::signal_stop_emission_by_name(entry, "insert-text");
        let (start, end) = entry_replacement_range(current.as_str(), *position, selection);
        let mut insert_position = start;
        if selection.is_some() {
            entry.delete_text(start, end);
        }
        if !limited.is_empty() {
            entry.insert_text(&limited, &mut insert_position);
        }
        *position = insert_position;
    });
}

fn limited_find_replace_insert_text(
    current: &str,
    insert: &str,
    position: i32,
    selection: Option<(i32, i32)>,
    limit_utf16_units: usize,
) -> Option<String> {
    let (start, end) = entry_replacement_range(current, position, selection);
    let start = byte_index_for_char_offset(current, start);
    let end = byte_index_for_char_offset(current, end);
    let selected_units = current[start..end].encode_utf16().count();
    let current_units = current.encode_utf16().count();
    let available = limit_utf16_units.saturating_sub(current_units.saturating_sub(selected_units));
    if insert.encode_utf16().count() <= available {
        return None;
    }

    Some(truncate_to_utf16_units(insert, available).to_owned())
}

fn entry_replacement_range(
    current: &str,
    position: i32,
    selection: Option<(i32, i32)>,
) -> (i32, i32) {
    let max = current.chars().count().min(i32::MAX as usize) as i32;
    let position = position.clamp(0, max);
    let Some((start, end)) = selection else {
        return (position, position);
    };
    let start = start.clamp(0, max);
    let end = end.clamp(0, max);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn truncate_to_utf16_units(text: &str, max_units: usize) -> &str {
    let mut used = 0usize;
    for (index, ch) in text.char_indices() {
        let next = used.saturating_add(ch.len_utf16());
        if next > max_units {
            return &text[..index];
        }
        used = next;
    }
    text
}

fn find_next_in_active_editor(
    state: &Rc<RefCell<LinuxState>>,
    needle: &str,
) -> Result<(), AppError> {
    ensure_active_replace_target(state)?;
    if needle.is_empty() {
        let language = state.borrow().app.ui_settings().language;
        return Err(AppError::user(ui_text(language).missing_find_text()));
    }
    sync_active_editor_content(state)?;
    let (page, start, end, theme) = {
        let state = state.borrow();
        let Some(tab) = state.tabs.active() else {
            return Err(AppError::platform(
                "find text",
                "active tab was not available",
            ));
        };
        let Some(page) = active_page(&state) else {
            return Err(AppError::platform(
                "find text",
                "active editor was not available",
            ));
        };
        let content = tab.content.as_str();
        let start = current_buffer_selection_byte_range(&page.buffer, content).end;
        let Some(found) = find_next_wrapping(content, needle, start) else {
            return Err(AppError::user(
                ui_text(state.app.ui_settings().language).no_match(),
            ));
        };
        let (start, end) = char_offsets_for_byte_range(content, found.start, found.end);
        (
            page.clone(),
            start,
            end,
            state.app.ui_settings().appearance.theme,
        )
    };
    select_find_result_offsets(&page.view, &page.buffer, start, end, theme);
    Ok(())
}

fn replace_one_in_active_editor(
    state: &Rc<RefCell<LinuxState>>,
    needle: &str,
    replacement: &str,
) -> Result<(), AppError> {
    ensure_active_replace_target(state)?;
    if needle.is_empty() {
        let language = state.borrow().app.ui_settings().language;
        return Err(AppError::user(ui_text(language).missing_find_text()));
    }
    sync_active_editor_content(state)?;
    let plan = {
        let state = state.borrow();
        let Some(tab) = state.tabs.active() else {
            return Err(AppError::platform(
                "replace text",
                "active tab was not available",
            ));
        };
        let Some(page) = active_page(&state) else {
            return Err(AppError::platform(
                "replace text",
                "active editor was not available",
            ));
        };
        let content = tab.content.as_str();
        let selection = current_buffer_selection_byte_range(&page.buffer, content);
        match replace_one_action(content, needle, selection) {
            ReplaceOneAction::ReplaceSelected(target) => {
                let next_start = target.start.checked_add(replacement.len()).ok_or_else(|| {
                    AppError::platform("replace text", "replacement position is too large")
                })?;
                if replacement == needle {
                    let (selection_start, selection_end) =
                        find_next_wrapping(content, needle, next_start)
                            .map(|found| {
                                char_offsets_for_byte_range(content, found.start, found.end)
                            })
                            .unwrap_or_else(|| {
                                let offset = char_offset_for_byte_index(content, next_start);
                                (offset, offset)
                            });
                    ReplaceOnePlan::Unchanged {
                        page: page.clone(),
                        selection_start,
                        selection_end,
                        theme: state.app.ui_settings().appearance.theme,
                    }
                } else {
                    let (target_start, target_end) =
                        char_offsets_for_byte_range(content, target.start, target.end);
                    ReplaceOnePlan::Replace {
                        page: page.clone(),
                        target,
                        target_start,
                        target_end,
                        next_start,
                        theme: state.app.ui_settings().appearance.theme,
                    }
                }
            }
            ReplaceOneAction::SelectNext(found) => {
                let (start, end) = char_offsets_for_byte_range(content, found.start, found.end);
                ReplaceOnePlan::Select {
                    page: page.clone(),
                    start,
                    end,
                    theme: state.app.ui_settings().appearance.theme,
                }
            }
            ReplaceOneAction::NoMatch => {
                return Err(AppError::user(
                    ui_text(state.app.ui_settings().language).no_match(),
                ));
            }
        }
    };

    match plan {
        ReplaceOnePlan::Select {
            page,
            start,
            end,
            theme,
        } => {
            select_find_result_offsets(&page.view, &page.buffer, start, end, theme);
        }
        ReplaceOnePlan::Replace {
            page,
            target,
            target_start,
            target_end,
            next_start,
            theme,
        } => {
            let (selection_start, selection_end) = {
                let mut state = state.borrow_mut();
                if !state
                    .tabs
                    .replace_active_content_range(target.start..target.end, replacement)
                {
                    return Err(AppError::platform(
                        "replace text",
                        "selected text no longer matches",
                    ));
                }
                let Some(content) = state.tabs.active().map(|tab| tab.content.as_str()) else {
                    return Err(AppError::platform(
                        "replace text",
                        "active tab was not available",
                    ));
                };
                find_next_wrapping(content, needle, next_start)
                    .map(|found| char_offsets_for_byte_range(content, found.start, found.end))
                    .unwrap_or_else(|| {
                        let offset = char_offset_for_byte_index(content, next_start);
                        (offset, offset)
                    })
            };
            replace_buffer_char_range(state, &page.buffer, target_start, target_end, replacement);
            update_actions(state);
            update_window_title(state);
            select_find_result_offsets(
                &page.view,
                &page.buffer,
                selection_start,
                selection_end,
                theme,
            );
        }
        ReplaceOnePlan::Unchanged {
            page,
            selection_start,
            selection_end,
            theme,
        } => {
            update_actions(state);
            select_find_result_offsets(
                &page.view,
                &page.buffer,
                selection_start,
                selection_end,
                theme,
            );
        }
    }
    Ok(())
}

fn replace_all_in_active_editor(
    state: &Rc<RefCell<LinuxState>>,
    needle: &str,
    replacement: &str,
) -> Result<(), AppError> {
    ensure_active_replace_target(state)?;
    if needle.is_empty() {
        let language = state.borrow().app.ui_settings().language;
        return Err(AppError::user(ui_text(language).missing_find_text()));
    }
    sync_active_editor_content(state)?;
    let (language, window, count, next_content) = {
        let state = state.borrow();
        let Some(tab) = state.tabs.active() else {
            return Err(AppError::platform(
                "replace all text",
                "active tab was not available",
            ));
        };
        let language = state.app.ui_settings().language;
        let result = replace_all_literal(
            &tab.content,
            needle,
            replacement,
            REPLACE_ALL_OUTPUT_BYTE_LIMIT,
        )
        .map_err(|error| replace_all_error_to_app_error(error, language))?;
        let next_content = match result.content {
            std::borrow::Cow::Owned(content) if result.count > 0 => Some(content),
            _ => None,
        };
        (
            language,
            state.widgets.window.clone(),
            result.count,
            next_content,
        )
    };
    if let Some(content) = next_content {
        state.borrow_mut().tabs.update_active_content(content);
        refresh_tabs(state)?;
        update_window_title(state);
    }
    show_info_message(
        Some(window.upcast_ref()),
        "j3TreeText",
        &ui_text(language).replace_all_count(count),
    );
    Ok(())
}

fn replace_all_error_to_app_error(error: ReplaceAllError, language: UiLanguage) -> AppError {
    let text = ui_text(language);
    match error {
        ReplaceAllError::OutputTooLarge { limit } => {
            AppError::user(text.replace_all_too_large(limit / 1024 / 1024))
        }
        ReplaceAllError::OutputSizeOverflow => AppError::user(text.replace_all_overflow()),
        ReplaceAllError::OutputAllocationFailed { requested } => {
            AppError::user(text.replace_all_allocation_failed(requested))
        }
    }
}

fn find_next_wrapping(content: &str, needle: &str, start: usize) -> Option<TextMatch> {
    find_next_literal(content, needle, start).or_else(|| {
        (start > 0)
            .then(|| find_next_literal(content, needle, 0))
            .flatten()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplaceOneAction {
    ReplaceSelected(TextMatch),
    SelectNext(TextMatch),
    NoMatch,
}

enum ReplaceOnePlan {
    Select {
        page: TabPage,
        start: i32,
        end: i32,
        theme: AppearanceTheme,
    },
    Replace {
        page: TabPage,
        target: TextMatch,
        target_start: i32,
        target_end: i32,
        next_start: usize,
        theme: AppearanceTheme,
    },
    Unchanged {
        page: TabPage,
        selection_start: i32,
        selection_end: i32,
        theme: AppearanceTheme,
    },
}

fn replace_one_action(content: &str, needle: &str, selection: TextMatch) -> ReplaceOneAction {
    if selection.end > selection.start
        && content.get(selection.start..selection.end) == Some(needle)
    {
        return ReplaceOneAction::ReplaceSelected(selection);
    }

    match find_next_wrapping(content, needle, selection.end) {
        Some(found) => ReplaceOneAction::SelectNext(found),
        None => ReplaceOneAction::NoMatch,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeMode {
    Active,
    Trash,
}

#[derive(Clone)]
struct UiDocument {
    nodes: Vec<UiNode>,
    node_indices_by_id: HashMap<i64, usize>,
    display_child_indices_by_parent: HashMap<Option<i64>, Vec<usize>>,
}

impl UiDocument {
    fn from_active_document(document: &Document, language: UiLanguage) -> Result<Self, AppError> {
        Self::from_nodes(document.nodes(), false, language, true)
    }

    fn from_trash_nodes(nodes: &[Node], language: UiLanguage) -> Result<Self, AppError> {
        Self::from_nodes(nodes, true, language, false)
    }

    fn from_search_results(
        results: Vec<DocumentSearchResult>,
        language: UiLanguage,
    ) -> Result<Self, AppError> {
        let nodes = results
            .into_iter()
            .map(|result| UiNode::from_search_result(result, language))
            .collect::<Vec<_>>();
        Ok(Self::from_ui_nodes(nodes))
    }

    fn from_nodes(
        source_nodes: &[Node],
        trash_mode: bool,
        language: UiLanguage,
        trusted_order: bool,
    ) -> Result<Self, AppError> {
        let deleted_ids = if trash_mode {
            source_nodes
                .iter()
                .map(|node| node.id)
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        };
        let mut nodes = source_nodes
            .iter()
            .map(|node| {
                let display_parent_id = if trash_mode {
                    node.parent_id
                        .filter(|parent_id| deleted_ids.contains(parent_id))
                } else {
                    node.parent_id
                };
                UiNode::from_node(node, display_parent_id, trash_mode, language)
            })
            .collect::<Vec<_>>();
        if !trusted_order {
            nodes.sort_by(compare_ui_nodes_for_display);
        }
        Ok(Self::from_ui_nodes_with_order(nodes, trusted_order))
    }

    fn from_ui_nodes(nodes: Vec<UiNode>) -> Self {
        Self::from_ui_nodes_with_order(nodes, false)
    }

    fn from_ui_nodes_with_order(nodes: Vec<UiNode>, trusted_order: bool) -> Self {
        let mut node_indices_by_id = HashMap::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            node_indices_by_id.insert(node.id, index);
        }
        let display_child_indices_by_parent =
            display_child_indices_by_parent(&nodes, trusted_order);
        Self {
            nodes,
            node_indices_by_id,
            display_child_indices_by_parent,
        }
    }

    fn node_by_id(&self, node_id: i64) -> Option<&UiNode> {
        self.node_indices_by_id
            .get(&node_id)
            .and_then(|index| self.nodes.get(*index))
            .filter(|node| node.id == node_id)
            .or_else(|| self.nodes.iter().find(|node| node.id == node_id))
    }

    fn visible_order(
        &self,
        expanded_node_ids: &HashSet<i64>,
        expand_all: bool,
    ) -> Vec<(usize, usize)> {
        let context = VisibleOrderContext {
            children: &self.display_child_indices_by_parent,
            expanded_node_ids,
            expand_all,
        };
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut seen = HashSet::with_capacity(self.nodes.len());
        self.collect_visible_order(None, 0, &context, &mut seen, &mut order);
        for index in 0..self.nodes.len() {
            let node = &self.nodes[index];
            let is_orphan_root = node
                .display_parent_id
                .is_none_or(|parent_id| !self.node_indices_by_id.contains_key(&parent_id));
            if is_orphan_root && seen.insert(node.id) {
                order.push((index, 0));
            }
        }
        order
    }

    fn has_display_children(&self, node_id: i64) -> bool {
        self.display_child_indices_by_parent
            .contains_key(&Some(node_id))
    }

    fn visible_descendant_order(
        &self,
        root_node_id: i64,
        root_depth: usize,
        expanded_node_ids: &HashSet<i64>,
    ) -> Vec<(usize, usize)> {
        let Some(root_node) = self.node_by_id(root_node_id) else {
            return Vec::new();
        };
        if root_node
            .display_parent_id
            .is_some_and(|parent_id| !self.node_indices_by_id.contains_key(&parent_id))
        {
            return Vec::new();
        }
        let context = VisibleOrderContext {
            children: &self.display_child_indices_by_parent,
            expanded_node_ids,
            expand_all: false,
        };
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        self.collect_visible_order(
            Some(root_node_id),
            root_depth.saturating_add(1),
            &context,
            &mut seen,
            &mut order,
        );
        order
    }

    fn display_depth(&self, node_id: i64) -> usize {
        let mut depth = 0usize;
        let mut current_parent = self
            .node_by_id(node_id)
            .and_then(|node| node.display_parent_id);
        let mut seen = HashSet::new();
        while let Some(parent_id) = current_parent {
            if !seen.insert(parent_id) {
                break;
            }
            let Some(parent) = self.node_by_id(parent_id) else {
                break;
            };
            depth = depth.saturating_add(1);
            current_parent = parent.display_parent_id;
        }
        depth
    }

    fn expandable_node_ids(&self) -> HashSet<i64> {
        self.display_child_indices_by_parent
            .keys()
            .copied()
            .flatten()
            .collect()
    }

    fn expandable_subtree_node_ids(&self, root_node_id: i64) -> HashSet<i64> {
        let mut expandable_node_ids = HashSet::new();
        let mut pending = vec![root_node_id];
        let mut seen = HashSet::new();
        while let Some(node_id) = pending.pop() {
            if !seen.insert(node_id) {
                continue;
            }
            let Some(child_indices) = self.display_child_indices_by_parent.get(&Some(node_id))
            else {
                continue;
            };
            expandable_node_ids.insert(node_id);
            pending.extend(child_indices.iter().map(|index| self.nodes[*index].id));
        }
        expandable_node_ids
    }

    fn display_ancestor_node_ids(&self, node_id: i64) -> Vec<i64> {
        let mut ancestors = Vec::new();
        let mut current_parent = self
            .node_by_id(node_id)
            .and_then(|node| node.display_parent_id);
        let mut seen = HashSet::new();
        while let Some(parent_id) = current_parent {
            if !seen.insert(parent_id) {
                break;
            }
            ancestors.push(parent_id);
            current_parent = self
                .node_by_id(parent_id)
                .and_then(|node| node.display_parent_id);
        }
        ancestors
    }

    fn collect_visible_order(
        &self,
        parent_id: Option<i64>,
        depth: usize,
        context: &VisibleOrderContext<'_>,
        seen: &mut HashSet<i64>,
        order: &mut Vec<(usize, usize)>,
    ) {
        let Some(indices) = context.children.get(&parent_id) else {
            return;
        };
        for &index in indices {
            let node = &self.nodes[index];
            if !seen.insert(node.id) {
                continue;
            }
            order.push((index, depth));
            if context.expand_all || context.expanded_node_ids.contains(&node.id) {
                self.collect_visible_order(Some(node.id), depth + 1, context, seen, order);
            }
        }
    }
}

fn display_child_indices_by_parent(
    nodes: &[UiNode],
    trusted_order: bool,
) -> HashMap<Option<i64>, Vec<usize>> {
    let mut children = HashMap::<Option<i64>, Vec<usize>>::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        children
            .entry(node.display_parent_id)
            .or_default()
            .push(index);
    }
    if !trusted_order {
        for indices in children.values_mut() {
            indices
                .sort_by(|left, right| compare_ui_nodes_for_display(&nodes[*left], &nodes[*right]));
        }
    }
    children
}

struct VisibleOrderContext<'a> {
    children: &'a HashMap<Option<i64>, Vec<usize>>,
    expanded_node_ids: &'a HashSet<i64>,
    expand_all: bool,
}

#[derive(Clone)]
struct UiNode {
    id: i64,
    parent_id: Option<i64>,
    display_parent_id: Option<i64>,
    title: String,
    sort_order: i64,
    display_title: String,
    updated_at: String,
    editable: bool,
    source: DocumentTabSource,
    search_content_matched: bool,
}

impl UiNode {
    fn from_node(
        node: &Node,
        display_parent_id: Option<i64>,
        trash_mode: bool,
        language: UiLanguage,
    ) -> Self {
        let display_title = if trash_mode {
            ui_text(language).deleted_title(&node.title)
        } else {
            node.title.clone()
        };
        Self {
            id: node.id,
            parent_id: node.parent_id,
            display_parent_id,
            title: node.title.clone(),
            sort_order: node.sort_order,
            display_title,
            updated_at: node.updated_at.clone(),
            editable: !trash_mode,
            source: if trash_mode {
                DocumentTabSource::Trash
            } else {
                DocumentTabSource::ActiveTree
            },
            search_content_matched: false,
        }
    }

    fn from_search_result(result: DocumentSearchResult, language: UiLanguage) -> Self {
        let DocumentSearchResult {
            node,
            parent_title,
            content_matched,
        } = result;
        let display_title =
            ui_text(language).search_result_title(&node.title, parent_title.as_deref());
        Self {
            id: node.id,
            parent_id: node.parent_id,
            display_parent_id: None,
            title: node.title,
            sort_order: node.sort_order,
            display_title,
            updated_at: node.updated_at,
            editable: true,
            source: DocumentTabSource::SearchResult,
            search_content_matched: content_matched,
        }
    }
}

fn compare_ui_nodes_for_display(left: &UiNode, right: &UiNode) -> std::cmp::Ordering {
    (
        left.display_parent_id,
        left.sort_order,
        left.title.as_str(),
        left.id,
    )
        .cmp(&(
            right.display_parent_id,
            right.sort_order,
            right.title.as_str(),
            right.id,
        ))
}

fn can_refresh_renamed_tree_row(
    previous: &UiDocument,
    next: &UiDocument,
    visible_node_ids: &[i64],
    node_id: i64,
) -> bool {
    if previous.nodes.len() != next.nodes.len() || !visible_node_ids.contains(&node_id) {
        return false;
    }

    for previous_node in &previous.nodes {
        let Some(next_node) = next.node_by_id(previous_node.id) else {
            return false;
        };
        if previous_node.id == node_id {
            if !renamed_tree_node_keeps_row_position(previous, next, previous_node, next_node) {
                return false;
            }
        } else if !tree_node_display_identity_matches(previous_node, next_node) {
            return false;
        }
    }

    true
}

fn renamed_tree_node_keeps_row_position(
    previous: &UiDocument,
    next: &UiDocument,
    previous_node: &UiNode,
    next_node: &UiNode,
) -> bool {
    previous_node.parent_id == next_node.parent_id
        && previous_node.display_parent_id == next_node.display_parent_id
        && previous_node.sort_order == next_node.sort_order
        && previous_node.editable == next_node.editable
        && previous_node.source == next_node.source
        && previous_node.search_content_matched == next_node.search_content_matched
        && previous.has_display_children(previous_node.id)
            == next.has_display_children(next_node.id)
        && !next.nodes.iter().any(|node| {
            node.id != next_node.id
                && node.display_parent_id == next_node.display_parent_id
                && node.sort_order == next_node.sort_order
        })
}

fn tree_node_display_identity_matches(left: &UiNode, right: &UiNode) -> bool {
    left.parent_id == right.parent_id
        && left.display_parent_id == right.display_parent_id
        && left.title == right.title
        && left.sort_order == right.sort_order
        && left.display_title == right.display_title
        && left.editable == right.editable
        && left.source == right.source
        && left.search_content_matched == right.search_content_matched
}

fn sibling_move_availability(document: &UiDocument, selected_node_id: Option<i64>) -> (bool, bool) {
    let Some(selected_node_id) = selected_node_id else {
        return (false, false);
    };
    if selected_node_id == ROOT_NODE_ID {
        return (false, false);
    }
    let Some(selected_node) = document.node_by_id(selected_node_id) else {
        return (false, false);
    };
    if let Some(sibling_indices) = document
        .display_child_indices_by_parent
        .get(&selected_node.display_parent_id)
    {
        let mut sibling_count = 0usize;
        let mut selected_position = None;
        for &index in sibling_indices {
            let Some(node) = document.nodes.get(index) else {
                continue;
            };
            if node.display_parent_id != selected_node.display_parent_id {
                continue;
            }
            if node.id == selected_node_id {
                selected_position = Some(sibling_count);
            }
            sibling_count = sibling_count.saturating_add(1);
        }
        if let Some(position) = selected_position {
            return (position > 0, position + 1 < sibling_count);
        }
    }

    sibling_move_availability_from_candidates(&document.nodes, selected_node_id, selected_node)
}

fn sibling_move_availability_from_candidates(
    candidates: &[UiNode],
    selected_node_id: i64,
    selected_node: &UiNode,
) -> (bool, bool) {
    let mut has_previous_sibling = false;
    let mut has_next_sibling = false;
    for node in candidates {
        if node.id == selected_node_id || node.display_parent_id != selected_node.display_parent_id
        {
            continue;
        }

        match compare_ui_nodes_for_display(node, selected_node) {
            std::cmp::Ordering::Less => has_previous_sibling = true,
            std::cmp::Ordering::Greater => has_next_sibling = true,
            std::cmp::Ordering::Equal => {}
        }

        if has_previous_sibling && has_next_sibling {
            break;
        }
    }

    (has_previous_sibling, has_next_sibling)
}

fn subtree_node_ids(document: &UiDocument, root_node_id: i64) -> Vec<i64> {
    let mut children = HashMap::<i64, Vec<i64>>::new();
    for node in &document.nodes {
        if let Some(parent_id) = node.parent_id {
            children.entry(parent_id).or_default().push(node.id);
        }
    }
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    let mut pending = vec![root_node_id];
    while let Some(node_id) = pending.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        ids.push(node_id);
        if let Some(child_ids) = children.get(&node_id) {
            pending.extend(child_ids.iter().rev().copied());
        }
    }
    ids
}

#[derive(Clone, Copy)]
struct UiText {
    language: UiLanguage,
}

fn ui_text(language: UiLanguage) -> UiText {
    UiText { language }
}

impl UiText {
    fn menu_file(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "파일",
            UiLanguage::English => "File",
        }
    }

    fn menu_edit(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "편집",
            UiLanguage::English => "Edit",
        }
    }

    fn menu_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "문서",
            UiLanguage::English => "Document",
        }
    }

    fn menu_view(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "보기",
            UiLanguage::English => "View",
        }
    }

    fn menu_help(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "도움말",
            UiLanguage::English => "Help",
        }
    }

    fn save_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "저장\tCtrl+S",
            UiLanguage::English => "Save\tCtrl+S",
        }
    }

    fn import_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져오기...",
            UiLanguage::English => "Import...",
        }
    }

    fn import_encoding(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져올 인코딩",
            UiLanguage::English => "Import Encoding",
        }
    }

    fn export_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "내보내기...",
            UiLanguage::English => "Export...",
        }
    }

    fn export_all_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 문서 내보내기...",
            UiLanguage::English => "Export All Documents...",
        }
    }

    fn export_encoding(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "내보낼 인코딩",
            UiLanguage::English => "Export Encoding",
        }
    }

    fn close_tab(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "탭 닫기\tCtrl+W",
            UiLanguage::English => "Close Tab\tCtrl+W",
        }
    }

    fn close_window(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "종료",
            UiLanguage::English => "Exit",
        }
    }

    fn undo(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "실행 취소",
            UiLanguage::English => "Undo",
        }
    }

    fn cut(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "잘라내기",
            UiLanguage::English => "Cut",
        }
    }

    fn copy(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "복사",
            UiLanguage::English => "Copy",
        }
    }

    fn paste(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "붙여넣기",
            UiLanguage::English => "Paste",
        }
    }

    fn delete_selection(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "삭제",
            UiLanguage::English => "Delete",
        }
    }

    fn select_all(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "전체 선택\tCtrl+A",
            UiLanguage::English => "Select All\tCtrl+A",
        }
    }

    fn find_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "찾기...\tCtrl+F",
            UiLanguage::English => "Find...\tCtrl+F",
        }
    }

    fn replace_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "바꾸기...\tCtrl+H",
            UiLanguage::English => "Replace...\tCtrl+H",
        }
    }

    fn new_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "새 문서\tCtrl+N",
            UiLanguage::English => "New Document\tCtrl+N",
        }
    }

    fn new_child_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "새 하위 문서\tCtrl+Enter",
            UiLanguage::English => "New Child Document\tCtrl+Enter",
        }
    }

    fn rename(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "이름 변경\tF2",
            UiLanguage::English => "Rename\tF2",
        }
    }

    fn move_up(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "위로\tCtrl+Up",
            UiLanguage::English => "Move Up\tCtrl+Up",
        }
    }

    fn move_down(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "아래로\tCtrl+Down",
            UiLanguage::English => "Move Down\tCtrl+Down",
        }
    }

    fn move_to_trash(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통으로 이동\tDelete",
            UiLanguage::English => "Move to Trash\tDelete",
        }
    }

    fn restore(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "복원",
            UiLanguage::English => "Restore",
        }
    }

    fn delete_permanently(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "영구 삭제",
            UiLanguage::English => "Delete Permanently",
        }
    }

    fn document_tree(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "문서 트리",
            UiLanguage::English => "Document Tree",
        }
    }

    fn trash(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통",
            UiLanguage::English => "Trash",
        }
    }

    fn theme(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "테마",
            UiLanguage::English => "Theme",
        }
    }

    fn language(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "언어",
            UiLanguage::English => "Language",
        }
    }

    fn word_wrap(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "자동 줄바꿈",
            UiLanguage::English => "Word Wrap",
        }
    }

    fn editor_font(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "편집기 글꼴...",
            UiLanguage::English => "Editor Font...",
        }
    }

    fn font_fallback(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "선택한 글꼴을 사용할 수 없어 기본 글꼴을 적용했습니다.",
            UiLanguage::English => {
                "The selected font is unavailable. The default font was applied."
            }
        }
    }

    fn about_menu(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "j3TreeText 정보",
            UiLanguage::English => "About j3TreeText",
        }
    }

    fn search_cue(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "제목 또는 본문 검색",
            UiLanguage::English => "Search title or content",
        }
    }

    fn caret_position_status(self, line: usize, column: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("줄 {line}, 열 {column}"),
            UiLanguage::English => format!("Ln {line}, Col {column}"),
        }
    }

    fn text_encoding_name(self, encoding: TextEncoding) -> &'static str {
        match encoding {
            TextEncoding::AutoDetect => match self.language {
                UiLanguage::Korean => "자동 감지",
                UiLanguage::English => "Auto Detect",
            },
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8WithBom => "UTF-8 BOM",
            TextEncoding::Utf16LeWithBom => "UTF-16 LE BOM",
            TextEncoding::Utf16BeWithBom => "UTF-16 BE BOM",
            TextEncoding::KoreanEucKr => match self.language {
                UiLanguage::Korean => "한국어 (EUC-KR/CP949)",
                UiLanguage::English => "Korean (EUC-KR/CP949)",
            },
            TextEncoding::Windows1252 => "Windows-1252",
        }
    }

    fn theme_name(self, theme: AppearanceTheme) -> &'static str {
        match theme {
            AppearanceTheme::Light => match self.language {
                UiLanguage::Korean => "밝게",
                UiLanguage::English => "Light",
            },
            AppearanceTheme::ClassicDark => match self.language {
                UiLanguage::Korean => "어둡게",
                UiLanguage::English => "Dark",
            },
            AppearanceTheme::SepiaTeal => match self.language {
                UiLanguage::Korean => "세피아",
                UiLanguage::English => "Sepia",
            },
            AppearanceTheme::Graphite => match self.language {
                UiLanguage::Korean => "그래파이트",
                UiLanguage::English => "Graphite",
            },
            AppearanceTheme::Forest => match self.language {
                UiLanguage::Korean => "숲",
                UiLanguage::English => "Forest",
            },
            AppearanceTheme::SteelBlue => match self.language {
                UiLanguage::Korean => "스틸 블루",
                UiLanguage::English => "Steel Blue",
            },
        }
    }

    fn deleted_title(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("[삭제됨] {title}"),
            UiLanguage::English => format!("[Deleted] {title}"),
        }
    }

    fn search_result_title(self, title: &str, parent_title: Option<&str>) -> String {
        let parent_title = parent_title.unwrap_or_else(|| self.no_parent());
        match self.language {
            UiLanguage::Korean => format!("{title} (부모: {parent_title})"),
            UiLanguage::English => format!("{title} (Parent: {parent_title})"),
        }
    }

    fn no_parent(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "상위 없음",
            UiLanguage::English => "No parent",
        }
    }

    fn save_conflict_message(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => {
                "다른 곳에서 먼저 저장되었습니다.\n현재 내용을 덮어쓰지 않습니다.\n\n예: 최신 내용 다시 불러오기\n아니요: 새 문서로 저장\n취소: 계속 편집"
            }
            UiLanguage::English => {
                "This document was saved elsewhere first.\nYour current content will not overwrite it.\n\nYes: reload the latest content\nNo: save as a new document\nCancel: keep editing"
            }
        }
    }

    fn conflicted_copy_title(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("{title} (복구됨)"),
            UiLanguage::English => format!("{title} (recovered)"),
        }
    }

    fn missing_find_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "찾을 내용을 입력하세요.",
            UiLanguage::English => "Enter text to find.",
        }
    }

    fn open_editable_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "편집 가능한 문서를 먼저 여세요.",
            UiLanguage::English => "Open an editable document first.",
        }
    }

    fn read_only_find_replace(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "읽기 전용 문서에서는 찾기/바꾸기를 사용할 수 없습니다.",
            UiLanguage::English => "Find/replace is not available in read-only documents.",
        }
    }

    fn no_match(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "일치 항목이 없습니다.",
            UiLanguage::English => "No matches found.",
        }
    }

    fn replace_all_too_large(self, limit_mib: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("결과가 너무 큽니다. {limit_mib}MiB 이하로 줄이세요."),
            UiLanguage::English => {
                format!("The result is too large. Keep it under {limit_mib}MiB.")
            }
        }
    }

    fn replace_all_overflow(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => {
                "결과가 처리 가능한 범위를 넘었습니다. 문서나 바꿀 내용을 줄이세요."
            }
            UiLanguage::English => {
                "The result is too large to process. Reduce the document or replacement text."
            }
        }
    }

    fn replace_all_allocation_failed(self, requested: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("메모리가 부족합니다. 예상 결과: {requested}바이트"),
            UiLanguage::English => {
                format!("Not enough memory. Estimated result: {requested} bytes")
            }
        }
    }

    fn replace_all_count(self, count: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("{count}개 변경됨"),
            UiLanguage::English => format!("{count} replacements made"),
        }
    }

    fn unsaved_changes(self, title: Option<&str>) -> String {
        match (self.language, title) {
            (UiLanguage::Korean, Some(title)) => {
                format!("\"{title}\"에 저장하지 않은 변경이 있습니다.\n저장할까요?")
            }
            (UiLanguage::Korean, None) => "저장하지 않은 변경이 있습니다.\n저장할까요?".to_owned(),
            (UiLanguage::English, Some(title)) => {
                format!("\"{title}\" has unsaved changes.\nSave them?")
            }
            (UiLanguage::English, None) => "There are unsaved changes.\nSave them?".to_owned(),
        }
    }

    fn discard_unsavable_changes(self, title: Option<&str>) -> String {
        match (self.language, title) {
            (UiLanguage::Korean, Some(title)) => {
                format!("\"{title}\"은 저장할 수 없습니다.\n변경을 버리고 계속할까요?")
            }
            (UiLanguage::Korean, None) => {
                "현재 저장할 수 없습니다.\n변경을 버리고 계속할까요?".to_owned()
            }
            (UiLanguage::English, Some(title)) => {
                format!("\"{title}\" cannot be saved.\nDiscard changes and continue?")
            }
            (UiLanguage::English, None) => {
                "The current changes cannot be saved.\nDiscard them and continue?".to_owned()
            }
        }
    }

    fn window_trash_suffix(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통",
            UiLanguage::English => "Trash",
        }
    }

    fn window_search_suffix(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "검색",
            UiLanguage::English => "Search",
        }
    }

    fn about_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "j3TreeText 정보",
            UiLanguage::English => "About j3TreeText",
        }
    }

    fn about_message(self, version: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("j3TreeText {version}"),
            UiLanguage::English => format!("j3TreeText {version}"),
        }
    }

    fn ok_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "확인",
            UiLanguage::English => "OK",
        }
    }

    fn open_import_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져올 문서를 먼저 여세요.",
            UiLanguage::English => "Open a document before importing.",
        }
    }

    fn open_export_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "내보낼 문서를 먼저 여세요.",
            UiLanguage::English => "Open a document before exporting.",
        }
    }

    fn imported_text_nul(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => {
                "가져온 텍스트에 지원하지 않는 NUL 문자가 있습니다. 해당 문자를 제거하세요."
            }
            UiLanguage::English => {
                "The imported text contains unsupported NUL characters. Remove them first."
            }
        }
    }

    fn imported_text_too_large(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져온 텍스트가 너무 큽니다. 파일을 나누어 가져오세요.",
            UiLanguage::English => "The imported text is too large. Split the file and try again.",
        }
    }

    fn editor_text_too_large(self, limit_mib: usize) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("편집기 본문이 너무 큽니다. {limit_mib}MiB 이하로 줄인 뒤 다시 시도하세요.")
            }
            UiLanguage::English => {
                format!("The editor text is too large. Reduce it to {limit_mib}MiB or less and try again.")
            }
        }
    }

    fn export_text_too_large(self, limit_mib: usize) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("내보낼 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누세요.")
            }
            UiLanguage::English => {
                format!("The text to export is too large. Split it into {limit_mib}MiB or smaller chunks.")
            }
        }
    }

    fn export_all_complete_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 문서 내보내기",
            UiLanguage::English => "Export All Documents",
        }
    }

    fn export_all_complete_message(self, count: usize, path: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("{count}개 문서를 내보냈습니다.\n경로: {path}"),
            UiLanguage::English => {
                format!("Exported {count} documents.\nPath: {path}")
            }
        }
    }

    fn active_tree_only(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통 보기에서는 사용할 수 없습니다.",
            UiLanguage::English => "This command is not available in Trash view.",
        }
    }

    fn search_not_allowed(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "검색 중에는 사용할 수 없습니다. 검색어를 지우세요.",
            UiLanguage::English => {
                "This command is not available while searching. Clear the search text."
            }
        }
    }

    fn trash_only(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통 보기에서만 사용할 수 있습니다.",
            UiLanguage::English => "This command is only available in Trash view.",
        }
    }

    fn confirm_delete(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("\"{title}\"을 휴지통으로 이동할까요?\n하위 문서도 함께 이동합니다.")
            }
            UiLanguage::English => {
                format!("Move \"{title}\" to the trash?\nChild documents will also be moved.")
            }
        }
    }

    fn confirm_restore(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("\"{title}\"을 복원할까요?\n원래 위치가 없으면 루트 아래로 이동합니다.")
            }
            UiLanguage::English => {
                format!("Restore \"{title}\"?\nIf the original location is missing, it will be moved under the root.")
            }
        }
    }

    fn confirm_permanent_delete(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("\"{title}\"을 영구 삭제할까요?\n하위 문서도 함께 삭제됩니다.")
            }
            UiLanguage::English => {
                format!("Permanently delete \"{title}\"?\nChild documents will also be deleted.")
            }
        }
    }

    fn file_dialog_import_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "텍스트 가져오기",
            UiLanguage::English => "Import Text",
        }
    }

    fn file_dialog_export_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "텍스트 내보내기",
            UiLanguage::English => "Export Text",
        }
    }

    fn file_dialog_export_folder_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 문서 내보낼 폴더 선택",
            UiLanguage::English => "Choose Export Folder",
        }
    }

    fn file_filter_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "텍스트 파일 (*.txt)",
            UiLanguage::English => "Text Files (*.txt)",
        }
    }

    fn file_filter_all(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 파일 (*.*)",
            UiLanguage::English => "All Files (*.*)",
        }
    }

    fn local_file_required(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "로컬 파일 경로를 선택하세요.",
            UiLanguage::English => "Choose a local file path.",
        }
    }

    fn confirm_overwrite_file(self, path: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("이미 있는 파일을 바꾸시겠습니까?\n{path}"),
            UiLanguage::English => format!("Replace the existing file?\n{path}"),
        }
    }

    fn find_label(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "찾을 내용",
            UiLanguage::English => "Find",
        }
    }

    fn replace_label(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "바꿀 내용",
            UiLanguage::English => "Replace With",
        }
    }

    fn find_next_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "다음 찾기",
            UiLanguage::English => "Find Next",
        }
    }

    fn replace_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "바꾸기",
            UiLanguage::English => "Replace",
        }
    }

    fn replace_all_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모두 바꾸기",
            UiLanguage::English => "Replace All",
        }
    }

    fn close_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "닫기",
            UiLanguage::English => "Close",
        }
    }

    fn yes_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "예",
            UiLanguage::English => "_Yes",
        }
    }

    fn no_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "아니요",
            UiLanguage::English => "_No",
        }
    }

    fn open_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "열기",
            UiLanguage::English => "_Open",
        }
    }

    fn save_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "저장",
            UiLanguage::English => "_Save",
        }
    }

    fn select_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "선택",
            UiLanguage::English => "_Select",
        }
    }

    fn cancel_button(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "취소",
            UiLanguage::English => "_Cancel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictDecision {
    Reload,
    SaveAsNewDocument,
    Cancel,
}

fn prompt_save_conflict(parent: &gtk::Window, language: UiLanguage) -> ConflictDecision {
    let text = ui_text(language);
    let buttons = yes_no_cancel_buttons(text);
    let response = run_message_dialog(
        Some(parent),
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        "j3TreeText",
        text.save_conflict_message(),
        &buttons,
        Some(WIN32_QUESTION_DEFAULT_RESPONSE),
    );
    save_conflict_decision_from_response(response)
}

fn save_conflict_decision_from_response(response: gtk::ResponseType) -> ConflictDecision {
    match response {
        gtk::ResponseType::Yes => ConflictDecision::Reload,
        gtk::ResponseType::No => ConflictDecision::SaveAsNewDocument,
        _ => ConflictDecision::Cancel,
    }
}

fn prompt_unsaved_changes(
    parent: &gtk::Window,
    title: Option<&str>,
    language: UiLanguage,
) -> DirtyTabDecision {
    let text = ui_text(language);
    let buttons = yes_no_cancel_buttons(text);
    let response = run_message_dialog(
        Some(parent),
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        "j3TreeText",
        &text.unsaved_changes(title),
        &buttons,
        Some(WIN32_QUESTION_DEFAULT_RESPONSE),
    );
    dirty_tab_decision_from_response(response)
}

fn dirty_tab_decision_from_response(response: gtk::ResponseType) -> DirtyTabDecision {
    match response {
        gtk::ResponseType::Yes => DirtyTabDecision::Save,
        gtk::ResponseType::No => DirtyTabDecision::Discard,
        _ => DirtyTabDecision::Cancel,
    }
}

fn prompt_discard_unsavable_changes(
    parent: &gtk::Window,
    language: UiLanguage,
    title: Option<&str>,
) -> bool {
    confirm_question(
        Some(parent),
        "j3TreeText",
        &ui_text(language).discard_unsavable_changes(title),
        language,
    )
}

fn yes_no_buttons(text: UiText) -> [(&'static str, gtk::ResponseType); 2] {
    [
        (text.yes_button(), gtk::ResponseType::Yes),
        (text.no_button(), gtk::ResponseType::No),
    ]
}

fn yes_no_cancel_buttons(text: UiText) -> [(&'static str, gtk::ResponseType); 3] {
    [
        (text.yes_button(), gtk::ResponseType::Yes),
        (text.no_button(), gtk::ResponseType::No),
        (text.cancel_button(), gtk::ResponseType::Cancel),
    ]
}

fn confirm_question(
    parent: Option<&gtk::Window>,
    title: &str,
    message: &str,
    language: UiLanguage,
) -> bool {
    let buttons = yes_no_buttons(ui_text(language));
    run_message_dialog(
        parent,
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        title,
        message,
        &buttons,
        Some(WIN32_QUESTION_DEFAULT_RESPONSE),
    ) == gtk::ResponseType::Yes
}

fn show_info_message(parent: Option<&gtk::Window>, title: &str, message: &str) {
    let _ = run_message_dialog(
        parent,
        gtk::MessageType::Info,
        gtk::ButtonsType::Ok,
        title,
        message,
        &[],
        None,
    );
}

fn show_about_dialog(parent: Option<&gtk::Window>, text: UiText, title: &str, message: &str) {
    let dialog = gtk::Dialog::builder().title(title).modal(true).build();
    dialog.set_resizable(false);
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }

    let content_area = dialog.content_area();
    content_area.set_spacing(10);
    content_area.set_margin_top(18);
    content_area.set_margin_bottom(12);
    content_area.set_margin_start(24);
    content_area.set_margin_end(24);

    let message_label = gtk::Label::new(Some(message));
    message_label.set_xalign(0.0);
    message_label.set_wrap(true);
    content_area.append(&message_label);

    let link = gtk::LinkButton::with_label(APP_AUTHOR_URL, APP_AUTHOR_URL);
    link.set_halign(gtk::Align::Start);
    {
        let dialog = dialog.clone();
        link.connect_activate_link(move |button| {
            button.set_visited(true);
            gtk::show_uri(Some(&dialog), APP_AUTHOR_URL, gdk::CURRENT_TIME);
            glib::Propagation::Stop
        });
    }
    content_area.append(&link);

    let ok_button = dialog.add_button(text.ok_button(), gtk::ResponseType::Ok);
    ok_button.set_receives_default(true);
    dialog.set_default_widget(Some(&ok_button));
    gtk::prelude::GtkWindowExt::set_focus(&dialog, Some(&ok_button));
    dialog.set_default_response(gtk::ResponseType::Ok);

    let _ = run_dialog_blocking(&dialog);
}

fn show_gtk_error_message(parent: Option<&gtk::Window>, title: &str, message: &str) {
    let _ = run_message_dialog(
        parent,
        gtk::MessageType::Error,
        gtk::ButtonsType::Ok,
        title,
        message,
        &[],
        None,
    );
}

fn run_message_dialog(
    parent: Option<&gtk::Window>,
    message_type: gtk::MessageType,
    buttons: gtk::ButtonsType,
    title: &str,
    message: &str,
    extra_buttons: &[(&str, gtk::ResponseType)],
    default_response: Option<gtk::ResponseType>,
) -> gtk::ResponseType {
    let text = message_dialog_text(title, message);
    let dialog = gtk::MessageDialog::builder()
        .message_type(message_type)
        .buttons(buttons)
        .title(text.title)
        .text(text.primary_text)
        .modal(true)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }
    for (label, response) in extra_buttons {
        let button = dialog.add_button(label, *response);
        if Some(*response) == default_response {
            button.set_receives_default(true);
            dialog.set_default_widget(Some(&button));
            gtk::prelude::GtkWindowExt::set_focus(&dialog, Some(&button));
        }
    }
    if let Some(default_response) = default_response {
        dialog.set_default_response(default_response);
    }
    attach_message_dialog_default_key_handler(&dialog, default_response);
    run_dialog_blocking(&dialog)
}

fn attach_message_dialog_default_key_handler(
    dialog: &gtk::MessageDialog,
    default_response: Option<gtk::ResponseType>,
) {
    if default_response.is_none() {
        return;
    }
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let dialog = dialog.clone();
        controller.connect_key_pressed(move |_, key, _, _| {
            if let Some(response) = message_dialog_response_for_key(key, default_response) {
                dialog.response(response);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    dialog.add_controller(controller);
}

fn message_dialog_response_for_key(
    key: gdk::Key,
    default_response: Option<gtk::ResponseType>,
) -> Option<gtk::ResponseType> {
    match key {
        gdk::Key::Return | gdk::Key::KP_Enter => default_response,
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct MessageDialogText<'a> {
    title: &'a str,
    primary_text: &'a str,
}

fn message_dialog_text<'a>(title: &'a str, message: &'a str) -> MessageDialogText<'a> {
    MessageDialogText {
        title,
        primary_text: message,
    }
}

fn run_dialog_blocking<D>(dialog: &D) -> gtk::ResponseType
where
    D: IsA<gtk::Dialog> + IsA<gtk::Window> + Clone + 'static,
{
    let loop_ = glib::MainLoop::new(None, false);
    let response_cell = Rc::new(Cell::new(gtk::ResponseType::None));
    {
        let loop_ = loop_.clone();
        let response_cell = Rc::clone(&response_cell);
        dialog.connect_response(move |dialog, response| {
            response_cell.set(response);
            dialog.close();
            loop_.quit();
        });
    }
    dialog.present();
    loop_.run();
    response_cell.get()
}

#[derive(Clone, Copy)]
enum FileDialogMode {
    Import,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DialogFileFilterSpec<'a> {
    name: &'a str,
    patterns: &'static [&'static str],
    default: bool,
}

fn choose_text_file(
    parent: &gtk::Window,
    language: UiLanguage,
    mode: FileDialogMode,
) -> Result<Option<PathBuf>, AppError> {
    let text = ui_text(language);
    let (title, action, accept) = match mode {
        FileDialogMode::Import => (
            text.file_dialog_import_title(),
            gtk::FileChooserAction::Open,
            text.open_button(),
        ),
        FileDialogMode::Export => (
            text.file_dialog_export_title(),
            gtk::FileChooserAction::Save,
            text.save_button(),
        ),
    };

    let mut retry_path = None::<PathBuf>;
    loop {
        let dialog = gtk::FileChooserNative::new(
            Some(title),
            Some(parent),
            action,
            Some(accept),
            Some(text.cancel_button()),
        );
        for filter_spec in text_file_dialog_filter_specs(text) {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(filter_spec.name));
            for pattern in filter_spec.patterns {
                filter.add_pattern(pattern);
            }
            dialog.add_filter(&filter);
            if filter_spec.default {
                dialog.set_filter(&filter);
            }
        }
        apply_text_file_dialog_initial_path(&dialog, mode, retry_path.as_deref());

        let response = run_native_dialog_blocking(&dialog);
        if response != gtk::ResponseType::Accept {
            return Ok(None);
        }

        let path = accepted_native_dialog_path(&dialog, language)?;
        match resolve_text_file_dialog_path(parent, language, mode, path) {
            TextFileDialogPath::Accepted(path) => return Ok(Some(path)),
            TextFileDialogPath::Reopen(path) => retry_path = Some(path),
        }
    }
}

fn apply_text_file_dialog_initial_path(
    dialog: &gtk::FileChooserNative,
    mode: FileDialogMode,
    retry_path: Option<&Path>,
) {
    if let Some(path) = retry_path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            let folder = gio::File::for_path(parent);
            let _ = dialog.set_current_folder(Some(&folder));
        }
        if let Some(file_name) = path.file_name() {
            dialog.set_current_name(&file_name.to_string_lossy());
        }
    } else if let Some(current_name) = text_file_dialog_initial_name(mode) {
        dialog.set_current_name(current_name);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TextFileDialogPath {
    Accepted(PathBuf),
    Reopen(PathBuf),
}

fn resolve_text_file_dialog_path(
    parent: &gtk::Window,
    language: UiLanguage,
    mode: FileDialogMode,
    path: PathBuf,
) -> TextFileDialogPath {
    match mode {
        FileDialogMode::Import => TextFileDialogPath::Accepted(path),
        FileDialogMode::Export => export_text_file_dialog_path(path, |path| {
            should_overwrite_text_file(parent, language, path)
        }),
    }
}

fn export_text_file_dialog_path(
    path: PathBuf,
    should_overwrite: impl FnOnce(&PathBuf) -> bool,
) -> TextFileDialogPath {
    let path = ensure_text_file_extension(path);
    if should_overwrite(&path) {
        TextFileDialogPath::Accepted(path)
    } else {
        TextFileDialogPath::Reopen(path)
    }
}

fn text_file_dialog_filter_specs(text: UiText) -> [DialogFileFilterSpec<'static>; 2] {
    [
        DialogFileFilterSpec {
            name: text.file_filter_text(),
            patterns: &["*.txt"],
            default: true,
        },
        DialogFileFilterSpec {
            name: text.file_filter_all(),
            patterns: &["*"],
            default: false,
        },
    ]
}

fn text_file_dialog_initial_name(_mode: FileDialogMode) -> Option<&'static str> {
    None
}

fn should_overwrite_text_file(parent: &gtk::Window, language: UiLanguage, path: &PathBuf) -> bool {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => confirm_question(
            Some(parent),
            "j3TreeText",
            &ui_text(language).confirm_overwrite_file(&path.display().to_string()),
            language,
        ),
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => true,
    }
}

fn choose_text_folder(
    parent: &gtk::Window,
    language: UiLanguage,
) -> Result<Option<PathBuf>, AppError> {
    let dialog = gtk::FileChooserNative::new(
        Some(ui_text(language).file_dialog_export_folder_title()),
        Some(parent),
        gtk::FileChooserAction::SelectFolder,
        Some(ui_text(language).select_button()),
        Some(ui_text(language).cancel_button()),
    );
    let response = run_native_dialog_blocking(&dialog);
    if response == gtk::ResponseType::Accept {
        Ok(Some(accepted_native_dialog_path(&dialog, language)?))
    } else {
        Ok(None)
    }
}

fn accepted_native_dialog_path(
    dialog: &gtk::FileChooserNative,
    language: UiLanguage,
) -> Result<PathBuf, AppError> {
    dialog
        .file()
        .and_then(|file| file.path())
        .ok_or_else(|| AppError::user(ui_text(language).local_file_required()))
}

fn ensure_text_file_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("txt");
    }
    path
}

fn run_native_dialog_blocking<D>(dialog: &D) -> gtk::ResponseType
where
    D: IsA<gtk::NativeDialog> + Clone + 'static,
{
    let loop_ = glib::MainLoop::new(None, false);
    let response_cell = Rc::new(Cell::new(gtk::ResponseType::None));
    {
        let loop_ = loop_.clone();
        let response_cell = Rc::clone(&response_cell);
        dialog.connect_response(move |dialog, response| {
            response_cell.set(response);
            dialog.hide();
            loop_.quit();
        });
    }
    dialog.show();
    loop_.run();
    response_cell.get()
}

fn choose_editor_font(
    parent: &gtk::Window,
    language: UiLanguage,
    current: &EditorFontSettings,
) -> Option<EditorFontSettings> {
    let text = ui_text(language);
    let dialog = gtk::FontChooserDialog::new(
        Some(strip_menu_accelerator(text.editor_font()).as_str()),
        Some(parent),
    );
    dialog.set_modal(true);
    dialog.set_level(editor_font_dialog_level());
    dialog.set_font_desc(&font_description_from_settings(current));
    let response = run_dialog_blocking(&dialog);
    if response != gtk::ResponseType::Ok {
        return None;
    }
    let desc = dialog.font_desc()?;
    let family = desc.family().map(|family| family.to_string())?;
    let size_pt = editor_font_size_pt_from_pango_units(desc.size());
    Some(EditorFontSettings::new(family, size_pt))
}

fn editor_font_dialog_level() -> gtk::FontChooserLevel {
    gtk::FontChooserLevel::FAMILY | gtk::FontChooserLevel::STYLE | gtk::FontChooserLevel::SIZE
}

fn font_description_from_settings(settings: &EditorFontSettings) -> pango::FontDescription {
    let mut desc = pango::FontDescription::new();
    desc.set_family(&settings.family);
    desc.set_size(settings.size_pt * pango::SCALE);
    desc
}

fn editor_font_size_pt_from_pango_units(size: i32) -> i32 {
    let scale = i64::from(pango::SCALE);
    let size = i64::from(size);
    let rounded_up = if size <= 0 {
        1
    } else {
        (size + scale - 1) / scale
    };
    rounded_up.clamp(
        i64::from(MIN_EDITOR_FONT_SIZE_PT),
        i64::from(MAX_EDITOR_FONT_SIZE_PT),
    ) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::sqlite::SqliteDocumentRepository;
    use std::error::Error;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn ui_node(id: i64, parent_id: Option<i64>, title: &str, sort_order: i64) -> UiNode {
        UiNode {
            id,
            parent_id,
            display_parent_id: parent_id,
            title: title.to_owned(),
            sort_order,
            display_title: title.to_owned(),
            updated_at: "2026-06-16T00:00:00Z".to_owned(),
            editable: true,
            source: DocumentTabSource::ActiveTree,
            search_content_matched: false,
        }
    }

    fn active_node(id: i64, parent_id: Option<i64>, title: &str, sort_order: i64) -> Node {
        let timestamp = "2026-06-16T00:00:00Z".to_owned();
        Node {
            id,
            parent_id,
            title: title.to_owned(),
            sort_order,
            content: String::new(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            deleted_at: None,
        }
    }

    fn tree_row_spec(node_id: i64) -> TreeRowSpec {
        TreeRowSpec {
            node_id,
            depth: 0,
            title: format!("Node {node_id}"),
            display_title: format!("Node {node_id}"),
            has_children: false,
            expanded: false,
            editing: false,
        }
    }

    fn unique_test_db_path() -> PathBuf {
        let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "j3treetext-linux-gui-{pid}-{nanos}-{counter}.db",
            pid = std::process::id()
        ))
    }

    fn remove_file_if_exists(path: &Path) -> Result<(), Box<dyn Error>> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Box::new(error)),
        }
    }

    #[test]
    fn existing_current_tab_can_activate_without_content_load() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(OpenDocumentTabInput {
            node_id: 10,
            parent_id: Some(ROOT_NODE_ID),
            title: "Draft".to_owned(),
            content: "cached body".to_owned(),
            loaded_updated_at: "2026-06-16T00:00:00Z".to_owned(),
            editable: true,
            source: DocumentTabSource::ActiveTree,
        });
        let node = ui_node(10, Some(ROOT_NODE_ID), "Draft", 0);

        assert_eq!(
            existing_tab_index_without_content_load(&tabs, &node),
            Some(0)
        );
    }

    #[test]
    fn existing_dirty_tab_can_activate_without_content_load_even_when_token_changed() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(OpenDocumentTabInput {
            node_id: 10,
            parent_id: Some(ROOT_NODE_ID),
            title: "Draft".to_owned(),
            content: "cached body".to_owned(),
            loaded_updated_at: "old-token".to_owned(),
            editable: true,
            source: DocumentTabSource::ActiveTree,
        });
        tabs.update_active_content("local draft".to_owned());
        let node = ui_node(10, Some(ROOT_NODE_ID), "Draft", 0);

        assert_eq!(
            existing_tab_index_without_content_load(&tabs, &node),
            Some(0)
        );
    }

    #[test]
    fn existing_stale_clean_tab_keeps_full_content_load_path() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(OpenDocumentTabInput {
            node_id: 10,
            parent_id: Some(ROOT_NODE_ID),
            title: "Draft".to_owned(),
            content: "cached body".to_owned(),
            loaded_updated_at: "old-token".to_owned(),
            editable: true,
            source: DocumentTabSource::ActiveTree,
        });
        let node = ui_node(10, Some(ROOT_NODE_ID), "Draft", 0);

        assert_eq!(existing_tab_index_without_content_load(&tabs, &node), None);
    }

    fn visible_ids(document: &UiDocument, expanded_node_ids: &[i64], expand_all: bool) -> Vec<i64> {
        let expanded_node_ids = expanded_node_ids.iter().copied().collect::<HashSet<_>>();
        document
            .visible_order(&expanded_node_ids, expand_all)
            .into_iter()
            .map(|(index, _)| document.nodes[index].id)
            .collect()
    }

    fn visible_depths(
        document: &UiDocument,
        expanded_node_ids: &[i64],
        expand_all: bool,
    ) -> Vec<usize> {
        let expanded_node_ids = expanded_node_ids.iter().copied().collect::<HashSet<_>>();
        document
            .visible_order(&expanded_node_ids, expand_all)
            .into_iter()
            .map(|(_, depth)| depth)
            .collect()
    }

    fn search_ui_node(title: &str, content_matched: bool) -> UiNode {
        UiNode {
            source: DocumentTabSource::SearchResult,
            search_content_matched: content_matched,
            ..ui_node(1, None, title, 0)
        }
    }

    fn gtk_action_name_from_detailed_action(detailed_action: &str) -> String {
        let action = detailed_action
            .strip_prefix("win.")
            .unwrap_or(detailed_action);
        action
            .split_once("::")
            .map_or(action, |(name, _)| name)
            .to_owned()
    }

    fn detailed_action_has_target(detailed_action: &str) -> bool {
        detailed_action
            .strip_prefix("win.")
            .unwrap_or(detailed_action)
            .contains("::")
    }

    fn rust_function_body<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("fn {name}");
        let signature_start = source
            .find(&signature)
            .unwrap_or_else(|| panic!("function `{name}` must exist"));
        let body_start = source[signature_start..]
            .find('{')
            .map(|offset| signature_start + offset + 1)
            .unwrap_or_else(|| panic!("function `{name}` must have a body"));

        let mut depth = 1usize;
        for (offset, character) in source[body_start..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[body_start..body_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("function `{name}` body must be closed");
    }

    fn insert_menu_action_requirement(
        requirements: &mut HashMap<String, bool>,
        detailed_action: &str,
    ) {
        let action = gtk_action_name_from_detailed_action(detailed_action);
        let has_target = detailed_action_has_target(detailed_action);
        if let Some(previous) = requirements.insert(action.clone(), has_target) {
            assert_eq!(
                previous, has_target,
                "GTK action `{action}` must not be used both with and without a target"
            );
        }
    }

    fn collect_menu_entry_action_requirements(
        entries: &[GuiMenuEntry],
        requirements: &mut HashMap<String, bool>,
    ) {
        for entry in entries {
            match *entry {
                GuiMenuEntry::Command(command) => {
                    insert_menu_action_requirement(requirements, command.gtk_detailed_action());
                }
                GuiMenuEntry::OptionMenu(option_menu) => {
                    collect_option_menu_action_requirements(option_menu, requirements);
                }
                GuiMenuEntry::Separator => {}
            }
        }
    }

    fn collect_option_menu_action_requirements(
        option_menu: GuiOptionMenu,
        requirements: &mut HashMap<String, bool>,
    ) {
        match option_menu {
            GuiOptionMenu::ImportEncoding => {
                for encoding in TextEncoding::import_options() {
                    if let Some(action) = option_menu.gtk_detailed_action_for_encoding(*encoding) {
                        insert_menu_action_requirement(requirements, &action);
                    }
                }
            }
            GuiOptionMenu::ExportEncoding => {
                for encoding in TextEncoding::export_options() {
                    if let Some(action) = option_menu.gtk_detailed_action_for_encoding(*encoding) {
                        insert_menu_action_requirement(requirements, &action);
                    }
                }
            }
            GuiOptionMenu::Theme => {
                for theme in AppearanceTheme::options() {
                    if let Some(action) = option_menu.gtk_detailed_action_for_theme(*theme) {
                        insert_menu_action_requirement(requirements, &action);
                    }
                }
            }
            GuiOptionMenu::Language => {
                for language in UiLanguage::options() {
                    if let Some(action) = option_menu.gtk_detailed_action_for_language(*language) {
                        insert_menu_action_requirement(requirements, &action);
                    }
                }
            }
        }
    }

    fn menu_accelerator(label: &str) -> Option<&str> {
        label.split_once('\t').map(|(_, accelerator)| accelerator)
    }

    fn menu_accelerator_for_gtk_accelerator(accelerator: &str) -> Option<String> {
        let (primary, key) = match accelerator.strip_prefix("<Primary>") {
            Some(key) => (true, key),
            None => (false, accelerator),
        };
        let key = match key {
            "Return" => "Enter",
            "s" | "S" => "S",
            "n" | "N" => "N",
            "w" | "W" => "W",
            "f" | "F" => "F",
            "h" | "H" => "H",
            "a" | "A" => "A",
            "Up" => "Up",
            "Down" => "Down",
            "F2" => "F2",
            "Delete" => "Delete",
            _ => return None,
        };

        Some(if primary {
            format!("Ctrl+{key}")
        } else {
            key.to_owned()
        })
    }

    fn command_shortcut_menu_accelerators(command: GuiCommand) -> HashSet<String> {
        GUI_SHORTCUTS
            .iter()
            .filter(|shortcut| shortcut.command == command)
            .filter_map(|shortcut| menu_accelerator_for_gtk_accelerator(shortcut.accelerator))
            .collect()
    }

    fn connected_action_binding_names(actions: &AppActions) -> HashSet<String> {
        let mut names = HashSet::new();

        macro_rules! collect_simple_binding {
            ($field:ident, $command:expr, $handler:path) => {{
                let _handler: fn(
                    &std::rc::Rc<std::cell::RefCell<LinuxState>>,
                ) -> Result<(), AppError> = $handler;
                let action_name = actions.$field.name().to_string();
                assert_eq!(action_name, $command.gtk_action_name());
                assert!(
                    names.insert(action_name),
                    "simple action binding names must be unique"
                );
            }};
        }
        for_each_simple_action_binding!(collect_simple_binding);

        macro_rules! collect_stateful_string_binding {
            ($field:ident, $action_name:expr, $handler:path) => {{
                let _handler: fn(
                    &std::rc::Rc<std::cell::RefCell<LinuxState>>,
                    &str,
                ) -> Result<(), AppError> = $handler;
                let registered_name = actions.$field.name().to_string();
                assert_eq!(registered_name, $action_name);
                assert!(
                    names.insert(registered_name),
                    "stateful string action binding names must be unique"
                );
            }};
        }
        for_each_stateful_string_action_binding!(collect_stateful_string_binding);

        macro_rules! collect_bool_stateful_binding {
            ($field:ident, $command:expr, $handler:path) => {{
                let _handler: fn(
                    &std::rc::Rc<std::cell::RefCell<LinuxState>>,
                ) -> Result<(), AppError> = $handler;
                let action_name = actions.$field.name().to_string();
                assert_eq!(action_name, $command.gtk_action_name());
                assert!(
                    names.insert(action_name),
                    "stateful bool action binding names must be unique"
                );
            }};
        }
        for_each_bool_stateful_action_binding!(collect_bool_stateful_binding);

        names
    }

    fn simple_action_binding_handlers() -> HashMap<GuiCommand, &'static str> {
        let mut handlers = HashMap::new();

        macro_rules! collect_simple_binding {
            ($field:ident, $command:expr, $handler:path) => {{
                assert!(
                    handlers.insert($command, stringify!($handler)).is_none(),
                    "simple action handlers must be unique per command"
                );
            }};
        }
        for_each_simple_action_binding!(collect_simple_binding);

        handlers
    }

    fn menu_model_attribute_string(
        model: &gio::MenuModel,
        index: i32,
        attribute: &str,
    ) -> Option<String> {
        model
            .item_attribute_value(index, attribute, None::<&glib::VariantTy>)
            .and_then(|value| value.str().map(|value| value.to_string()))
    }

    fn menu_model_link(model: &gio::MenuModel, index: i32, link: &str) -> gio::MenuModel {
        model
            .item_link(index, link)
            .unwrap_or_else(|| panic!("menu item {index} must include `{link}` link"))
    }

    fn menu_entry_sections(entries: &[GuiMenuEntry]) -> Vec<&[GuiMenuEntry]> {
        let mut sections = Vec::new();
        let mut start = 0;
        for (index, entry) in entries.iter().enumerate() {
            if *entry == GuiMenuEntry::Separator {
                if start < index {
                    sections.push(&entries[start..index]);
                }
                start = index + 1;
            }
        }
        if start < entries.len() {
            sections.push(&entries[start..]);
        }
        sections
    }

    fn assert_menu_model_action(
        model: &gio::MenuModel,
        index: i32,
        expected_detailed_action: &str,
    ) {
        let (expected_action, expected_target) = expected_detailed_action
            .split_once("::")
            .map_or((expected_detailed_action, None), |(action, target)| {
                (action, Some(target))
            });
        assert_eq!(
            menu_model_attribute_string(model, index, "action").as_deref(),
            Some(expected_action)
        );
        assert_eq!(
            menu_model_attribute_string(model, index, "target").as_deref(),
            expected_target
        );
    }

    fn assert_option_menu_model(model: &gio::MenuModel, option_menu: GuiOptionMenu, text: UiText) {
        match option_menu {
            GuiOptionMenu::ImportEncoding => {
                let options = TextEncoding::import_options();
                assert_eq!(model.n_items(), options.len() as i32);
                for (index, encoding) in options.iter().enumerate() {
                    let index = index as i32;
                    assert_eq!(
                        menu_model_attribute_string(model, index, "label").as_deref(),
                        Some(text.text_encoding_name(*encoding))
                    );
                    let action = option_menu
                        .gtk_detailed_action_for_encoding(*encoding)
                        .expect("import encoding menu entry should have an action");
                    assert_menu_model_action(model, index, &action);
                }
            }
            GuiOptionMenu::ExportEncoding => {
                let options = TextEncoding::export_options();
                assert_eq!(model.n_items(), options.len() as i32);
                for (index, encoding) in options.iter().enumerate() {
                    let index = index as i32;
                    assert_eq!(
                        menu_model_attribute_string(model, index, "label").as_deref(),
                        Some(text.text_encoding_name(*encoding))
                    );
                    let action = option_menu
                        .gtk_detailed_action_for_encoding(*encoding)
                        .expect("export encoding menu entry should have an action");
                    assert_menu_model_action(model, index, &action);
                }
            }
            GuiOptionMenu::Theme => {
                let options = AppearanceTheme::options();
                assert_eq!(model.n_items(), options.len() as i32);
                for (index, theme) in options.iter().enumerate() {
                    let index = index as i32;
                    assert_eq!(
                        menu_model_attribute_string(model, index, "label").as_deref(),
                        Some(text.theme_name(*theme))
                    );
                    let action = option_menu
                        .gtk_detailed_action_for_theme(*theme)
                        .expect("theme menu entry should have an action");
                    assert_menu_model_action(model, index, &action);
                }
            }
            GuiOptionMenu::Language => {
                let options = UiLanguage::options();
                assert_eq!(model.n_items(), options.len() as i32);
                for (index, language) in options.iter().enumerate() {
                    let index = index as i32;
                    assert_eq!(
                        menu_model_attribute_string(model, index, "label").as_deref(),
                        Some(language.display_name())
                    );
                    let action = option_menu
                        .gtk_detailed_action_for_language(*language)
                        .expect("language menu entry should have an action");
                    assert_menu_model_action(model, index, &action);
                }
            }
        }
    }

    fn assert_menu_entries_model(model: &gio::MenuModel, entries: &[GuiMenuEntry], text: UiText) {
        let sections = menu_entry_sections(entries);
        assert_eq!(model.n_items(), sections.len() as i32);
        for (section_index, section_entries) in sections.iter().enumerate() {
            let section = menu_model_link(model, section_index as i32, "section");
            assert_eq!(section.n_items(), section_entries.len() as i32);
            for (entry_index, entry) in section_entries.iter().enumerate() {
                let entry_index = entry_index as i32;
                match *entry {
                    GuiMenuEntry::Command(command) => {
                        assert_eq!(
                            menu_model_attribute_string(&section, entry_index, "label").as_deref(),
                            Some(command_label(text, command))
                        );
                        assert_menu_model_action(
                            &section,
                            entry_index,
                            command.gtk_detailed_action(),
                        );
                    }
                    GuiMenuEntry::OptionMenu(option_menu) => {
                        assert_eq!(
                            menu_model_attribute_string(&section, entry_index, "label").as_deref(),
                            Some(option_menu_label(text, option_menu))
                        );
                        let submenu = menu_model_link(&section, entry_index, "submenu");
                        assert_option_menu_model(&submenu, option_menu, text);
                    }
                    GuiMenuEntry::Separator => unreachable!("sections do not include separators"),
                }
            }
        }
    }

    #[test]
    fn find_match_highlight_palette_matches_win32_baseline() {
        let expected = [
            (AppearanceTheme::Light, "#ffe680"),
            (AppearanceTheme::ClassicDark, "#5b4a19"),
            (AppearanceTheme::SepiaTeal, "#53521f"),
            (AppearanceTheme::Graphite, "#59501f"),
            (AppearanceTheme::Forest, "#3e582a"),
            (AppearanceTheme::SteelBlue, "#425871"),
        ];

        for (theme, color) in expected {
            assert_eq!(ThemePalette::for_theme(theme).find_match_bg, color);
        }
    }

    #[test]
    fn apply_theme_refreshes_current_find_match_highlight_like_win32() {
        let body = rust_function_body(include_str!("linux.rs"), "apply_theme");
        assert!(
            body.contains("refresh_find_match_highlight_tags(state, theme);"),
            "theme changes must recolor the current find highlight like Win32 re-applies the active match"
        );
    }

    #[test]
    fn find_match_highlight_tag_updates_existing_tag_color() {
        let tag_body = rust_function_body(include_str!("linux.rs"), "find_match_highlight_tag");
        assert!(
            tag_body.contains("configure_find_match_highlight_tag(&tag, background);"),
            "reusing the existing find-match tag must refresh its color for the current theme"
        );

        let refresh_body = rust_function_body(
            include_str!("linux.rs"),
            "refresh_find_match_highlight_tags",
        );
        assert!(
            refresh_body.contains("configure_find_match_highlight_tag(&tag, background);"),
            "theme changes must update already-applied find-match tags"
        );
    }

    #[test]
    fn editor_sync_clears_find_match_highlight_like_win32_store() {
        let body = rust_function_body(include_str!("linux.rs"), "sync_active_editor_content");
        assert!(
            body.contains("clear_find_match_highlight(&page.buffer);"),
            "active editor sync must clear the temporary find highlight like Win32 store_editor_content_in_active_tab"
        );
    }

    #[test]
    fn editor_change_defers_full_text_sync_like_win32() {
        let body = rust_function_body(include_str!("linux.rs"), "handle_editor_changed_for_page");
        assert!(
            body.contains("state.editor_content_pending_sync = true;"),
            "GTK editor changes must mark pending sync instead of copying the full buffer"
        );
        assert!(
            body.contains("state.tabs.mark_active_dirty_from_view();"),
            "GTK editor changes must still mark the active tab dirty immediately"
        );
        assert!(
            !body.contains("sync_active_editor_content(state)"),
            "GTK editor changes must not synchronously copy the full buffer"
        );
        assert!(
            !body.contains("buffer_text("),
            "GTK editor changes must not read the full buffer on every keystroke"
        );
    }

    #[test]
    fn editor_sync_reads_full_text_only_when_pending() {
        let body = rust_function_body(include_str!("linux.rs"), "sync_active_editor_content");
        assert!(
            body.contains(
                "let pending_editor_content_sync = state.borrow().editor_content_pending_sync;"
            ) && body.contains("if pending_editor_content_sync {"),
            "active editor sync must branch on the pending content flag"
        );
        assert!(
            body.contains("let mut content = buffer_text(&page.buffer);"),
            "active editor sync must still read the live buffer when content is pending"
        );
        assert!(
            body.contains("state.editor_content_pending_sync = false;"),
            "successful sync must clear the pending content flag"
        );

        let refresh_body = rust_function_body(include_str!("linux.rs"), "refresh_tabs");
        assert!(
            refresh_body.contains("if state.borrow().editor_content_pending_sync {")
                && refresh_body.contains("sync_active_editor_content(state)?;"),
            "tab refresh must flush pending live editor text before rebuilding buffers"
        );
    }

    #[test]
    fn find_replace_dialog_close_clears_find_match_highlight_like_win32() {
        let body = rust_function_body(include_str!("linux.rs"), "clear_find_replace_dialog");
        assert!(
            body.contains("clear_active_find_match_highlight(state);"),
            "closing the find/replace dialog must clear the active match like Win32 FR_DIALOGTERM"
        );
    }

    #[test]
    fn switching_find_replace_dialog_kind_clears_find_match_highlight_like_win32() {
        let body = rust_function_body(include_str!("linux.rs"), "open_find_replace_dialog");
        assert!(
            body.contains("ExistingFindReplaceDialogAction::Close(dialog)")
                && body.contains("clear_active_find_match_highlight(state);"),
            "replacing an existing find/replace dialog must clear the active match like Win32 destroys the previous common dialog"
        );
    }

    #[test]
    fn theme_action_restores_current_find_highlight_like_win32() {
        let body = rust_function_body(include_str!("linux.rs"), "theme_action");
        assert!(
            body.contains(
                "let current_find_match = active_find_match_selection_byte_range(state);"
            ),
            "theme changes must remember the active find highlight before syncing editor content"
        );
        assert!(
            body.contains("restore_active_find_match_highlight(state, current_find_match);"),
            "theme changes must restore the active find highlight after applying the visual setting"
        );
    }

    #[test]
    fn editor_font_action_clears_find_highlight_like_win32_editor_reload() {
        let body = rust_function_body(include_str!("linux.rs"), "editor_font_action");
        assert!(
            body.contains("sync_active_editor_content(state)?;"),
            "font changes must store the active editor before reloading it like Win32"
        );
        assert!(
            !body.contains("restore_active_find_match_highlight(state,"),
            "font changes reload the active editor on Win32, so Linux must not restore temporary find highlight"
        );
    }

    #[test]
    fn editor_reload_boundaries_reset_gtk_undo_like_win32() {
        for handler in [
            "open_or_activate_tab_from_node",
            "handle_tab_switched",
            "toggle_word_wrap_action",
            "editor_font_action",
            "autosave_all_dirty_tabs",
            "resolve_dirty_tabs_for_nodes",
            "close_tab_at_index",
        ] {
            let body = rust_function_body(include_str!("linux.rs"), handler);
            assert!(
                body.contains("reset_active_editor_undo_stack(state);"),
                "{handler} must clear GTK undo at Win32 active-editor reload boundaries"
            );
        }

        let update_body = rust_function_body(include_str!("linux.rs"), "update_existing_tab_page");
        assert!(
            update_body.contains("reset_text_buffer_undo_stack(&page.buffer);"),
            "programmatic content reload into an existing GTK tab must clear undo like Win32 editor reload"
        );
        assert!(
            update_body.contains("let editor_access_changed =")
                && update_body.contains("if editor_access_changed {"),
            "read-only/editable transitions must clear undo like Win32 active editor reload"
        );

        let reset_body =
            rust_function_body(include_str!("linux.rs"), "reset_text_buffer_undo_stack");
        assert!(
            reset_body.contains("buffer.set_enable_undo(false);")
                && reset_body.contains("buffer.set_enable_undo(true);"),
            "GTK undo reset must rebuild the TextBuffer undo manager without changing text"
        );
    }

    #[test]
    fn editor_font_resolution_keeps_available_requested_font() {
        let requested = EditorFontSettings::new("JetBrains Mono", 14);

        let resolved =
            resolve_editor_font_settings(&requested, |family| family == "JetBrains Mono");

        assert_eq!(resolved.settings, requested);
        assert!(!resolved.used_fallback);
    }

    #[test]
    fn editor_font_resolution_falls_back_to_default_when_requested_family_is_missing() {
        let requested = EditorFontSettings::new("Unavailable Font", 14);

        let resolved = resolve_editor_font_settings(&requested, |_| false);

        assert_eq!(resolved.settings, EditorFontSettings::default());
        assert!(resolved.used_fallback);
    }

    #[test]
    fn editor_font_resolution_keeps_default_request_even_when_default_family_is_missing() {
        let requested = EditorFontSettings::default();

        let resolved = resolve_editor_font_settings(&requested, |_| false);

        assert_eq!(resolved.settings, requested);
        assert!(!resolved.used_fallback);
    }

    #[test]
    fn ui_settings_resolve_editor_font_keeps_other_setting_changes_in_single_update() {
        let mut settings = UiSettings::default();
        settings.appearance.set_theme(AppearanceTheme::Forest);
        settings.editor_font = EditorFontSettings::new("Unavailable Font", 14);

        let (resolved, used_fallback) = ui_settings_with_resolved_editor_font(settings, |_| false);

        assert_eq!(resolved.appearance.theme, AppearanceTheme::Forest);
        assert_eq!(resolved.editor_font, EditorFontSettings::default());
        assert!(used_fallback);
    }

    #[test]
    fn editor_font_dialog_level_matches_win32_choosefont_scope() {
        let level = editor_font_dialog_level();

        assert!(level.contains(gtk::FontChooserLevel::FAMILY));
        assert!(level.contains(gtk::FontChooserLevel::STYLE));
        assert!(level.contains(gtk::FontChooserLevel::SIZE));
        assert!(!level.contains(gtk::FontChooserLevel::VARIATIONS));
        assert!(!level.contains(gtk::FontChooserLevel::FEATURES));
    }

    #[test]
    fn editor_font_pango_size_rounds_up_like_win32_dialog_tenths() {
        assert_eq!(editor_font_size_pt_from_pango_units(9 * pango::SCALE), 9);
        assert_eq!(
            editor_font_size_pt_from_pango_units(9 * pango::SCALE + 1),
            10
        );
        assert_eq!(
            editor_font_size_pt_from_pango_units(9 * pango::SCALE + pango::SCALE / 2),
            10
        );
        assert_eq!(
            editor_font_size_pt_from_pango_units(MAX_EDITOR_FONT_SIZE_PT * pango::SCALE + 1),
            MAX_EDITOR_FONT_SIZE_PT
        );
        assert_eq!(
            editor_font_size_pt_from_pango_units(0),
            MIN_EDITOR_FONT_SIZE_PT
        );
    }

    #[test]
    fn font_family_name_matching_is_case_insensitive() {
        assert!(font_family_names_match("Noto Sans Mono", "noto sans mono"));
    }

    #[test]
    fn menu_accelerator_labels_match_registered_shortcut_contract() {
        let text = ui_text(UiLanguage::English);
        for command in GuiCommand::ALL {
            let Some(accelerator) = menu_accelerator(command_label(text, command)) else {
                continue;
            };
            let registered = command_shortcut_menu_accelerators(command);
            assert!(
                registered.contains(accelerator),
                "menu label for {command:?} advertises `{accelerator}`, but registered shortcuts are {registered:?}"
            );
        }
    }

    #[test]
    fn global_accelerator_registration_groups_multiple_shortcuts_per_action() {
        let bindings = global_accelerator_bindings()
            .into_iter()
            .collect::<HashMap<_, _>>();

        assert_eq!(
            bindings
                .get(GuiCommand::SaveDocument.gtk_detailed_action())
                .map(Vec::as_slice),
            Some(&["<Primary>s", "<Primary><Alt>s"][..])
        );
        assert_eq!(
            bindings
                .get(GuiCommand::NewDocument.gtk_detailed_action())
                .map(Vec::as_slice),
            Some(&["<Primary>n", "<Primary><Alt>n"][..])
        );
        assert_eq!(
            bindings
                .get(GuiCommand::CloseTab.gtk_detailed_action())
                .map(Vec::as_slice),
            Some(&["<Primary>w", "<Primary><Alt>w"][..])
        );
        assert_eq!(
            bindings
                .get(GuiCommand::FindText.gtk_detailed_action())
                .map(Vec::as_slice),
            Some(&["<Primary>f", "<Primary><Alt>f"][..])
        );
        assert_eq!(
            bindings
                .get(GuiCommand::ReplaceText.gtk_detailed_action())
                .map(Vec::as_slice),
            Some(&["<Primary>h", "<Primary><Alt>h"][..])
        );
        assert!(!bindings.contains_key(GuiCommand::SelectAll.gtk_detailed_action()));
    }

    #[test]
    fn linux_menu_model_matches_main_menu_contract_order_labels_and_actions() {
        for language in UiLanguage::options() {
            let text = ui_text(*language);
            let model = build_menu_model(*language).upcast::<gio::MenuModel>();
            assert_eq!(model.n_items(), MAIN_MENU_SPECS.len() as i32);

            for (index, spec) in MAIN_MENU_SPECS.iter().enumerate() {
                let index = index as i32;
                assert_eq!(
                    menu_model_attribute_string(&model, index, "label").as_deref(),
                    Some(menu_label(text, spec.kind))
                );
                let submenu = menu_model_link(&model, index, "submenu");
                assert_menu_entries_model(&submenu, spec.entries, text);
            }
        }
    }

    #[test]
    fn linux_context_menu_models_match_context_contract_labels_and_actions() {
        for language in UiLanguage::options() {
            let text = ui_text(*language);
            for entries in [TREE_CONTEXT_MENU_ENTRIES, EDITOR_CONTEXT_MENU_ENTRIES] {
                let model = build_menu_entries(entries, text).upcast::<gio::MenuModel>();
                assert_menu_entries_model(&model, entries, text);
            }
        }
    }

    #[test]
    fn yes_no_buttons_use_ui_language_in_win32_message_box_order() {
        let korean_buttons = yes_no_buttons(ui_text(UiLanguage::Korean));
        assert_eq!(
            korean_buttons
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            vec!["예", "아니요"]
        );
        assert_eq!(
            korean_buttons
                .iter()
                .map(|(_, response)| *response)
                .collect::<Vec<_>>(),
            vec![gtk::ResponseType::Yes, gtk::ResponseType::No]
        );

        let english_buttons = yes_no_buttons(ui_text(UiLanguage::English));
        assert_eq!(
            english_buttons
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            vec!["_Yes", "_No"]
        );
        assert_eq!(WIN32_QUESTION_DEFAULT_RESPONSE, gtk::ResponseType::Yes);
    }

    #[test]
    fn message_dialog_return_keys_activate_win32_default_response() {
        assert_eq!(
            message_dialog_response_for_key(gdk::Key::Return, Some(gtk::ResponseType::Yes)),
            Some(gtk::ResponseType::Yes)
        );
        assert_eq!(
            message_dialog_response_for_key(gdk::Key::KP_Enter, Some(gtk::ResponseType::Yes)),
            Some(gtk::ResponseType::Yes)
        );
        assert_eq!(
            message_dialog_response_for_key(gdk::Key::Escape, Some(gtk::ResponseType::Yes)),
            None
        );
        assert_eq!(
            message_dialog_response_for_key(gdk::Key::Return, None),
            None
        );
    }

    #[test]
    fn message_dialog_keeps_win32_modal_owner_and_default_button_contract() {
        let body = rust_function_body(include_str!("linux.rs"), "run_message_dialog");

        assert!(
            body.contains(".modal(true)")
                && body.contains("dialog.set_transient_for(Some(parent));"),
            "GTK message dialogs must stay modal and parented like Win32 owner MessageBox calls"
        );
        assert!(
            body.contains("button.set_receives_default(true);")
                && body.contains("dialog.set_default_widget(Some(&button));")
                && body.contains("gtk::prelude::GtkWindowExt::set_focus(&dialog, Some(&button));")
                && body.contains("dialog.set_default_response(default_response);")
                && body.contains(
                    "attach_message_dialog_default_key_handler(&dialog, default_response);"
                ),
            "GTK confirmation dialogs must keep the Win32 default-button and Return-key behavior"
        );
    }

    #[test]
    fn yes_no_cancel_buttons_use_ui_language_without_changing_responses() {
        let korean_buttons = yes_no_cancel_buttons(ui_text(UiLanguage::Korean));
        assert_eq!(
            korean_buttons
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            vec!["예", "아니요", "취소"]
        );
        assert_eq!(
            korean_buttons
                .iter()
                .map(|(_, response)| *response)
                .collect::<Vec<_>>(),
            vec![
                gtk::ResponseType::Yes,
                gtk::ResponseType::No,
                gtk::ResponseType::Cancel
            ]
        );

        let english_buttons = yes_no_cancel_buttons(ui_text(UiLanguage::English));
        assert_eq!(
            english_buttons
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            vec!["_Yes", "_No", "_Cancel"]
        );
    }

    #[test]
    fn blocking_dialog_responses_match_win32_message_box_decisions() {
        assert_eq!(
            save_conflict_decision_from_response(gtk::ResponseType::Yes),
            ConflictDecision::Reload
        );
        assert_eq!(
            save_conflict_decision_from_response(gtk::ResponseType::No),
            ConflictDecision::SaveAsNewDocument
        );
        assert_eq!(
            save_conflict_decision_from_response(gtk::ResponseType::Cancel),
            ConflictDecision::Cancel
        );
        assert_eq!(
            save_conflict_decision_from_response(gtk::ResponseType::DeleteEvent),
            ConflictDecision::Cancel
        );

        assert_eq!(
            dirty_tab_decision_from_response(gtk::ResponseType::Yes),
            DirtyTabDecision::Save
        );
        assert_eq!(
            dirty_tab_decision_from_response(gtk::ResponseType::No),
            DirtyTabDecision::Discard
        );
        assert_eq!(
            dirty_tab_decision_from_response(gtk::ResponseType::Cancel),
            DirtyTabDecision::Cancel
        );
        assert_eq!(
            dirty_tab_decision_from_response(gtk::ResponseType::None),
            DirtyTabDecision::Cancel
        );
    }

    #[test]
    fn linux_app_error_messages_match_win32_common_copy() {
        let read_error = AppError::io_with_user_message(
            "read text file",
            IoUserMessage::ReadTextFile,
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        );
        assert_eq!(
            app_error_user_message(&read_error, UiLanguage::English),
            "Cannot read the text file. Check the path and permissions."
        );

        let save_error = AppError::sqlite_with_user_message(
            "save document",
            SqliteUserMessage::SaveDocumentContent,
            rusqlite::Error::InvalidQuery,
        );
        assert_eq!(
            app_error_user_message(&save_error, UiLanguage::Korean),
            "문서를 저장할 수 없습니다. DB 파일 권한과 디스크 용량을 확인하세요."
        );

        let domain_error = AppError::from(DomainError::CannotMoveRoot);
        assert_eq!(
            app_error_user_message(&domain_error, UiLanguage::English),
            "The root document cannot be moved."
        );

        let encode_error = AppError::text_encoding_with_user_message(
            "export text",
            TextEncodingUserMessage::Encode,
            TextEncoding::Windows1252,
            "unmappable character",
        );
        assert_eq!(
            app_error_user_message(&encode_error, UiLanguage::English),
            "Some characters cannot be saved with the selected encoding (Windows-1252).\nExport with UTF-8 or UTF-16."
        );
    }

    #[test]
    fn linux_platform_error_messages_use_linux_or_desktop_copy() {
        let generic = AppError::platform("handle menu command", "window state was not attached");
        assert_eq!(
            app_error_user_message(&generic, UiLanguage::English),
            "Linux UI error. Try again."
        );

        let win32_startup = AppError::platform_with_user_message(
            "create main window",
            PlatformUserMessage::Win32Startup,
            "Win32 error code 1",
        );
        assert_eq!(
            app_error_user_message(&win32_startup, UiLanguage::English),
            "Could not start the desktop UI."
        );
    }

    #[test]
    fn linux_action_handler_bindings_cover_every_registered_action() {
        let actions = AppActions::build();
        let registered_names = actions
            .all()
            .iter()
            .map(|action| action.name().to_string())
            .collect::<HashSet<_>>();

        assert_eq!(connected_action_binding_names(&actions), registered_names);
    }

    #[test]
    fn close_window_menu_action_keeps_close_window_handler_binding() {
        let handlers = simple_action_binding_handlers();
        assert_eq!(
            handlers.get(&GuiCommand::CloseWindow).copied(),
            Some("close_window_action")
        );
    }

    #[test]
    fn active_tree_mutation_handlers_keep_search_and_trash_guard() {
        let source = include_str!("linux.rs");
        for handler in [
            "new_document_action",
            "new_child_document_action",
            "rename_action",
            "move_selected_node_within_parent",
            "delete_node_action",
            "move_node_by_drop",
        ] {
            let body = rust_function_body(source, handler);
            assert!(
                body.contains("ensure_active_tree_browse_mode(state)?"),
                "{handler} must reject trash/search contexts like the Win32 tree command path"
            );
        }
    }

    #[test]
    fn new_document_actions_use_memory_document_for_initial_label_edit() {
        let source = include_str!("linux.rs");
        for handler in ["new_document_action", "new_child_document_action"] {
            let body = rust_function_body(source, handler);
            assert!(
                body.contains("state.borrow_mut().app.create_document(parent_id)?")
                    && body.contains("reload_visible_document_for_label_edit(state, node_id)?"),
                "{handler} must refresh the UI from the committed in-memory document before editing"
            );
            assert!(
                !body.contains("reload_active_document_and_tree")
                    && !body.contains("start_label_edit(state, node_id)"),
                "{handler} must not reload SQLite or rebuild the tree twice after creating a node"
            );
        }

        let helper_body = rust_function_body(source, "reload_visible_document_for_label_edit");
        assert!(
            helper_body.contains("state.borrow_mut().editing_node_id = Some(node_id);")
                && helper_body.contains("reload_visible_document(state, Some(node_id))")
                && !helper_body.contains("reload_active_document_and_tree")
                && !helper_body.contains("rebuild_tree_list"),
            "created-node label edit must be folded into the single visible document refresh"
        );
    }

    #[test]
    fn linux_action_enabled_states_match_win32_menu_state_contract() {
        let actions = AppActions::build();
        for action in actions.all() {
            action.set_enabled(false);
        }

        let command_availability = GuiCommandAvailability {
            save_enabled: true,
            close_tab_enabled: false,
            new_child_document_enabled: true,
            new_document_enabled: false,
            rename_enabled: true,
            move_up_enabled: false,
            move_down_enabled: true,
            delete_enabled: false,
            restore_enabled: true,
            delete_permanently_enabled: false,
            active_tree_checked: true,
            trash_checked: false,
        };
        let editor_availability = GuiEditorAvailability {
            undo_enabled: false,
            cut_enabled: true,
            copy_enabled: false,
            paste_enabled: true,
            delete_enabled: false,
            select_all_enabled: true,
            find_replace_enabled: false,
        };

        apply_action_enabled_states(&actions, command_availability, editor_availability);

        assert!(actions.save.is_enabled());
        assert!(actions.import_text.is_enabled());
        assert!(actions.export_text.is_enabled());
        assert!(actions.export_all_text.is_enabled());
        assert!(!actions.close_tab.is_enabled());
        assert!(actions.close_window.is_enabled());
        assert!(!actions.undo.is_enabled());
        assert!(actions.cut.is_enabled());
        assert!(!actions.copy.is_enabled());
        assert!(actions.paste.is_enabled());
        assert!(!actions.delete_selection.is_enabled());
        assert!(actions.select_all.is_enabled());
        assert!(!actions.find.is_enabled());
        assert!(!actions.replace.is_enabled());
        assert!(!actions.new_document.is_enabled());
        assert!(actions.new_child_document.is_enabled());
        assert!(actions.rename.is_enabled());
        assert!(!actions.move_up.is_enabled());
        assert!(actions.move_down.is_enabled());
        assert!(!actions.delete.is_enabled());
        assert!(actions.restore.is_enabled());
        assert!(!actions.delete_permanently.is_enabled());
        assert!(actions.tree_mode.is_enabled());
        assert!(actions.import_encoding.is_enabled());
        assert!(actions.export_encoding.is_enabled());
        assert!(actions.theme.is_enabled());
        assert!(actions.language.is_enabled());
        assert!(actions.word_wrap.is_enabled());
        assert!(actions.editor_font.is_enabled());
        assert!(actions.about.is_enabled());
    }

    #[test]
    fn linux_registered_actions_cover_all_menu_references() {
        let actions = AppActions::build();
        let all_actions = actions.all();
        let registered_names = all_actions
            .iter()
            .map(|action| action.name().to_string())
            .collect::<HashSet<_>>();
        assert_eq!(
            registered_names.len(),
            all_actions.len(),
            "Linux GTK action registry must not contain duplicate names"
        );

        let mut requirements = HashMap::new();
        for spec in MAIN_MENU_SPECS {
            collect_menu_entry_action_requirements(spec.entries, &mut requirements);
        }
        collect_menu_entry_action_requirements(TREE_CONTEXT_MENU_ENTRIES, &mut requirements);
        collect_menu_entry_action_requirements(EDITOR_CONTEXT_MENU_ENTRIES, &mut requirements);

        for action_name in &registered_names {
            assert!(
                requirements.contains_key(action_name),
                "GTK action `{action_name}` is registered but is not referenced by any menu contract"
            );
        }
        for (action_name, has_target) in &requirements {
            assert!(
                registered_names.contains(action_name),
                "menu references GTK action `{action_name}` but AppActions does not register it"
            );
            let action = all_actions
                .iter()
                .find(|action| action.name().as_str() == action_name.as_str())
                .expect("registered action should be present");
            let parameter_type = action
                .parameter_type()
                .map(|parameter_type| parameter_type.to_string());
            if *has_target {
                assert_eq!(
                    parameter_type.as_deref(),
                    Some("s"),
                    "targeted menu action `{action_name}` must accept a string target"
                );
            } else {
                assert!(
                    parameter_type.is_none(),
                    "untargeted menu action `{action_name}` must not require a parameter"
                );
            }
        }
    }

    #[test]
    fn message_dialog_text_keeps_caption_out_of_body_like_win32_message_box() {
        assert_eq!(
            message_dialog_text("j3TreeText", "Save changes?"),
            MessageDialogText {
                title: "j3TreeText",
                primary_text: "Save changes?",
            }
        );
    }

    #[test]
    fn text_file_dialog_filters_match_win32_visible_choices() {
        let filters = text_file_dialog_filter_specs(ui_text(UiLanguage::English));

        assert_eq!(
            filters,
            [
                DialogFileFilterSpec {
                    name: "Text Files (*.txt)",
                    patterns: &["*.txt"],
                    default: true,
                },
                DialogFileFilterSpec {
                    name: "All Files (*.*)",
                    patterns: &["*"],
                    default: false,
                },
            ]
        );
    }

    #[test]
    fn export_dialog_does_not_prefill_filename_like_win32_save_dialog() {
        assert_eq!(text_file_dialog_initial_name(FileDialogMode::Import), None);
        assert_eq!(text_file_dialog_initial_name(FileDialogMode::Export), None);
    }

    #[test]
    fn export_dialog_reopens_after_declined_overwrite_like_win32_save_dialog() {
        let path = PathBuf::from("/tmp/report");

        assert_eq!(
            export_text_file_dialog_path(path, |_| false),
            TextFileDialogPath::Reopen(PathBuf::from("/tmp/report.txt"))
        );
    }

    #[test]
    fn export_dialog_accepts_approved_or_new_path_with_windows_txt_extension() {
        assert_eq!(
            export_text_file_dialog_path(PathBuf::from("/tmp/report"), |_| true),
            TextFileDialogPath::Accepted(PathBuf::from("/tmp/report.txt"))
        );
        assert_eq!(
            export_text_file_dialog_path(PathBuf::from("/tmp/report.md"), |_| true),
            TextFileDialogPath::Accepted(PathBuf::from("/tmp/report.md"))
        );
    }

    #[test]
    fn import_text_reloads_active_editor_even_when_content_matches_win32() {
        let import_body = rust_function_body(include_str!("linux.rs"), "import_text_action");
        assert!(
            import_body.contains("reload_active_editor_page_from_tab_state(state);"),
            "import must reload the active editor after refresh like Win32 show_active_tab_in_editor_with_prepared_text"
        );

        let reload_body = rust_function_body(
            include_str!("linux.rs"),
            "reload_active_editor_page_from_tab_state",
        );
        assert!(
            reload_body.contains("clear_find_match_highlight(&page.buffer);")
                && reload_body.contains("restore_buffer_view_state(")
                && reload_body.contains("reset_text_buffer_undo_stack(&page.buffer);"),
            "import editor reload must clear temporary highlight, restore tab view state, and reset undo"
        );
    }

    #[test]
    fn tree_delete_shortcut_restores_focus_after_action() {
        let active_delete = tree_key_command(
            gdk::Key::Delete,
            gdk::ModifierType::empty(),
            TreeMode::Active,
        )
        .expect("active tree Delete should be handled");
        let trash_delete = tree_key_command(
            gdk::Key::Delete,
            gdk::ModifierType::empty(),
            TreeMode::Trash,
        )
        .expect("trash tree Delete should be handled");
        let rename = tree_key_command(gdk::Key::F2, gdk::ModifierType::empty(), TreeMode::Active)
            .expect("F2 should be handled");
        let new_document = tree_key_command(
            gdk::Key::Return,
            gdk::ModifierType::empty(),
            TreeMode::Active,
        )
        .expect("Return should be handled");

        assert_eq!(active_delete, TreeKeyCommand::MoveToTrash);
        assert_eq!(trash_delete, TreeKeyCommand::DeletePermanently);
        assert!(active_delete.restores_tree_focus_after_action());
        assert!(trash_delete.restores_tree_focus_after_action());
        assert!(!rename.restores_tree_focus_after_action());
        assert!(!new_document.restores_tree_focus_after_action());
        assert_eq!(
            tree_key_command(
                gdk::Key::Return,
                gdk::ModifierType::ALT_MASK,
                TreeMode::Active
            ),
            None
        );
    }

    #[test]
    fn tree_shortcuts_match_win32_alt_modifier_policy() {
        let ctrl_alt = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;

        assert_eq!(
            tree_key_command(
                gdk::Key::Return,
                gdk::ModifierType::ALT_MASK,
                TreeMode::Active
            ),
            None
        );
        assert_eq!(
            tree_key_command(gdk::Key::Return, ctrl_alt, TreeMode::Active),
            None
        );
        assert_eq!(
            tree_key_command(gdk::Key::F2, gdk::ModifierType::ALT_MASK, TreeMode::Active),
            Some(TreeKeyCommand::Rename)
        );
        assert_eq!(
            tree_key_command(gdk::Key::F2, ctrl_alt, TreeMode::Active),
            Some(TreeKeyCommand::Rename)
        );
        assert_eq!(
            tree_key_command(
                gdk::Key::Delete,
                gdk::ModifierType::ALT_MASK,
                TreeMode::Active
            ),
            Some(TreeKeyCommand::MoveToTrash)
        );
        assert_eq!(
            tree_key_command(
                gdk::Key::Delete,
                gdk::ModifierType::ALT_MASK,
                TreeMode::Trash
            ),
            Some(TreeKeyCommand::DeletePermanently)
        );
        assert_eq!(
            tree_key_command(gdk::Key::Up, ctrl_alt, TreeMode::Active),
            Some(TreeKeyCommand::MoveUp)
        );
        assert_eq!(
            tree_key_command(gdk::Key::Down, ctrl_alt, TreeMode::Active),
            Some(TreeKeyCommand::MoveDown)
        );
    }

    #[test]
    fn tree_navigation_keys_match_win32_treeview_defaults() {
        let right = tree_key_command(
            gdk::Key::Right,
            gdk::ModifierType::empty(),
            TreeMode::Active,
        );
        let keypad_right = tree_key_command(
            gdk::Key::KP_Right,
            gdk::ModifierType::empty(),
            TreeMode::Active,
        );
        let left = tree_key_command(gdk::Key::Left, gdk::ModifierType::empty(), TreeMode::Active);
        let expand = tree_key_command(gdk::Key::plus, gdk::ModifierType::empty(), TreeMode::Active);
        let collapse = tree_key_command(
            gdk::Key::minus,
            gdk::ModifierType::empty(),
            TreeMode::Active,
        );
        let expand_subtree = tree_key_command(
            gdk::Key::asterisk,
            gdk::ModifierType::empty(),
            TreeMode::Active,
        );

        assert_eq!(right, Some(TreeKeyCommand::ExpandOrSelectChild));
        assert_eq!(keypad_right, Some(TreeKeyCommand::ExpandOrSelectChild));
        assert_eq!(left, Some(TreeKeyCommand::CollapseOrSelectParent));
        assert_eq!(expand, Some(TreeKeyCommand::ExpandNode));
        assert_eq!(collapse, Some(TreeKeyCommand::CollapseNode));
        assert_eq!(expand_subtree, Some(TreeKeyCommand::ExpandSubtree));
        assert_eq!(
            tree_key_command(
                gdk::Key::Right,
                gdk::ModifierType::CONTROL_MASK,
                TreeMode::Active
            ),
            None
        );
        assert_eq!(
            tree_key_command(
                gdk::Key::Left,
                gdk::ModifierType::ALT_MASK,
                TreeMode::Active
            ),
            None
        );
    }

    #[test]
    fn command_availability_ignores_stale_selected_node_id() {
        let document = UiDocument::from_ui_nodes(vec![ui_node(ROOT_NODE_ID, None, "Root", 0)]);
        let selected_node_id = existing_selected_node_id(&document, Some(99));
        let (move_up_enabled, move_down_enabled) =
            sibling_move_availability(&document, selected_node_id);

        let availability = GuiCommandAvailability::for_context(
            GuiTreeMode::Active,
            false,
            selected_node_id,
            move_up_enabled,
            move_down_enabled,
            false,
        );

        assert_eq!(selected_node_id, None);
        assert!(availability.new_document_enabled);
        assert!(!availability.new_child_document_enabled);
        assert!(!availability.rename_enabled);
        assert!(!availability.delete_enabled);
    }

    #[test]
    fn sibling_move_availability_tracks_order_boundaries() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(ROOT_NODE_ID, None, "Root", 0),
            ui_node(2, Some(ROOT_NODE_ID), "Alpha", 0),
            ui_node(3, Some(ROOT_NODE_ID), "Beta", 1),
            ui_node(4, Some(ROOT_NODE_ID), "Gamma", 2),
        ]);

        assert_eq!(sibling_move_availability(&document, None), (false, false));
        assert_eq!(
            sibling_move_availability(&document, Some(ROOT_NODE_ID)),
            (false, false)
        );
        assert_eq!(sibling_move_availability(&document, Some(2)), (false, true));
        assert_eq!(sibling_move_availability(&document, Some(3)), (true, true));
        assert_eq!(sibling_move_availability(&document, Some(4)), (true, false));
    }

    #[test]
    fn sibling_move_availability_uses_display_order_when_nodes_are_not_vector_sorted() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(ROOT_NODE_ID, None, "Root", 0),
            ui_node(4, Some(ROOT_NODE_ID), "Gamma", 2),
            ui_node(2, Some(ROOT_NODE_ID), "Alpha", 0),
            ui_node(3, Some(ROOT_NODE_ID), "Beta", 1),
        ]);

        assert_eq!(sibling_move_availability(&document, Some(2)), (false, true));
        assert_eq!(sibling_move_availability(&document, Some(4)), (true, false));
    }

    #[test]
    fn trusted_active_document_uses_source_order_for_display_children() -> Result<(), AppError> {
        let nodes = vec![
            active_node(ROOT_NODE_ID, None, "Root", 0),
            active_node(4, Some(ROOT_NODE_ID), "Gamma", 2),
            active_node(2, Some(ROOT_NODE_ID), "Alpha", 0),
            active_node(3, Some(ROOT_NODE_ID), "Beta", 1),
        ];

        let document = UiDocument::from_nodes(&nodes, false, UiLanguage::English, true)?;

        assert_eq!(
            visible_ids(&document, &[ROOT_NODE_ID], false),
            vec![1, 4, 2, 3]
        );
        assert_eq!(sibling_move_availability(&document, Some(4)), (false, true));
        Ok(())
    }

    #[test]
    fn sibling_move_availability_reuses_display_child_index_cache() {
        let body = rust_function_body(include_str!("linux.rs"), "sibling_move_availability");

        assert!(body.contains("display_child_indices_by_parent"));
        assert!(!body.contains("collect::<Vec"));
        assert!(!body.contains("sort_by"));
    }

    #[test]
    fn selected_node_id_for_commands_rejects_stale_selection_like_win32_selected_node() {
        let document = UiDocument::from_ui_nodes(vec![ui_node(ROOT_NODE_ID, None, "Root", 0)]);

        assert_eq!(
            selected_existing_node_id(&document, Some(ROOT_NODE_ID)).expect("root exists"),
            ROOT_NODE_ID
        );

        for selected_node_id in [None, Some(99)] {
            let error = selected_existing_node_id(&document, selected_node_id)
                .expect_err("missing selection must be rejected");
            match error {
                AppError::Domain(DomainError::NodeNotFound { node_id }) => {
                    assert_eq!(node_id, 0)
                }
                other => panic!("unexpected error: {other}"),
            }
        }
    }

    #[test]
    fn selection_ui_setting_updates_only_from_active_browse_selection_like_win32() {
        let current = Some(10);

        assert_eq!(
            selection_node_id_for_ui_settings(TreeMode::Active, "", Some(42), current),
            Some(42)
        );
        assert_eq!(
            selection_node_id_for_ui_settings(TreeMode::Active, "", None, current),
            current
        );
        assert_eq!(
            selection_node_id_for_ui_settings(TreeMode::Active, "needle", Some(42), current),
            current
        );
        assert_eq!(
            selection_node_id_for_ui_settings(TreeMode::Trash, "", Some(42), current),
            current
        );
    }

    #[test]
    fn tree_selection_ui_setting_save_is_debounced_and_flushed_with_persist() {
        let source = include_str!("linux.rs");
        let select_body = rust_function_body(source, "select_tree_node_with_navigation");
        let schedule_body = rust_function_body(source, "schedule_selection_ui_setting_save");
        let flush_body = rust_function_body(source, "flush_pending_selection_ui_setting");
        let persist_body = rust_function_body(source, "persist_ui_settings");

        assert!(select_body.contains("schedule_selection_ui_setting_save(state, node_id);"));
        assert!(!select_body.contains("save_selection_ui_setting"));
        assert!(schedule_body.contains("pending_selection_node_id"));
        assert!(schedule_body.contains("selection_ui_settings_generation"));
        assert!(schedule_body.contains("timeout_add_local_once"));
        assert!(schedule_body.contains("flush_pending_selection_ui_setting(&state)"));
        assert!(flush_body.contains("state.app.save_ui_settings(settings)?"));
        assert!(persist_body.contains("pending_selection_node_id"));
    }

    #[test]
    fn visible_order_respects_tree_expansion_state() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 1),
            ui_node(4, Some(2), "Alpha child", 0),
        ]);

        assert_eq!(visible_ids(&document, &[], false), vec![1]);
        assert_eq!(visible_ids(&document, &[1], false), vec![1, 2, 3]);
        assert_eq!(visible_ids(&document, &[1, 2], false), vec![1, 2, 4, 3]);
        assert_eq!(visible_depths(&document, &[1, 2], false), vec![0, 1, 2, 1]);
        assert_eq!(visible_ids(&document, &[], true), vec![1, 2, 4, 3]);
    }

    #[test]
    fn visible_descendant_count_uses_contiguous_depth_range() {
        let row_specs = vec![
            TreeRowSpec {
                depth: 0,
                ..tree_row_spec(1)
            },
            TreeRowSpec {
                depth: 1,
                ..tree_row_spec(2)
            },
            TreeRowSpec {
                depth: 2,
                ..tree_row_spec(4)
            },
            TreeRowSpec {
                depth: 1,
                ..tree_row_spec(3)
            },
            TreeRowSpec {
                depth: 0,
                ..tree_row_spec(5)
            },
        ];

        assert_eq!(visible_descendant_count(&row_specs, 0), 3);
        assert_eq!(visible_descendant_count(&row_specs, 1), 1);
        assert_eq!(visible_descendant_count(&row_specs, 3), 0);
    }

    #[test]
    fn tree_navigation_actions_match_win32_treeview_defaults() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 1),
            ui_node(4, Some(2), "Alpha child", 0),
        ]);
        let collapsed = visible_ids(&document, &[], false);
        let root_expanded = visible_ids(&document, &[1], false);
        let alpha_expanded = visible_ids(&document, &[1, 2], false);

        assert_eq!(
            tree_right_key_action(&document, &collapsed, 1),
            TreeNavigationAction::Expand(1)
        );
        assert_eq!(
            tree_right_key_action(&document, &root_expanded, 1),
            TreeNavigationAction::Select(2)
        );
        assert_eq!(
            tree_left_key_action(&document, &root_expanded, 1),
            TreeNavigationAction::Collapse(1)
        );
        assert_eq!(
            tree_left_key_action(&document, &root_expanded, 2),
            TreeNavigationAction::Select(1)
        );
        assert_eq!(
            tree_expand_key_action(&document, 1),
            TreeNavigationAction::Expand(1)
        );
        assert_eq!(
            tree_collapse_key_action(&document, 1),
            TreeNavigationAction::Collapse(1)
        );
        assert_eq!(
            tree_expand_subtree_key_action(&document, 1),
            TreeNavigationAction::ExpandSubtree(1)
        );
        assert_eq!(
            tree_right_key_action(&document, &alpha_expanded, 4),
            TreeNavigationAction::None
        );
    }

    #[test]
    fn search_result_expansion_can_be_collapsed_after_initial_win32_expand_all() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 1),
            ui_node(4, Some(2), "Alpha child", 0),
        ]);
        let expanded_node_ids = document.expandable_node_ids();
        let expanded_node_ids = expanded_node_ids.iter().copied().collect::<Vec<_>>();

        assert_eq!(
            visible_ids(&document, &expanded_node_ids, false),
            vec![1, 2, 4, 3]
        );
        assert_eq!(visible_ids(&document, &[], false), vec![1]);
        assert_eq!(
            document.expandable_subtree_node_ids(1),
            HashSet::from([1, 2])
        );
    }

    #[test]
    fn tree_full_rebuild_expands_all_like_win32_tree_population() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 1),
            ui_node(4, Some(2), "Alpha child", 0),
        ]);
        let current = HashSet::from([99]);

        assert_eq!(
            expanded_node_ids_for_tree_refresh(
                &document,
                &current,
                TreeRefreshExpansion::ExpandAll
            ),
            HashSet::from([1, 2])
        );
    }

    #[test]
    fn tree_incremental_refresh_preserves_existing_expansion_state() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 1),
        ]);
        let current = HashSet::from([1, 99]);

        assert_eq!(
            expanded_node_ids_for_tree_refresh(&document, &current, TreeRefreshExpansion::Preserve),
            current
        );
    }

    #[test]
    fn tree_row_order_change_tracks_local_insert_delete_and_move_windows() {
        let previous = [tree_row_spec(1), tree_row_spec(3)];
        let next = [tree_row_spec(1), tree_row_spec(2), tree_row_spec(3)];
        assert_eq!(
            tree_row_order_change(&previous, &next),
            Some(TreeRowOrderChange {
                prefix_len: 1,
                previous_changed_len: 0,
                next_changed_len: 1,
                suffix_len: 1,
            })
        );

        let previous = [tree_row_spec(1), tree_row_spec(2), tree_row_spec(3)];
        let next = [tree_row_spec(1), tree_row_spec(3)];
        assert_eq!(
            tree_row_order_change(&previous, &next),
            Some(TreeRowOrderChange {
                prefix_len: 1,
                previous_changed_len: 1,
                next_changed_len: 0,
                suffix_len: 1,
            })
        );

        let previous = [
            tree_row_spec(1),
            tree_row_spec(2),
            tree_row_spec(3),
            tree_row_spec(4),
        ];
        let next = [
            tree_row_spec(1),
            tree_row_spec(3),
            tree_row_spec(2),
            tree_row_spec(4),
        ];
        assert_eq!(
            tree_row_order_change(&previous, &next),
            Some(TreeRowOrderChange {
                prefix_len: 1,
                previous_changed_len: 2,
                next_changed_len: 2,
                suffix_len: 1,
            })
        );
    }

    #[test]
    fn tree_row_order_change_uses_full_rebuild_when_no_stable_edge_exists() {
        let previous = [tree_row_spec(1), tree_row_spec(2)];
        let next = [tree_row_spec(2), tree_row_spec(1)];

        assert_eq!(tree_row_order_change(&previous, &next), None);
        assert_eq!(tree_row_order_change(&previous, &previous), None);
        assert_eq!(tree_row_order_change(&[], &next), None);
    }

    #[test]
    fn renamed_tree_row_refresh_allows_unique_sort_order_without_rebuild() {
        let previous = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 1),
        ]);
        let next = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Gamma", 0),
            ui_node(3, Some(1), "Beta", 1),
        ]);

        assert!(can_refresh_renamed_tree_row(
            &previous,
            &next,
            &[1, 2, 3],
            2
        ));
    }

    #[test]
    fn renamed_tree_row_refresh_rebuilds_when_title_can_change_order() {
        let previous = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(3, Some(1), "Beta", 0),
        ]);
        let next = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Gamma", 0),
            ui_node(3, Some(1), "Beta", 0),
        ]);

        assert!(!can_refresh_renamed_tree_row(
            &previous,
            &next,
            &[1, 2, 3],
            2
        ));
    }

    #[test]
    fn display_ancestor_node_ids_returns_nearest_parent_first() {
        let document = UiDocument::from_ui_nodes(vec![
            ui_node(1, None, "Root", 0),
            ui_node(2, Some(1), "Alpha", 0),
            ui_node(4, Some(2), "Alpha child", 0),
        ]);

        assert_eq!(document.display_ancestor_node_ids(4), vec![2, 1]);
        assert_eq!(document.display_ancestor_node_ids(1), Vec::<i64>::new());
    }

    #[test]
    fn search_result_document_keeps_matches_visible_without_expansion() {
        let node = Node {
            id: 10,
            parent_id: Some(1),
            title: "Needle".to_owned(),
            sort_order: 0,
            content: String::new(),
            created_at: "2026-06-16T00:00:00Z".to_owned(),
            updated_at: "2026-06-16T00:00:00Z".to_owned(),
            deleted_at: None,
        };
        let document = UiDocument::from_search_results(
            vec![DocumentSearchResult {
                node,
                parent_title: Some("Root".to_owned()),
                content_matched: true,
            }],
            UiLanguage::English,
        )
        .expect("search result UI document should build");

        assert_eq!(visible_ids(&document, &[], false), vec![10]);
        assert_eq!(document.nodes[0].source, DocumentTabSource::SearchResult);
        assert!(document.nodes[0].search_content_matched);
        assert_eq!(document.nodes[0].display_parent_id, None);
    }

    #[test]
    fn search_refinement_allows_short_title_only_refinement() {
        let document = UiDocument::from_ui_nodes(vec![search_ui_node("xy title", false)]);

        assert!(can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "x",
            "xy"
        ));
    }

    #[test]
    fn search_refinement_rejects_content_matched_results() {
        let document = UiDocument::from_ui_nodes(vec![search_ui_node("Notes", true)]);

        assert!(!can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "body",
            "body followup"
        ));
    }

    #[test]
    fn refined_search_document_filters_visible_title_results() {
        let document = UiDocument::from_ui_nodes(vec![
            search_ui_node("Alpha", false),
            search_ui_node("Alpine", false),
            search_ui_node("Beta", false),
        ]);

        let refined =
            refined_search_document_from_visible_document(&document, TreeMode::Active, "a", "al")
                .expect("title-only search results should refine in memory");

        assert_eq!(
            refined
                .nodes
                .iter()
                .map(|node| node.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Alpine"]
        );
    }

    #[test]
    fn find_next_wrapping_restarts_from_document_start() {
        let content = "one two one";

        assert_eq!(
            find_next_wrapping(content, "one", content.len()),
            Some(TextMatch { start: 0, end: 3 })
        );
        assert_eq!(find_next_wrapping(content, "missing", 4), None);
    }

    #[test]
    fn replace_one_action_requires_exact_selected_match() {
        let content = "one two one";

        assert_eq!(
            replace_one_action(content, "one", TextMatch { start: 0, end: 0 }),
            ReplaceOneAction::SelectNext(TextMatch { start: 0, end: 3 })
        );
        assert_eq!(
            replace_one_action(content, "one", TextMatch { start: 0, end: 3 }),
            ReplaceOneAction::ReplaceSelected(TextMatch { start: 0, end: 3 })
        );
        assert_eq!(
            replace_one_action(content, "two", TextMatch { start: 0, end: 3 }),
            ReplaceOneAction::SelectNext(TextMatch { start: 4, end: 7 })
        );
    }

    #[test]
    fn linux_find_replace_paths_do_not_clone_active_content() {
        let source = include_str!("linux.rs");
        let find_next = rust_function_body(source, "find_next_in_active_editor");
        let replace_one = rust_function_body(source, "replace_one_in_active_editor");
        let replace_all = rust_function_body(source, "replace_all_in_active_editor");

        for body in [find_next, replace_one, replace_all] {
            assert!(
                !body.contains("tab.content.clone()"),
                "find/replace paths must search borrowed active tab content"
            );
        }
        assert!(
            !replace_one.contains("content.clone()") && !replace_one.contains("next.clone()"),
            "single replace must not clone active content"
        );
        assert!(
            !replace_one.contains("String::with_capacity")
                && !replace_one.contains("next.push_str")
                && replace_one.contains("replace_active_content_range"),
            "single replace must update the active tab content without assembling a full replacement string"
        );
    }

    #[test]
    fn find_replace_entry_limit_uses_windows_utf16_capacity() {
        let insert = "a".repeat(FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS + 1);

        assert_eq!(
            limited_find_replace_insert_text(
                "",
                &insert,
                0,
                None,
                FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS,
            )
            .expect("over-limit insert should be truncated")
            .len(),
            FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS
        );
        assert!(limited_find_replace_insert_text(
            "",
            &"a".repeat(FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS),
            0,
            None,
            FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS,
        )
        .is_none());
    }

    #[test]
    fn find_replace_entry_limit_counts_emoji_as_two_utf16_units() {
        let rocket = "🚀";
        let allowed = FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS / rocket.encode_utf16().count();
        let insert = rocket.repeat(allowed + 1);
        let truncated = limited_find_replace_insert_text(
            "",
            &insert,
            0,
            None,
            FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS,
        )
        .expect("UTF-16 over-limit insert should be truncated");

        assert_eq!(truncated, rocket.repeat(allowed));
        assert!(truncated.encode_utf16().count() <= FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS);
    }

    #[test]
    fn find_replace_entry_limit_accounts_for_selected_replacement() {
        let current = "a".repeat(FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS);
        let insert = "bc";

        assert_eq!(
            limited_find_replace_insert_text(
                &current,
                insert,
                0,
                Some((0, 1)),
                FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS,
            )
            .expect("replacement should be truncated to the freed selection width"),
            "b"
        );
        assert!(limited_find_replace_insert_text(
            &current,
            "z",
            0,
            Some((0, 1)),
            FIND_REPLACE_TEXT_LIMIT_UTF16_UNITS,
        )
        .is_none());
    }

    #[test]
    fn imported_text_uses_windows_editor_utf16_limit_after_normalization() {
        let mut content = "\n".repeat(TEXT_FILE_BYTE_LIMIT / 2 + 1);

        prepare_imported_text(&mut content, UiLanguage::English)
            .expect("LF-heavy imported text should fit the Windows editor limit");

        assert!(content.len() > TEXT_FILE_BYTE_LIMIT);
        assert!(
            document_editor_text_len_utf16(&content).expect("prepared text length should fit")
                <= DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS
        );
    }

    #[test]
    fn imported_text_with_existing_crlf_does_not_reallocate() {
        let mut content = String::from("Alpha\r\nBeta 🙂\r\nGamma");
        let original_ptr = content.as_ptr();
        let original_capacity = content.capacity();

        prepare_imported_text(&mut content, UiLanguage::English)
            .expect("CRLF-only imported text should already be normalized");

        assert_eq!(content, "Alpha\r\nBeta 🙂\r\nGamma");
        assert_eq!(content.as_ptr(), original_ptr);
        assert_eq!(content.capacity(), original_capacity);
    }

    #[test]
    fn editor_sync_text_limit_detects_windows_utf16_overflow() {
        let ascii = "abcde";
        assert_eq!(utf16_limit_truncation_byte_index(ascii, 3), Some(3));

        let emoji = "ab🙂c";
        assert_eq!(utf16_limit_truncation_byte_index(emoji, 3), Some(2));

        let exact = "ab🙂";
        assert_eq!(utf16_limit_truncation_byte_index(exact, 4), None);
    }

    #[test]
    fn editor_view_state_offsets_scan_caret_and_selection_once() {
        let offsets = scan_gtk_tab_view_state_utf16_offsets("a🙂bc", 2, Some((1, 3)), None);

        assert_eq!(offsets.caret_position_utf16, 3);
        assert_eq!(offsets.selection_start_utf16, 1);
        assert_eq!(offsets.selection_end_utf16, 4);
        assert_eq!(offsets.utf16_limit_truncation_byte_index, None);
    }

    #[test]
    fn editor_view_state_offsets_scan_reports_windows_utf16_limit() {
        let offsets = scan_gtk_tab_view_state_utf16_offsets("ab🙂c", 0, None, Some(3));

        assert_eq!(offsets.utf16_limit_truncation_byte_index, Some(2));
    }

    #[test]
    fn restored_selection_scan_maps_utf16_offsets_directly_to_char_offsets() {
        let offsets = scan_restored_selection_char_offsets("a🙂bc", 2, 3);

        assert_eq!(offsets.max_text_offset_utf16, 5);
        assert_eq!(offsets.selection_start_char_offset, 1);
        assert_eq!(offsets.selection_end_char_offset, 2);

        let end_offsets = scan_restored_selection_char_offsets("a🙂bc", 9, 9);
        assert_eq!(end_offsets.selection_start_char_offset, 4);
        assert_eq!(end_offsets.selection_end_char_offset, 4);
    }

    #[test]
    fn editor_sync_rejects_too_large_text_without_truncating_model_or_buffer() {
        let body = rust_function_body(include_str!("linux.rs"), "sync_active_editor_content");
        let checked_index = body
            .find("gtk_tab_view_state_checked(&page.view, &page.buffer, &content)")
            .unwrap_or_else(|| panic!("over-limit editor sync must check editor text"));
        let update_index = body
            .find("state.tabs.update_active_content_reusing(&mut content);")
            .unwrap_or_else(|| panic!("successful editor sync must still update the tab model"));
        assert!(
            body.contains("editor_text_too_large(DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB)")
                && checked_index < update_index,
            "active editor sync must reject over-limit text as a recoverable user error"
        );
        assert!(
            !body.contains("delete_buffer_suffix_from_byte_index")
                && !body.contains("content.truncate("),
            "active editor sync must not silently delete over-limit editor text"
        );
    }

    #[test]
    fn editor_menu_text_state_prefers_live_buffer_like_win32() {
        assert!(editor_has_text_for_menu_state(Some(""), Some(1)));
        assert!(!editor_has_text_for_menu_state(
            Some("stale model"),
            Some(0)
        ));
    }

    #[test]
    fn editor_menu_text_state_falls_back_to_tab_model_without_live_buffer() {
        assert!(editor_has_text_for_menu_state(Some("cached"), None));
        assert!(!editor_has_text_for_menu_state(Some(""), None));
        assert!(!editor_has_text_for_menu_state(None, None));
    }

    #[test]
    fn caret_status_column_uses_utf16_units_like_win32() {
        assert_eq!(caret_column_utf16_for_line_prefix("B🚀"), 4);
    }

    #[test]
    fn caret_status_column_is_one_based() {
        assert_eq!(caret_column_utf16_for_line_prefix(""), 1);
        assert_eq!(caret_column_utf16_for_line_prefix("ab"), 3);
    }

    #[test]
    fn caret_status_does_not_copy_full_buffer_for_column() {
        let body = rust_function_body(include_str!("linux.rs"), "caret_line_column_for_page");
        assert!(
            !body.contains("buffer_text("),
            "caret status must not copy the full GTK buffer on mark-set updates"
        );

        let column_body =
            rust_function_body(include_str!("linux.rs"), "caret_column_utf16_for_range");
        assert!(
            !column_body.contains(".text("),
            "caret column status must not allocate line prefix text on mark-set updates"
        );
    }

    #[test]
    fn text_mark_set_updates_only_for_cursor_and_selection_marks() {
        let body = rust_function_body(include_str!("linux.rs"), "connect_text_page_signals");

        assert!(body.contains("is_insert_mark(buffer, mark)"));
        assert!(body.contains("is_selection_bound_mark(buffer, mark)"));
        assert!(body.contains("update_caret_status(&state);"));
        assert!(body.contains("update_actions(&state);"));
    }

    #[test]
    fn context_menu_pointing_coordinate_is_clamped_and_finite() {
        assert_eq!(pointing_coordinate(12.4), 12);
        assert_eq!(pointing_coordinate(12.5), 13);
        assert_eq!(pointing_coordinate(f64::NAN), 0);
        assert_eq!(pointing_coordinate(f64::INFINITY), 0);
        assert_eq!(pointing_coordinate((i32::MAX as f64) * 2.0), i32::MAX);
        assert_eq!(pointing_coordinate((i32::MIN as f64) * 2.0), i32::MIN);
    }

    #[test]
    fn context_menu_pointer_tracking_keeps_only_finite_points() {
        assert_eq!(
            context_menu_point_from_pointer(10.0, 20.5),
            Some(ContextMenuPoint { x: 10.0, y: 20.5 })
        );
        assert_eq!(context_menu_point_from_pointer(f64::NAN, 20.0), None);
        assert_eq!(context_menu_point_from_pointer(10.0, f64::INFINITY), None);
    }

    #[test]
    fn context_menu_key_matches_win32_keyboard_entry_points() {
        assert!(is_context_menu_key(
            gdk::Key::Menu,
            gdk::ModifierType::empty()
        ));
        assert!(is_context_menu_key(
            gdk::Key::F10,
            gdk::ModifierType::SHIFT_MASK
        ));
        assert!(!is_context_menu_key(
            gdk::Key::F10,
            gdk::ModifierType::empty()
        ));
        assert!(!is_context_menu_key(
            gdk::Key::F10,
            gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK
        ));
        assert!(!is_context_menu_key(
            gdk::Key::F10,
            gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK
        ));
    }

    #[test]
    fn tree_widget_keeps_win32_keyboard_focus_contract() {
        let source = include_str!("linux.rs");
        let window_signals_body = rust_function_body(source, "connect_window_signals");

        assert!(
            source.contains("tree.set_focusable(true);"),
            "GTK ListBox tree must be focusable so Win32 TreeView keyboard commands reach it"
        );
        assert!(
            window_signals_body.contains("tree.add_controller(controller);")
                && window_signals_body.contains("handle_tree_key_pressed(&state, key, modifiers)"),
            "tree key handling must stay attached to the focusable tree widget"
        );
        assert!(
            window_signals_body.contains("tree_for_focus.grab_focus();"),
            "tree mouse presses must focus the tree before Win32-style keyboard commands"
        );
    }

    #[test]
    fn context_menu_click_gesture_preempts_native_widget_context_menus() {
        let body = rust_function_body(include_str!("linux.rs"), "context_menu_click_gesture");

        assert!(
            body.contains("gesture.set_button(3);")
                && body.contains("gtk::PropagationPhase::Capture")
                && body.contains("gesture.set_exclusive(true);"),
            "tree right-click context gestures must run before default widget handling and own the sequence"
        );
        assert!(
            include_str!("linux.rs").contains("gtk::EventSequenceState::Claimed"),
            "tree right-click handlers must claim the sequence before showing the app context menu"
        );
    }

    #[test]
    fn editor_context_menu_controller_stops_native_textview_menu() {
        let source = include_str!("linux.rs");
        let window_signals_body = rust_function_body(source, "connect_window_signals");
        let editor_body =
            rust_function_body(source, "show_editor_context_menu_from_window_position");

        assert!(
            window_signals_body.contains("let controller = gtk::EventControllerLegacy::new();")
                && window_signals_body
                    .contains("controller.set_propagation_phase(gtk::PropagationPhase::Capture);")
                && window_signals_body.contains("secondary_button_press_position(event)")
                && window_signals_body.contains("show_editor_context_menu_from_window_position")
                && window_signals_body.contains("glib::Propagation::Stop"),
            "editor right-click must be captured at the window before GTK TextView opens its native menu"
        );
        assert!(
            editor_body.contains("context_menu_point_in_widget(&view, &window, 0.0, 0.0)")
                && editor_body.contains("point_inside_widget(&view, view_x, view_y)")
                && editor_body.contains("show_context_menu(EDITOR_CONTEXT_MENU_ENTRIES"),
            "editor right-click capture must be limited to the active editor and show the app menu"
        );
    }

    #[test]
    fn keyboard_editor_context_menu_reuses_only_active_tab_pointer() {
        let remembered = Some(EditorContextMenuPoint {
            node_id: 42,
            point: ContextMenuPoint { x: 15.0, y: 25.0 },
        });

        assert_eq!(
            active_editor_context_menu_point(Some(42), remembered),
            Some((15.0, 25.0))
        );
        assert_eq!(active_editor_context_menu_point(Some(7), remembered), None);
        assert_eq!(active_editor_context_menu_point(None, remembered), None);
    }

    #[test]
    fn context_menus_are_parented_to_window_with_translated_points() {
        let source = include_str!("linux.rs");
        let tree_body = rust_function_body(source, "show_tree_context_menu_at_position");
        let editor_body =
            rust_function_body(source, "show_editor_context_menu_from_window_position");

        assert!(
            tree_body.contains("state.widgets.window.clone().upcast::<gtk::Widget>()")
                && tree_body.contains("context_menu_point_in_window(&tree, &window"),
            "tree popovers must be parented to the stable window, not the rebuilt ListBox"
        );
        assert!(
            editor_body.contains("state.widgets.window.clone().upcast::<gtk::Widget>()")
                && editor_body.contains("show_context_menu(EDITOR_CONTEXT_MENU_ENTRIES, text, &window"),
            "editor popovers must be parented to the stable window while preempting the native TextView menu"
        );
    }

    #[test]
    fn drop_move_accepts_only_matching_internal_tree_drag() {
        assert_eq!(accepted_internal_dragged_node(Some(42), "42"), Some(42));
        assert_eq!(accepted_internal_dragged_node(Some(42), "7"), None);
        assert_eq!(accepted_internal_dragged_node(None, "42"), None);
        assert_eq!(accepted_internal_dragged_node(Some(42), "not-a-node"), None);
    }

    #[test]
    fn tree_drag_drop_keeps_win32_internal_move_contract() {
        let source = include_str!("linux.rs");
        let connect_body = rust_function_body(source, "connect_tree_row_drag_drop");
        let enabled_body = rust_function_body(source, "tree_drag_drop_enabled");
        let move_body = rust_function_body(source, "move_node_by_drop");

        assert!(
            connect_body.contains("let drag = gtk::DragSource::new();")
                && connect_body.contains("drag.set_actions(gdk::DragAction::MOVE);")
                && connect_body.contains("drag.connect_prepare")
                && connect_body
                    .contains("drag_begin_state.borrow_mut().dragging_node_id = Some(node_id);")
                && connect_body
                    .contains("drag_cancel_state.borrow_mut().dragging_node_id = None;")
                && connect_body.contains("drag_end_state.borrow_mut().dragging_node_id = None;")
                && connect_body.contains(
                    "let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);"
                )
                && connect_body
                    .contains("accepted_internal_dragged_node(state.dragging_node_id, text)")
                && connect_body
                    .contains("let result = move_node_by_drop(&state, dragged_node_id, node_id);")
                && connect_body.contains("state.borrow_mut().dragging_node_id = None;"),
            "tree drop must stay limited to the active internal GTK tree drag and clear drag state"
        );
        assert!(
            enabled_body
                .contains("state.tree_mode == TreeMode::Active && state.search_query.trim().is_empty()"),
            "tree drag/drop must remain disabled outside active browse mode like Win32 mutation commands"
        );
        assert!(
            move_body.contains("ensure_active_tree_browse_mode(state)?;")
                && move_body.contains(".move_node_to_parent_end(dragged_node_id, target_node_id)?;")
                && move_body.contains("sync_tabs_from_active_document_local_metadata(state, true)?;")
                && move_body.contains("reload_visible_document(state, Some(dragged_node_id))"),
            "drop move must keep the Win32 drop path order: guard, move, metadata sync, tree reload"
        );
    }

    #[test]
    fn tab_refresh_updates_in_place_only_when_page_counts_match() {
        assert!(can_refresh_tabs_in_place(2, 2, 2));
        assert!(can_refresh_tabs_in_place(0, 0, 0));
        assert!(!can_refresh_tabs_in_place(1, 2, 2));
        assert!(!can_refresh_tabs_in_place(2, 1, 2));
        assert!(!can_refresh_tabs_in_place(2, 2, 1));
    }

    #[test]
    fn tab_refresh_reuses_content_revision_instead_of_full_text_comparison() {
        let refresh_body = rust_function_body(include_str!("linux.rs"), "refresh_tabs");
        assert!(
            !refresh_body.contains("state.tabs.clone()"),
            "tab refresh must not deep-clone open tab contents before deciding how to update pages"
        );
        assert!(
            !refresh_body.contains("state.tab_pages.clone()"),
            "tab refresh should snapshot only the page updates it needs"
        );

        let updates_body =
            rust_function_body(include_str!("linux.rs"), "existing_tab_page_updates");
        assert!(
            updates_body.contains("page.content_revision.get() == content_revision")
                && updates_body
                    .contains("content: (!content_is_current).then(|| tab.content.clone())"),
            "in-place tab refresh should clone document content only for stale pages"
        );

        let update_body = rust_function_body(include_str!("linux.rs"), "update_existing_tab_page");
        assert!(
            !update_body.contains("buffer_text("),
            "in-place tab refresh must not stringify the whole GTK TextBuffer for equality checks"
        );
        assert!(
            update_body.contains("update.content.as_deref()"),
            "existing pages should skip TextBuffer writes when the content revision is current"
        );
    }

    #[test]
    fn splitter_width_is_clamped_before_persisting() {
        assert_eq!(clamp_split_width_for_window(10_000, 900), 736);
        assert_eq!(clamp_split_width_for_window(1, 900), MIN_SPLIT_WIDTH_PX);
        assert_eq!(clamp_split_width_for_window(500, 200), 196);
        assert_eq!(clamp_split_width_for_window(120, 0), 0);
    }

    #[test]
    fn fixed_layout_metrics_match_win32_baseline() {
        assert_eq!(SEARCH_BOX_HEIGHT_PX, 24);
        assert_eq!(SEARCH_PANEL_PADDING_PX, 6);
        assert_eq!(SEARCH_BOX_HEIGHT_PX + SEARCH_PANEL_PADDING_PX * 2, 36);
        assert_eq!(SPLITTER_WIDTH_PX, 4);
        assert_eq!(TAB_BAR_HEIGHT_PX, 28);
        assert_eq!(TAB_CLOSE_HIT_WIDTH_PX, 22);
        assert_eq!(TAB_CLOSE_HIT_RIGHT_PADDING_PX, 2);
        assert_eq!(CARET_STATUS_HEIGHT_PX, 22);
        assert_eq!(CARET_STATUS_HORIZONTAL_PADDING_PX, 8);
        assert_eq!(TREE_INDENT_PX, 18);
        assert_eq!(TREE_ROW_HORIZONTAL_PADDING_PX, 6);
        assert_eq!(TREE_ROW_VERTICAL_PADDING_PX, 3);
        assert_eq!(TREE_EXPANDER_HIT_SIZE_PX, 20);
    }

    #[test]
    fn tree_list_visual_contract_tracks_win32_treeview_baseline() {
        let source = include_str!("linux.rs");
        let setup_body = rust_function_body(source, "build_main_window");
        let row_body = rust_function_body(source, "build_tree_row");

        assert!(
            setup_body.contains("tree.set_selection_mode(gtk::SelectionMode::Single);")
                && setup_body.contains("tree.set_focusable(true);")
                && setup_body.contains("tree.add_css_class(\"j3-tree\");"),
            "GTK tree must stay a single-select focusable widget like the Win32 TreeView"
        );
        assert!(
            row_body.contains("row.add_css_class(\"j3-tree-row\");")
                && row_body.contains("TREE_ROW_VERTICAL_PADDING_PX")
                && row_body.contains("TREE_ROW_HORIZONTAL_PADDING_PX")
                && row_body.contains("TREE_INDENT_PX")
                && row_body.contains("toggle.add_css_class(\"j3-tree-expander\");"),
            "GTK tree rows must keep explicit Win32-aligned row spacing, indent, and expander affordance"
        );
        assert!(
            source.contains(".j3-tree row:selected")
                && source.contains("background: {accent};")
                && source.contains("color: {accent_fg};")
                && source.contains("min-width: {tree_expander_hit_size}px;")
                && source.contains("min-height: {tree_expander_hit_size}px;"),
            "GTK tree CSS must keep persistent selected-row styling and a stable expander hit area"
        );
    }

    #[test]
    fn light_theme_css_paints_app_owned_surfaces_explicitly() {
        let css = theme_css(AppearanceTheme::Light, &EditorFontSettings::default());

        for selector in [
            ".j3-window .j3-menu-bar",
            ".j3-window .j3-search",
            ".j3-window .j3-tree-scroller",
            ".j3-window .j3-tree",
            ".j3-window .j3-tabs",
            ".j3-window .j3-caret-status",
            ".j3-editor-scroller",
        ] {
            assert!(
                css.contains(selector),
                "light theme CSS must explicitly paint `{selector}` instead of inheriting a dark GTK theme"
            );
        }

        for color in [
            "background: #f0f0f0;",
            "background: #ffffff;",
            "color: #000000;",
        ] {
            assert!(
                css.contains(color),
                "light theme CSS must include `{color}` for readable app-owned surfaces"
            );
        }
    }

    #[test]
    fn main_window_widgets_receive_theme_css_classes() {
        let source = include_str!("linux.rs");
        let setup_body = rust_function_body(source, "build_main_window");
        let build_tab_page_body = rust_function_body(source, "build_tab_page_for_index");
        let tree_row_body = rust_function_body(source, "build_tree_row");

        for class_assignment in [
            "menu_bar.add_css_class(\"j3-menu-bar\");",
            "search.add_css_class(\"j3-search\");",
            "tree_scroller.add_css_class(\"j3-tree-scroller\");",
            "left.add_css_class(\"j3-left-pane\");",
            "right.add_css_class(\"j3-right-pane\");",
            "root.add_css_class(\"j3-root\");",
            "caret_status.add_css_class(\"j3-caret-status\");",
        ] {
            assert!(
                setup_body.contains(class_assignment),
                "main window setup must keep `{class_assignment}` so theme CSS reaches GTK widgets"
            );
        }

        assert!(
            build_tab_page_body.contains("scroller.add_css_class(\"j3-editor-scroller\");"),
            "editor scrollers must be themeable because they frame the TextView background"
        );
        assert!(
            tree_row_body.contains("entry.add_css_class(\"j3-tree-entry\");"),
            "tree label edit entries must follow the active theme"
        );
    }

    #[test]
    fn tab_close_button_keeps_win32_click_contract() {
        let body = rust_function_body(include_str!("linux.rs"), "build_tab_label");
        assert!(
            body.contains(".label(\"x\")")
                && body.contains(".focusable(false)")
                && body.contains("close.add_css_class(\"j3-tab-close\");"),
            "GTK tab close affordance must stay a non-focusable x button like the Win32 tab marker"
        );
        assert!(
            body.contains("close.set_size_request(TAB_CLOSE_HIT_WIDTH_PX, -1);")
                && body.contains("close.set_margin_end(TAB_CLOSE_HIT_RIGHT_PADDING_PX);"),
            "GTK tab close target must keep the Win32 22px hit width and 2px trailing gap"
        );
        assert!(
            body.contains("close.connect_clicked")
                && body.contains(".widgets")
                && body.contains(".notebook")
                && body.contains(".page_num(&page)")
                && body.contains(".and_then(|index| usize::try_from(index).ok())")
                && body.contains("run_and_report(&state_clone, close_tab_at_index(&state_clone, index));"),
            "GTK tab close click must resolve the current notebook page at click time before closing"
        );
    }

    #[test]
    fn window_size_persist_preserves_previous_settings_when_current_size_is_invalid() {
        let current = WindowSettings::new(100, 120, 900, 600);

        assert_eq!(persisted_window_settings(current, 0, 600), current);
        assert_eq!(persisted_window_settings(current, 900, 0), current);
        assert_eq!(persisted_window_settings(current, -1, 600), current);

        let resized = persisted_window_settings(current, 100, 100);
        assert_eq!(resized.x, Some(100));
        assert_eq!(resized.y, Some(120));
        assert_eq!(resized.width, 320);
        assert_eq!(resized.height, 240);

        let default_position = persisted_window_settings(WindowSettings::default(), 1024, 768);
        assert_eq!(default_position.x, None);
        assert_eq!(default_position.y, None);
        assert_eq!(default_position.width, 1024);
        assert_eq!(default_position.height, 768);
    }

    #[test]
    fn ui_settings_persist_updates_window_splitter_and_active_selection_like_win32() {
        let current = UiSettings {
            window: WindowSettings::new(100, 120, 900, 600),
            splitter: SplitterSettings::new(240),
            selection: crate::domain::SelectionSettings { node_id: Some(7) },
            ..Default::default()
        };

        let persisted =
            persisted_ui_settings(current, 1024, 768, 500, TreeMode::Active, "", Some(42));

        assert_eq!(persisted.window.x, Some(100));
        assert_eq!(persisted.window.y, Some(120));
        assert_eq!(persisted.window.width, 1024);
        assert_eq!(persisted.window.height, 768);
        assert_eq!(persisted.splitter.left_width, 500);
        assert_eq!(persisted.selection.node_id, Some(42));
    }

    #[test]
    fn ui_settings_persist_preserves_selection_outside_active_browse_like_win32() {
        let current = UiSettings {
            selection: crate::domain::SelectionSettings { node_id: Some(7) },
            ..Default::default()
        };

        let search_persisted = persisted_ui_settings(
            current.clone(),
            1024,
            768,
            300,
            TreeMode::Active,
            "needle",
            Some(42),
        );
        let trash_persisted =
            persisted_ui_settings(current, 1024, 768, 300, TreeMode::Trash, "", Some(42));

        assert_eq!(search_persisted.selection.node_id, Some(7));
        assert_eq!(trash_persisted.selection.node_id, Some(7));
    }

    #[test]
    fn restored_trash_tab_becomes_editable_active_tab() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "deleted body")?;
            let draft_id = draft.id;
            repository.soft_delete_node_cascade(draft_id)?;

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let (deleted_content, deleted_updated_at) = app.load_deleted_node_content(draft_id)?;
            let deleted_node = app
                .deleted_nodes()?
                .into_iter()
                .find(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotDeleted { node_id: draft_id })?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: deleted_node.parent_id,
                title: deleted_node.title,
                content: deleted_content,
                loaded_updated_at: deleted_updated_at,
                editable: false,
                source: DocumentTabSource::Trash,
            });

            app.restore_node(draft_id)?;
            sync_tabs_from_active_document_local_metadata_for_tabs(&app, &mut tabs, true)?;

            let tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let restored = app
                .document()
                .node_by_id(draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert!(tab.editable);
            assert_eq!(tab.source, DocumentTabSource::ActiveTree);
            assert_eq!(tab.content, "deleted body");
            assert_eq!(tab.loaded_updated_at, restored.updated_at);
            assert!(!tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn active_tree_sync_marks_deleted_open_tab_read_only() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "cached body")?;
            let draft_id = draft.id;
            let draft_updated_at = draft.updated_at.clone();

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "cached body".to_owned(),
                loaded_updated_at: draft_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });

            app.delete_node(draft_id)?;
            let active_marked_read_only =
                sync_tabs_from_active_document_local_metadata_for_tabs(&app, &mut tabs, true)?;

            let tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert!(active_marked_read_only);
            assert!(!tab.editable);
            assert_eq!(tab.source, DocumentTabSource::ActiveTree);
            assert_eq!(tab.content, "cached body");
            assert!(!tab.is_save_target());
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn reloaded_active_tree_sync_reloads_stale_clean_inactive_tab() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let alpha =
                repository.create_document_with_content(ROOT_NODE_ID, "Alpha", "alpha body")?;
            let beta = repository.create_document_with_content(ROOT_NODE_ID, "Beta", "old beta")?;
            let alpha_id = alpha.id;
            let beta_id = beta.id;
            let alpha_updated_at = alpha.updated_at.clone();
            let beta_updated_at = beta.updated_at.clone();

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: beta_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Beta".to_owned(),
                content: "old beta".to_owned(),
                loaded_updated_at: beta_updated_at.clone(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: alpha_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Alpha".to_owned(),
                content: "cached current body".to_owned(),
                loaded_updated_at: alpha_updated_at.clone(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            let active_content_ptr = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: alpha_id })?
                .content
                .as_ptr();

            let mut external_repository = SqliteDocumentRepository::open(&db_path)?;
            let reloaded_beta_updated_at = external_repository.update_document_content(
                beta_id,
                "new beta",
                &beta_updated_at,
            )?;

            app.reload_document()?;
            sync_tabs_from_reloaded_active_document_metadata_for_tabs(
                &app,
                &mut tabs,
                true,
                Some(alpha_id),
            )?;

            let active_tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: alpha_id })?;
            assert_eq!(active_tab.node_id, alpha_id);
            assert_eq!(active_tab.content, "cached current body");
            assert_eq!(active_tab.content.as_ptr(), active_content_ptr);
            assert_eq!(active_tab.loaded_updated_at, alpha_updated_at);
            assert!(!active_tab.dirty);

            let beta_tab = tabs
                .tabs()
                .iter()
                .find(|tab| tab.node_id == beta_id)
                .ok_or(DomainError::NodeNotFound { node_id: beta_id })?;
            assert_eq!(beta_tab.content, "new beta");
            assert_eq!(beta_tab.loaded_content(), "new beta");
            assert_eq!(beta_tab.loaded_updated_at, reloaded_beta_updated_at);
            assert!(!beta_tab.dirty);
            assert!(beta_tab.editable);
            assert_eq!(beta_tab.source, DocumentTabSource::ActiveTree);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn rename_metadata_sync_advances_dirty_baseline_token() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let draft_updated_at = draft.updated_at.clone();

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "stored body".to_owned(),
                loaded_updated_at: draft_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            tabs.update_active_content("draft body".to_owned());

            app.rename_node(draft_id, "Renamed")?;
            sync_tabs_from_active_document_local_metadata_for_tabs(&app, &mut tabs, true)?;

            let tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let node = app
                .document()
                .node_by_id(draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title, "Renamed");
            assert_eq!(tab.content, "draft body");
            assert_eq!(tab.loaded_content(), "stored body");
            assert_eq!(tab.loaded_updated_at, node.updated_at);
            assert!(tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn move_metadata_sync_advances_dirty_baseline_token() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            repository.create_document_with_content(ROOT_NODE_ID, "Alpha", "alpha body")?;
            let beta =
                repository.create_document_with_content(ROOT_NODE_ID, "Beta", "stored body")?;
            let beta_id = beta.id;
            let beta_updated_at = beta.updated_at.clone();

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: beta_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Beta".to_owned(),
                content: "stored body".to_owned(),
                loaded_updated_at: beta_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            tabs.update_active_content("draft body".to_owned());

            app.move_node_within_parent(beta_id, SiblingMoveDirection::Up)?;
            sync_tabs_from_active_document_local_metadata_for_tabs(&app, &mut tabs, true)?;

            let tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: beta_id })?;
            let node = app
                .document()
                .node_by_id(beta_id)
                .ok_or(DomainError::NodeNotFound { node_id: beta_id })?;
            assert_eq!(tab.title, "Beta");
            assert_eq!(tab.content, "draft body");
            assert_eq!(tab.loaded_content(), "stored body");
            assert_eq!(tab.loaded_updated_at, node.updated_at);
            assert!(tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn drop_move_metadata_sync_advances_dirty_baseline_token() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let alpha = repository.create_document_with_content(ROOT_NODE_ID, "Alpha", "")?;
            let beta =
                repository.create_document_with_content(ROOT_NODE_ID, "Beta", "stored body")?;
            let beta_id = beta.id;
            let beta_updated_at = beta.updated_at.clone();

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: beta_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Beta".to_owned(),
                content: "stored body".to_owned(),
                loaded_updated_at: beta_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            tabs.update_active_content("draft body".to_owned());

            app.move_node_to_parent_end(beta_id, alpha.id)?;
            sync_tabs_from_active_document_local_metadata_for_tabs(&app, &mut tabs, true)?;

            let tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: beta_id })?;
            let node = app
                .document()
                .node_by_id(beta_id)
                .ok_or(DomainError::NodeNotFound { node_id: beta_id })?;
            assert_eq!(tab.parent_id, Some(alpha.id));
            assert_eq!(tab.content, "draft body");
            assert_eq!(tab.loaded_content(), "stored body");
            assert_eq!(tab.loaded_updated_at, node.updated_at);
            assert!(tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn delete_metadata_sync_advances_remaining_dirty_sibling_token() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path();
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let alpha =
                repository.create_document_with_content(ROOT_NODE_ID, "Alpha", "alpha body")?;
            let beta =
                repository.create_document_with_content(ROOT_NODE_ID, "Beta", "beta body")?;
            let alpha_id = alpha.id;
            let beta_id = beta.id;
            let alpha_updated_at = alpha.updated_at.clone();

            let mut app = App::from_repository_for_test(db_path.clone(), repository)?;
            let mut tabs = OpenTabs::new();
            tabs.open_or_activate(OpenDocumentTabInput {
                node_id: alpha_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Alpha".to_owned(),
                content: "alpha body".to_owned(),
                loaded_updated_at: alpha_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            tabs.update_active_content("draft alpha".to_owned());

            app.delete_node(beta_id)?;
            sync_tabs_from_active_document_local_metadata_for_tabs(&app, &mut tabs, true)?;

            let tab = tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: alpha_id })?;
            let node = app
                .document()
                .node_by_id(alpha_id)
                .ok_or(DomainError::NodeNotFound { node_id: alpha_id })?;
            assert_eq!(tab.title, "Alpha");
            assert_eq!(tab.content, "draft alpha");
            assert_eq!(tab.loaded_content(), "alpha body");
            assert_eq!(tab.loaded_updated_at, node.updated_at);
            assert!(tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }
}
