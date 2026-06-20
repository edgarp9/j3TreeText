use std::cell::{RefCell, RefMut};
use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_TAB_CLASSES, ICC_TREEVIEW_CLASSES, INITCOMMONCONTROLSEX,
    TCN_SELCHANGE, TCN_SELCHANGING, TVS_EDITLABELS, TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT,
    TVS_SHOWSELALWAYS, WC_TREEVIEWW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, IsDialogMessageW, LoadCursorW, LoadImageW, MessageBoxW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, CREATESTRUCTW, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL,
    ES_WANTRETURN, GA_ROOT, GA_ROOTOWNER, GWLP_USERDATA, HICON, HMENU, ICON_BIG, ICON_SMALL,
    IDC_ARROW, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, MB_ICONERROR, MB_OK, MSG, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE,
    WM_COMMAND, WM_CONTEXTMENU, WM_CREATE, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_IME_ENDCOMPOSITION,
    WM_IME_STARTCOMPOSITION, WM_INITMENUPOPUP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_NOTIFY, WM_SETICON, WM_SIZE, WM_TIMER, WNDCLASSW,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_HSCROLL, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use super::commands::{
    active_replace_dialog, close_active_tab_from_window, close_tab_at_index_from_window,
    create_child_document_from_selection, create_sibling_document_from_selection,
    delete_selected_node_from_keyboard, find_replace_message_id, handle_close, handle_command,
    handle_find_replace_message, handle_tab_selection_changed, handle_tab_selection_changing,
    handle_timer, move_selected_node_within_parent, move_tab_from_window,
    open_find_dialog_from_window, open_replace_dialog_from_window,
    pause_search_debounce_for_size_move, rename_selected_node,
    run_deferred_search_debounce_after_size_move, save_current_document_from_window,
    select_all_editor_text, update_window_title,
};
use super::common::{
    last_win32_error, last_win32_error_with_user_message, win32_error_with_user_message,
    APP_ICON_RESOURCE_ID, CARET_STATUS_CONTROL_CLASS_NAME, CONTROL_CARET_STATUS_ID,
    CONTROL_EDITOR_ID, CONTROL_SEARCH_ID, CONTROL_TAB_ID, CONTROL_TREE_ID,
    DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME, DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT, EMPTY_TEXT,
    EM_EXLIMITTEXT_RICH_EDIT, EM_SETCUEBANNER_EDIT_CONTROL, EM_SETEVENTMASK_RICH_EDIT,
    EM_SETTARGETDEVICE_RICH_EDIT, EM_SETTEXTMODE_RICH_EDIT, EM_SHOWSCROLLBAR_RICH_EDIT,
    ENM_CHANGE_RICH_EDIT, ENM_SELCHANGE_RICH_EDIT, EN_SELCHANGE_RICH_EDIT, RICH_EDIT_MODULE_NAME,
    SB_HORZ_SCROLLBAR, SEARCH_EDIT_CONTROL_CLASS_NAME, SS_RIGHT_STATIC_CONTROL,
    TM_PLAINTEXT_RICH_EDIT, VK_A_KEY, VK_CONTROL_KEY, VK_DELETE_KEY, VK_DOWN_KEY, VK_F2_KEY,
    VK_F_KEY, VK_H_KEY, VK_MENU_KEY, VK_N_KEY, VK_RETURN_KEY, VK_S_KEY, VK_UP_KEY, VK_W_KEY,
    WINDOW_CLASS_NAME, WM_APP_CLOSE_TAB, WM_APP_EDITOR_IME_END, WM_APP_EDITOR_IME_START,
    WM_APP_MOVE_TAB, WM_APP_REFRESH_TREE_AFTER_LABEL_EDIT,
};
use super::dpi::{
    enable_non_client_dpi_scaling, enable_process_dpi_awareness, suggested_rect_from_dpi_change,
    DpiMetrics,
};
use super::font::{create_editor_font, set_editor_font};
use super::i18n::{app_error_user_message, ui_text};
use super::layout::{
    begin_splitter_drag, finish_splitter_drag, initial_window_placement, layout_children,
    point_is_on_splitter, save_current_ui_settings, set_splitter_cursor, splitter_width_for_client,
    update_splitter_drag,
};
use super::menu::{handle_context_menu, set_main_menu, update_menu_state};
use super::state::{UiDocument, WindowState};
use super::tabs::create_tab_control;
use super::text::{utf8_to_wide_null, utf8_to_wide_null_lossy, ProgrammaticTextUpdateGuard};
use super::theme::{
    apply_window_theme, control_color_brush, control_color_brush_for_active_theme,
    erase_window_background, erase_window_background_for_active_theme,
};
use super::tree::{
    clear_tree_drag, handle_notify, handle_tree_drag_over, handle_tree_drop,
    notify_header_from_lparam, populate_tree, refresh_tree_after_label_edit, select_tree_node,
};
use crate::app::App;
use crate::domain::{SiblingMoveDirection, UiLanguage, UiSettings};
use crate::error::{AppError, PlatformUserMessage};
const EDITOR_CONTEXT_MENU_SUBCLASS_ID: usize = 1;
type WindowStateCell = RefCell<WindowState>;
pub(super) type WindowStateRef<'a> = RefMut<'a, WindowState>;
pub fn run_message_loop(app: App) -> Result<(), AppError> {
    enable_process_dpi_awareness();

    let class_name = utf8_to_wide_null("convert window class name", WINDOW_CLASS_NAME)?;
    let window_title = utf8_to_wide_null("convert window title", app.window_title())?;
    let ui_document = UiDocument::from_active_document(app.document())?;

    // SAFETY: Wide buffers are NUL-terminated and live for the registration/create calls. The
    // message loop runs on the same thread that creates the window.
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        if instance.is_null() {
            return Err(last_win32_error("get module handle"));
        }

        register_window_class(instance, &class_name)?;
        create_main_window(instance, &class_name, &window_title, app, ui_document)?;
        run_loop()
    }
}

pub fn show_error_message(title: &str, message: &str) {
    enable_process_dpi_awareness();

    let title = utf8_to_wide_null_lossy(title);
    let message = utf8_to_wide_null_lossy(message);

    // SAFETY: The title and message buffers are NUL-terminated UTF-16 and remain alive for the
    // synchronous MessageBoxW call.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub(super) fn show_app_error_for_language(language: UiLanguage, error: &AppError) {
    eprintln!("detail: {error}");
    show_error_message("j3TreeText", &app_error_user_message(error, language));
}

pub(super) unsafe fn show_app_error(hwnd: HWND, error: &AppError) {
    show_app_error_for_language(window_language(hwnd), error);
}

pub(super) unsafe fn window_language(hwnd: HWND) -> UiLanguage {
    window_state(hwnd)
        .map(|state| state.app.ui_settings().language)
        .unwrap_or_default()
}

unsafe fn register_window_class(instance: HINSTANCE, class_name: &[u16]) -> Result<(), AppError> {
    let cursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
    if cursor.is_null() {
        return Err(last_win32_error("load cursor"));
    }
    let icon = load_app_icon(
        instance,
        GetSystemMetrics(SM_CXICON),
        GetSystemMetrics(SM_CYICON),
    )?;

    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: icon,
        hCursor: cursor,
        hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };

    let atom = RegisterClassW(&window_class);
    if atom == 0 {
        let code = GetLastError();
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(win32_error_with_user_message(
                "register window class",
                PlatformUserMessage::Win32Startup,
                code,
            ));
        }
    }

    Ok(())
}

unsafe fn create_main_window(
    instance: HINSTANCE,
    class_name: &[u16],
    window_title: &[u16],
    app: App,
    document: UiDocument,
) -> Result<HWND, AppError> {
    let startup_dpi = DpiMetrics::system();
    let mut ui_settings = app.ui_settings();
    scale_default_startup_ui_settings(&mut ui_settings, startup_dpi);
    let placement = initial_window_placement(ui_settings.window);
    let mut state = Box::new(RefCell::new(WindowState::new(
        app,
        document,
        ui_settings,
        startup_dpi,
    )?));
    let state_ptr = state.as_ref() as *const WindowStateCell as *mut WindowStateCell;

    // SAFETY: state_ptr points to a boxed WindowStateCell that is kept alive after successful
    // window creation. WM_NCCREATE stores it in GWLP_USERDATA before later message handling reads
    // it.
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        window_title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
        placement.x,
        placement.y,
        placement.width,
        placement.height,
        ptr::null_mut(),
        ptr::null_mut(),
        instance,
        state_ptr.cast::<c_void>() as *const c_void,
    );

    if hwnd.is_null() {
        return Err(last_win32_error_with_user_message(
            "create main window",
            PlatformUserMessage::Win32Startup,
        ));
    }

    state.get_mut().drop_on_destroy = true;
    let _ = Box::into_raw(state);

    set_main_window_icons(hwnd, instance)?;
    ShowWindow(hwnd, SW_SHOW);
    Ok(hwnd)
}

fn scale_default_startup_ui_settings(settings: &mut UiSettings, metrics: DpiMetrics) {
    if settings.window.x.is_some() && settings.window.y.is_some() {
        return;
    }

    let scale = metrics.ui_scale();
    settings.window.width = scale.px(settings.window.width);
    settings.window.height = scale.px(settings.window.height);
    settings.splitter.left_width = scale.px(settings.splitter.left_width);
}

unsafe fn set_main_window_icons(hwnd: HWND, instance: HINSTANCE) -> Result<(), AppError> {
    let big_icon = load_app_icon(
        instance,
        GetSystemMetrics(SM_CXICON),
        GetSystemMetrics(SM_CYICON),
    )?;
    let small_icon = load_app_icon(
        instance,
        GetSystemMetrics(SM_CXSMICON),
        GetSystemMetrics(SM_CYSMICON),
    )?;

    SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, big_icon as LPARAM);
    SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, small_icon as LPARAM);
    Ok(())
}

unsafe fn load_app_icon(instance: HINSTANCE, width: i32, height: i32) -> Result<HICON, AppError> {
    let icon = LoadImageW(
        instance,
        app_icon_resource_name(),
        IMAGE_ICON,
        width,
        height,
        LR_DEFAULTCOLOR | LR_SHARED,
    ) as HICON;

    if icon.is_null() {
        return Err(last_win32_error("load application icon"));
    }

    Ok(icon)
}

fn app_icon_resource_name() -> *const u16 {
    APP_ICON_RESOURCE_ID as usize as *const u16
}

unsafe fn create_child_controls(
    parent: HWND,
    instance: HINSTANCE,
    state: &mut WindowState,
) -> Result<(), AppError> {
    init_common_controls()?;

    state.search = create_search_edit(parent, instance, state.app.ui_settings().language)?;
    state.tree = create_tree_view(parent, instance)?;
    state.tab_bar = create_tab_control(parent, instance)?;
    state.editor = create_document_editor_rich_edit_control(
        parent,
        instance,
        state.app.ui_settings().editor.word_wrap,
    )?;
    state.caret_status = create_caret_status_control(parent, instance)?;
    subclass_editor_context_menu(state.editor, parent)?;
    apply_initial_editor_font(state)?;
    apply_window_theme(parent, state);
    layout_children(parent, state);

    let initial_selection = populate_tree(
        state.tree,
        &state.document,
        state.app.ui_settings().selection.node_id,
    )?;
    select_tree_node(parent, state, initial_selection)?;
    update_menu_state(parent, state)?;
    update_window_title(parent, state)?;
    save_current_ui_settings(parent, state)?;

    Ok(())
}

unsafe fn init_common_controls() -> Result<(), AppError> {
    let init = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TREEVIEW_CLASSES | ICC_TAB_CLASSES,
    };

    if InitCommonControlsEx(&init) == 0 {
        return Err(last_win32_error("initialize common controls"));
    }

    Ok(())
}

unsafe fn ensure_document_editor_rich_edit_module_loaded() -> Result<(), AppError> {
    let library_name = utf8_to_wide_null("convert Rich Edit module name", RICH_EDIT_MODULE_NAME)?;
    let existing = GetModuleHandleW(library_name.as_ptr());
    if !existing.is_null() {
        return Ok(());
    }

    let module = LoadLibraryW(library_name.as_ptr());
    if module.is_null() {
        return Err(last_win32_error_with_user_message(
            "load Rich Edit library",
            PlatformUserMessage::RichEditStartup,
        ));
    }

    Ok(())
}

unsafe fn create_search_edit(
    parent: HWND,
    instance: HINSTANCE,
    language: UiLanguage,
) -> Result<HWND, AppError> {
    let class_name = utf8_to_wide_null(
        "convert search edit control class name",
        SEARCH_EDIT_CONTROL_CLASS_NAME,
    )?;
    let hwnd = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        class_name.as_ptr(),
        EMPTY_TEXT.as_ptr(),
        search_edit_style(),
        0,
        0,
        0,
        0,
        parent,
        CONTROL_SEARCH_ID as HMENU,
        instance,
        ptr::null(),
    );

    if hwnd.is_null() {
        return Err(last_win32_error("create search edit control"));
    }

    set_search_cue_text(hwnd, language)?;

    Ok(hwnd)
}

pub(super) unsafe fn set_search_cue_text(
    search: HWND,
    language: UiLanguage,
) -> Result<(), AppError> {
    let cue = utf8_to_wide_null("convert search cue text", ui_text(language).search_cue())?;
    SendMessageW(
        search,
        EM_SETCUEBANNER_EDIT_CONTROL,
        0,
        cue.as_ptr() as LPARAM,
    );
    Ok(())
}

fn search_edit_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | ES_AUTOHSCROLL as u32
}

unsafe fn create_caret_status_control(parent: HWND, instance: HINSTANCE) -> Result<HWND, AppError> {
    let class_name = utf8_to_wide_null(
        "convert caret status control class name",
        CARET_STATUS_CONTROL_CLASS_NAME,
    )?;
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        EMPTY_TEXT.as_ptr(),
        caret_status_style(),
        0,
        0,
        0,
        0,
        parent,
        CONTROL_CARET_STATUS_ID as HMENU,
        instance,
        ptr::null(),
    );

    if hwnd.is_null() {
        return Err(last_win32_error("create caret status control"));
    }

    Ok(hwnd)
}

fn caret_status_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | SS_RIGHT_STATIC_CONTROL
}

unsafe fn create_tree_view(parent: HWND, instance: HINSTANCE) -> Result<HWND, AppError> {
    let style = WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | WS_CLIPSIBLINGS
        | WS_VSCROLL
        | TVS_HASBUTTONS
        | TVS_HASLINES
        | TVS_LINESATROOT
        | TVS_SHOWSELALWAYS
        | TVS_EDITLABELS;

    let hwnd = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        WC_TREEVIEWW,
        EMPTY_TEXT.as_ptr(),
        style,
        0,
        0,
        0,
        0,
        parent,
        CONTROL_TREE_ID as HMENU,
        instance,
        ptr::null(),
    );

    if hwnd.is_null() {
        return Err(last_win32_error("create tree view"));
    }

    Ok(hwnd)
}

unsafe fn create_document_editor_rich_edit_control(
    parent: HWND,
    instance: HINSTANCE,
    word_wrap: bool,
) -> Result<HWND, AppError> {
    ensure_document_editor_rich_edit_module_loaded()?;
    let class_name = utf8_to_wide_null(
        "convert document editor Rich Edit class name",
        DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME,
    )?;
    let hwnd = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        class_name.as_ptr(),
        EMPTY_TEXT.as_ptr(),
        document_editor_rich_edit_style(word_wrap),
        0,
        0,
        0,
        0,
        parent,
        CONTROL_EDITOR_ID as HMENU,
        instance,
        ptr::null(),
    );

    if hwnd.is_null() {
        return Err(last_win32_error("create document editor Rich Edit control"));
    }

    if let Err(error) = configure_document_editor_plain_text_rich_edit(hwnd, word_wrap) {
        DestroyWindow(hwnd);
        return Err(error);
    }

    Ok(hwnd)
}

unsafe fn configure_document_editor_plain_text_rich_edit(
    hwnd: HWND,
    word_wrap: bool,
) -> Result<(), AppError> {
    let text_mode_result = SendMessageW(hwnd, EM_SETTEXTMODE_RICH_EDIT, TM_PLAINTEXT_RICH_EDIT, 0);
    if text_mode_result != 0 {
        return Err(AppError::platform(
            "set document editor Rich Edit plain text mode",
            format!("Rich Edit rejected plain text mode with result {text_mode_result}"),
        ));
    }

    SendMessageW(
        hwnd,
        EM_EXLIMITTEXT_RICH_EDIT,
        0,
        DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT as LPARAM,
    );
    configure_document_editor_word_wrap(hwnd, word_wrap)?;
    SendMessageW(
        hwnd,
        EM_SETEVENTMASK_RICH_EDIT,
        0,
        ENM_CHANGE_RICH_EDIT | ENM_SELCHANGE_RICH_EDIT,
    );
    Ok(())
}

unsafe fn configure_document_editor_word_wrap(hwnd: HWND, word_wrap: bool) -> Result<(), AppError> {
    let line_width: LPARAM = if word_wrap { 0 } else { 1 };
    let result = SendMessageW(hwnd, EM_SETTARGETDEVICE_RICH_EDIT, 0, line_width);
    if result == 0 {
        return Err(AppError::platform(
            "set document editor Rich Edit word wrap",
            "Rich Edit rejected target-device formatting width",
        ));
    }

    let show_horizontal_scrollbar: LPARAM = if word_wrap { 0 } else { 1 };
    SendMessageW(
        hwnd,
        EM_SHOWSCROLLBAR_RICH_EDIT,
        SB_HORZ_SCROLLBAR,
        show_horizontal_scrollbar,
    );
    Ok(())
}

unsafe fn apply_initial_editor_font(state: &mut WindowState) -> Result<(), AppError> {
    let requested = state.app.ui_settings().editor_font;
    let applied = create_editor_font(state.editor, &requested)?;

    if applied.settings != requested {
        let mut settings = state.app.ui_settings();
        settings.editor_font = applied.settings.clone();
        state.app.save_ui_settings(settings)?;
    }

    set_editor_font(state.editor, &state.suppress_editor_change, &applied.handle);
    state.editor_font_handle = Some(applied.handle);
    Ok(())
}

pub(super) unsafe fn recreate_document_editor_for_word_wrap(
    parent: HWND,
    state: &mut WindowState,
    word_wrap: bool,
) -> Result<(), AppError> {
    let instance = GetModuleHandleW(ptr::null());
    if instance.is_null() {
        return Err(last_win32_error("get module handle"));
    }

    let new_editor = create_document_editor_rich_edit_control(parent, instance, word_wrap)?;
    let old_editor = state.editor;

    if let Err(error) = subclass_editor_context_menu(new_editor, parent) {
        destroy_editor_control(
            state,
            new_editor,
            "destroy new editor after subclass failure",
        )?;
        return Err(error);
    }

    let mut created_font_handle = None;
    if let Some(handle) = state.editor_font_handle.as_ref() {
        set_editor_font(new_editor, &state.suppress_editor_change, handle);
    } else {
        let requested = state.app.ui_settings().editor_font;
        let applied = match create_editor_font(new_editor, &requested) {
            Ok(applied) => applied,
            Err(error) => {
                destroy_editor_control(
                    state,
                    new_editor,
                    "destroy new editor after font creation failure",
                )?;
                return Err(error);
            }
        };

        if applied.settings != requested {
            let mut settings = state.app.ui_settings();
            settings.editor_font = applied.settings.clone();
            if let Err(error) = state.app.save_ui_settings(settings) {
                destroy_editor_control(
                    state,
                    new_editor,
                    "destroy new editor after font settings save failure",
                )?;
                return Err(error);
            }
        }

        set_editor_font(new_editor, &state.suppress_editor_change, &applied.handle);
        created_font_handle = Some(applied.handle);
    }

    state.editor = new_editor;

    if let Err(error) = state.show_active_tab_in_editor() {
        state.editor = old_editor;
        destroy_editor_control(
            state,
            new_editor,
            "destroy new editor after active tab load failure",
        )?;
        return Err(error);
    }

    if let Err(error) = destroy_editor_control(state, old_editor, "destroy old editor control") {
        state.editor = old_editor;
        destroy_editor_control(
            state,
            new_editor,
            "destroy new editor after old editor destroy failure",
        )?;
        return Err(error);
    }

    if let Some(handle) = created_font_handle {
        state.editor_font_handle = Some(handle);
    }

    apply_window_theme(parent, state);
    layout_children(parent, state);
    Ok(())
}

unsafe fn destroy_editor_control(
    state: &mut WindowState,
    editor: HWND,
    action: &'static str,
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    let destroyed = {
        let _guard = ProgrammaticTextUpdateGuard::enter(&state.suppress_editor_change);
        DestroyWindow(editor) != 0
    };

    if !destroyed {
        return Err(last_win32_error(action));
    }

    Ok(())
}

fn document_editor_rich_edit_style(word_wrap: bool) -> u32 {
    let mut style = WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | WS_CLIPSIBLINGS
        | WS_VSCROLL
        | ES_MULTILINE as u32
        | ES_AUTOVSCROLL as u32
        | ES_WANTRETURN as u32
        | ES_NOHIDESEL as u32;

    if !word_wrap {
        style |= WS_HSCROLL | ES_AUTOHSCROLL as u32;
    }

    style
}

unsafe fn subclass_editor_context_menu(editor: HWND, parent: HWND) -> Result<(), AppError> {
    if SetWindowSubclass(
        editor,
        Some(editor_context_menu_subclass_proc),
        EDITOR_CONTEXT_MENU_SUBCLASS_ID,
        parent as usize,
    ) == 0
    {
        return Err(last_win32_error("subclass editor context menu"));
    }

    Ok(())
}

unsafe extern "system" fn editor_context_menu_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    parent: usize,
) -> LRESULT {
    if message == WM_CONTEXTMENU {
        let parent = parent as HWND;
        if !parent.is_null() {
            SendMessageW(parent, WM_CONTEXTMENU, hwnd as WPARAM, lparam);
            return 0;
        }
    }

    if message == WM_IME_STARTCOMPOSITION {
        let parent = parent as HWND;
        if !parent.is_null() {
            SendMessageW(parent, WM_APP_EDITOR_IME_START, hwnd as WPARAM, 0);
        }
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }

    if message == WM_IME_ENDCOMPOSITION {
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        let parent = parent as HWND;
        if !parent.is_null() {
            SendMessageW(parent, WM_APP_EDITOR_IME_END, hwnd as WPARAM, 0);
        }
        return result;
    }

    DefSubclassProc(hwnd, message, wparam, lparam)
}

unsafe fn run_loop() -> Result<(), AppError> {
    // SAFETY: MSG is a plain Win32 POD struct, and an all-zero value is valid before GetMessageW
    // initializes it.
    let mut message: MSG = std::mem::zeroed();

    loop {
        let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
        if result == -1 {
            return Err(last_win32_error("read Windows message"));
        }
        if result == 0 {
            return Ok(());
        }

        if replace_dialog_handles_message(&message) {
            continue;
        }

        if handle_keyboard_shortcut(&message) {
            continue;
        }

        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

unsafe fn replace_dialog_handles_message(message: &MSG) -> bool {
    if message.hwnd.is_null() {
        return false;
    }

    let hwnd = GetAncestor(message.hwnd, GA_ROOTOWNER);
    if hwnd.is_null() {
        return false;
    }

    let Some(dialog) = active_replace_dialog(hwnd) else {
        return false;
    };

    IsDialogMessageW(dialog, message) != 0
}

unsafe fn handle_keyboard_shortcut(message: &MSG) -> bool {
    if message.message != WM_KEYDOWN || message.hwnd.is_null() {
        return false;
    }

    let hwnd = GetAncestor(message.hwnd, GA_ROOT);
    if hwnd.is_null() || window_state(hwnd).is_none() {
        return false;
    }

    let is_ctrl_down = GetKeyState(VK_CONTROL_KEY) < 0;
    let is_alt_down = GetKeyState(VK_MENU_KEY) < 0;
    let result = match message.wParam {
        VK_A_KEY
            if is_ctrl_down && !is_alt_down && shortcut_origin_is_editor(hwnd, message.hwnd) =>
        {
            Some(select_all_editor_text(hwnd))
        }
        VK_S_KEY if is_ctrl_down => Some(save_current_document_from_window(hwnd)),
        VK_N_KEY if is_ctrl_down => Some(create_sibling_document_from_selection(hwnd)),
        VK_W_KEY if is_ctrl_down => Some(close_active_tab_from_window(hwnd)),
        VK_F_KEY if is_ctrl_down => Some(open_find_dialog_from_window(hwnd)),
        VK_H_KEY if is_ctrl_down => Some(open_replace_dialog_from_window(hwnd)),
        VK_RETURN_KEY
            if is_ctrl_down && !is_alt_down && shortcut_origin_is_tree(hwnd, message.hwnd) =>
        {
            Some(create_child_document_from_selection(hwnd))
        }
        VK_RETURN_KEY
            if !is_ctrl_down && !is_alt_down && shortcut_origin_is_tree(hwnd, message.hwnd) =>
        {
            Some(create_sibling_document_from_selection(hwnd))
        }
        VK_F2_KEY if shortcut_origin_is_tree(hwnd, message.hwnd) => {
            Some(rename_selected_node(hwnd))
        }
        VK_DELETE_KEY if shortcut_origin_is_tree(hwnd, message.hwnd) => {
            Some(delete_selected_node_from_keyboard(hwnd))
        }
        VK_UP_KEY if is_ctrl_down && shortcut_origin_is_tree(hwnd, message.hwnd) => Some(
            move_selected_node_within_parent(hwnd, SiblingMoveDirection::Up),
        ),
        VK_DOWN_KEY if is_ctrl_down && shortcut_origin_is_tree(hwnd, message.hwnd) => Some(
            move_selected_node_within_parent(hwnd, SiblingMoveDirection::Down),
        ),
        _ => None,
    };

    if let Some(result) = result {
        if let Err(error) = result {
            show_app_error(hwnd, &error);
        }
        true
    } else {
        false
    }
}

unsafe fn shortcut_origin_is_tree(hwnd: HWND, origin: HWND) -> bool {
    match window_state(hwnd) {
        Some(state) => origin == state.tree,
        None => false,
    }
}

unsafe fn shortcut_origin_is_editor(hwnd: HWND, origin: HWND) -> bool {
    match window_state(hwnd) {
        Some(state) => origin == state.editor,
        None => false,
    }
}

unsafe fn handle_notify_message(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if wparam == CONTROL_EDITOR_ID {
        if let Some(header) = notify_header_from_lparam(lparam) {
            let Some(mut state) = window_state(hwnd) else {
                return 0;
            };
            if (*header).hwndFrom != state.editor {
                return 0;
            }

            if (*header).code == EN_SELCHANGE_RICH_EDIT && !state.suppress_editor_change.get() {
                let result = state
                    .clear_current_find_match_highlight()
                    .and_then(|_| state.update_caret_status_from_editor());
                if let Err(error) = result {
                    show_app_error_for_language(state.app.ui_settings().language, &error);
                }
            }
        }
        return 0;
    }

    if wparam == CONTROL_TAB_ID {
        if let Some(header) = notify_header_from_lparam(lparam) {
            // SAFETY: notify_header_from_lparam checked that lparam covers an NMHDR before this
            // message is accepted as a tab notification.
            let tab_notification = window_state(hwnd)
                .map(|state| (*header).hwndFrom == state.tab_bar)
                .unwrap_or(false);
            if !tab_notification {
                return 0;
            }

            match (*header).code {
                TCN_SELCHANGING => match handle_tab_selection_changing(hwnd) {
                    Ok(true) => return 0,
                    Ok(false) => return 1,
                    Err(error) => {
                        show_app_error(hwnd, &error);
                        return 1;
                    }
                },
                TCN_SELCHANGE => {
                    if let Err(error) = handle_tab_selection_changed(hwnd) {
                        show_app_error(hwnd, &error);
                    }
                }
                _ => {}
            }
        }
        return 0;
    }

    handle_notify(hwnd, wparam, lparam)
}

unsafe fn handle_dpi_changed(hwnd: HWND, lparam: LPARAM) {
    let Some(mut state) = window_state(hwnd) else {
        return;
    };

    if state.size_move.in_loop() {
        state.size_move.defer_dpi_change();
        return;
    }

    if let Some(rect) = suggested_rect_from_dpi_change(lparam) {
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }

    apply_dpi_dependent_options(hwnd, &mut state);
}

unsafe fn apply_dpi_dependent_options(hwnd: HWND, state: &mut WindowState) {
    let metrics = DpiMetrics::for_window(hwnd);
    if state.dpi_metrics == metrics {
        return;
    }

    state.set_dpi_metrics(metrics);
    state.split_width = splitter_width_for_client(hwnd, state);
    if let Err(error) = apply_initial_editor_font(state) {
        show_app_error_for_language(state.app.ui_settings().language, &error);
    }
    apply_window_theme(hwnd, state);
    layout_children(hwnd, state);
    if let Err(error) = state.update_caret_status_from_editor() {
        show_app_error_for_language(state.app.ui_settings().language, &error);
    }
}

unsafe fn handle_enter_size_move(hwnd: HWND) {
    if let Some(mut state) = window_state(hwnd) {
        state.size_move.enter();
        if let Err(error) = pause_search_debounce_for_size_move(hwnd, &mut state) {
            show_app_error_for_language(state.app.ui_settings().language, &error);
        }
    }
}

unsafe fn handle_exit_size_move(hwnd: HWND) {
    if let Some(mut state) = window_state(hwnd) {
        let exit = state.size_move.exit();
        if exit.dpi_changed {
            apply_dpi_dependent_options(hwnd, &mut state);
        }
        if exit.search_debounce_pending {
            if let Err(error) = run_deferred_search_debounce_after_size_move(hwnd, &mut state) {
                show_app_error_for_language(state.app.ui_settings().language, &error);
            }
        }
        if let Err(error) = save_current_ui_settings(hwnd, &mut state) {
            show_app_error_for_language(state.app.ui_settings().language, &error);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let find_replace_message = find_replace_message_id();
    if find_replace_message != 0 && message == find_replace_message {
        if let Err(error) = handle_find_replace_message(hwnd, lparam) {
            show_app_error(hwnd, &error);
        }
        return 0;
    }

    match message {
        WM_NCCREATE => {
            enable_non_client_dpi_scaling(hwnd);
            if store_window_state(hwnd, lparam) {
                1
            } else {
                0
            }
        }
        WM_CREATE => match handle_create(hwnd, lparam) {
            Ok(()) => 0,
            Err(error) => {
                show_app_error(hwnd, &error);
                -1
            }
        },
        WM_ERASEBKGND => {
            if let Some(state) = window_state(hwnd) {
                if let Some(result) = erase_window_background(hwnd, &state, wparam) {
                    return result;
                }
            } else if let Some(result) = erase_window_background_for_active_theme(hwnd, wparam) {
                return result;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some(state) = window_state(hwnd) {
                if let Some(result) = control_color_brush(&state, wparam) {
                    return result;
                }
            } else if let Some(result) = control_color_brush_for_active_theme(wparam) {
                return result;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_DPICHANGED => {
            handle_dpi_changed(hwnd, lparam);
            0
        }
        WM_SIZE => {
            if let Some(mut state) = window_state(hwnd) {
                if state.size_move.in_loop() && state.size_move.dpi_changed() {
                    return 0;
                }
                layout_children(hwnd, &state);
                if let Err(error) = state.update_caret_status_from_editor() {
                    show_app_error_for_language(state.app.ui_settings().language, &error);
                }
            }
            0
        }
        WM_ENTERSIZEMOVE => {
            handle_enter_size_move(hwnd);
            0
        }
        WM_EXITSIZEMOVE => {
            handle_exit_size_move(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            if let Some(mut state) = window_state(hwnd) {
                if begin_splitter_drag(hwnd, &mut state, lparam) {
                    return 0;
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            if let Some(mut state) = window_state(hwnd) {
                if state.dragging_splitter {
                    update_splitter_drag(hwnd, &mut state, lparam);
                    set_splitter_cursor();
                    return 0;
                }
                if point_is_on_splitter(hwnd, &state, lparam) {
                    set_splitter_cursor();
                }
                handle_tree_drag_over(&mut state, lparam);
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(mut state) = window_state(hwnd) {
                if state.dragging_splitter {
                    finish_splitter_drag(hwnd, &mut state, lparam);
                    if let Err(error) = state.update_caret_status_from_editor() {
                        show_app_error_for_language(state.app.ui_settings().language, &error);
                    }
                    if let Err(error) = save_current_ui_settings(hwnd, &mut state) {
                        show_app_error_for_language(state.app.ui_settings().language, &error);
                    }
                    return 0;
                }
                if state.dragging_node_id.is_some() {
                    if let Err(error) = handle_tree_drop(hwnd, &mut state, lparam) {
                        clear_tree_drag(&mut state);
                        show_app_error_for_language(state.app.ui_settings().language, &error);
                    }
                    return 0;
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_APP_EDITOR_IME_START => {
            if let Some(mut state) = window_state(hwnd) {
                if wparam as HWND == state.editor {
                    state.start_editor_ime_composition();
                }
            }
            0
        }
        WM_APP_EDITOR_IME_END => {
            if let Some(mut state) = window_state(hwnd) {
                if wparam as HWND == state.editor {
                    if let Err(error) = state.finish_editor_ime_composition() {
                        show_app_error_for_language(state.app.ui_settings().language, &error);
                    }
                }
            }
            0
        }
        WM_APP_CLOSE_TAB => {
            if let Err(error) = close_tab_at_index_from_window(hwnd, wparam) {
                show_app_error(hwnd, &error);
            }
            0
        }
        WM_APP_MOVE_TAB => {
            let to_index = usize::try_from(lparam).ok();
            if let Some(to_index) = to_index {
                if let Err(error) = move_tab_from_window(hwnd, wparam, to_index) {
                    show_app_error(hwnd, &error);
                }
            }
            0
        }
        WM_APP_REFRESH_TREE_AFTER_LABEL_EDIT => {
            if let Some(mut state) = window_state(hwnd) {
                let preferred_node_id = state
                    .pending_tree_label_edit_refresh_node_id
                    .take()
                    .or(state.selected_node_id);
                if let Err(error) =
                    refresh_tree_after_label_edit(hwnd, &mut state, preferred_node_id)
                {
                    show_app_error_for_language(state.app.ui_settings().language, &error);
                }
            }
            0
        }
        WM_COMMAND => {
            handle_command(hwnd, wparam);
            0
        }
        WM_INITMENUPOPUP => {
            if let Some(state) = window_state(hwnd) {
                if let Err(error) = update_menu_state(hwnd, &state) {
                    show_app_error_for_language(state.app.ui_settings().language, &error);
                }
            }
            0
        }
        WM_TIMER => {
            if handle_timer(hwnd, wparam) {
                0
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_CONTEXTMENU => match handle_context_menu(hwnd, wparam, lparam) {
            Ok(true) => 0,
            Ok(false) => DefWindowProcW(hwnd, message, wparam, lparam),
            Err(error) => {
                show_app_error(hwnd, &error);
                0
            }
        },
        WM_NOTIFY => handle_notify_message(hwnd, wparam, lparam),
        WM_CLOSE => {
            match handle_close(hwnd) {
                Ok(true) => {
                    DestroyWindow(hwnd);
                }
                Ok(false) => {}
                Err(error) => {
                    show_app_error(hwnd, &error);
                }
            }
            0
        }
        WM_DESTROY => {
            if let Some(mut state) = window_state(hwnd) {
                if let Err(error) = save_current_ui_settings(hwnd, &mut state) {
                    show_app_error_for_language(state.app.ui_settings().language, &error);
                }
            }
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            release_window_state(hwnd);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn store_window_state(hwnd: HWND, lparam: LPARAM) -> bool {
    // SAFETY: WM_NCCREATE provides lparam as a valid CREATESTRUCTW pointer for this callback.
    let create = lparam as *const CREATESTRUCTW;
    if create.is_null() {
        return false;
    }

    let state_ptr = (*create).lpCreateParams.cast::<WindowStateCell>();
    if state_ptr.is_null() {
        return false;
    }

    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
    true
}

unsafe fn handle_create(hwnd: HWND, lparam: LPARAM) -> Result<(), AppError> {
    // SAFETY: WM_CREATE provides lparam as a valid CREATESTRUCTW pointer for this callback.
    let create = lparam as *const CREATESTRUCTW;
    if create.is_null() {
        return Err(AppError::platform_with_user_message(
            "create main window",
            PlatformUserMessage::Win32Startup,
            "WM_CREATE did not include CREATESTRUCTW",
        ));
    }

    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform_with_user_message(
            "create main window",
            PlatformUserMessage::Win32Startup,
            "window state was not attached",
        )
    })?;
    state.set_dpi_metrics(DpiMetrics::for_window(hwnd));
    set_main_menu(hwnd, state.app.ui_settings().language)?;
    update_menu_state(hwnd, &state)?;
    create_child_controls(hwnd, (*create).hInstance, &mut state)
}

pub(super) unsafe fn window_state(hwnd: HWND) -> Option<WindowStateRef<'static>> {
    // SAFETY: GWLP_USERDATA is set only from a Box<RefCell<WindowState>> in store_window_state and
    // cleared in release_window_state. RefCell keeps reentrant Win32 callbacks from constructing a
    // second mutable WindowState borrow while the first borrow is still live on the UI thread.
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowStateCell;
    state_ptr.as_ref()?.try_borrow_mut().ok()
}

unsafe fn release_window_state(hwnd: HWND) {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowStateCell;
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);

    if !state_ptr.is_null() && (*state_ptr).get_mut().drop_on_destroy {
        // SAFETY: This pointer came from Box::into_raw after CreateWindowExW succeeded, and this
        // path runs once after GWLP_USERDATA is cleared during WM_NCDESTROY.
        drop(Box::from_raw(state_ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::super::common::{
        DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME, DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT, EMPTY_TEXT,
        EM_SETTEXTMODE_RICH_EDIT, ENM_CHANGE_RICH_EDIT, ENM_SELCHANGE_RICH_EDIT,
        RICH_EDIT_MODULE_NAME, TM_PLAINTEXT_RICH_EDIT,
    };
    use super::super::test_support::{enter_win32_control_test, Win32ControlTestGuard};
    use super::super::text::utf8_to_wide_null;
    use super::*;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, SetWindowTextW, ES_MULTILINE, WS_OVERLAPPED,
    };

    const EM_GETEVENTMASK_RICH_EDIT: u32 = 0x043B;
    const EM_GETLINECOUNT_EDIT_CONTROL: u32 = 0x00BA;
    const EM_GETLIMITTEXT_EDIT_CONTROL: u32 = 0x00D5;
    const EM_GETTEXTMODE_RICH_EDIT: u32 = 0x045A;

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

    fn compact(source: &str) -> String {
        source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    fn assert_body_contains_all(body: &str, expectations: &[&str]) {
        let body = compact(body);
        for expectation in expectations {
            let expectation = compact(expectation);
            assert!(
                body.contains(&expectation),
                "expected function body to contain `{expectation}`"
            );
        }
    }

    struct TestRichEdit(HWND, #[allow(dead_code)] Win32ControlTestGuard);

    impl TestRichEdit {
        unsafe fn create(width: i32) -> Self {
            let editor = Self::create_raw(width);
            set_test_rich_edit_plain_text_mode(editor.0);
            editor
        }

        unsafe fn create_raw(width: i32) -> Self {
            let guard = enter_win32_control_test();
            let module = utf8_to_wide_null("test Rich Edit module name", RICH_EDIT_MODULE_NAME)
                .expect("Rich Edit module name should convert");
            if GetModuleHandleW(module.as_ptr()).is_null()
                && LoadLibraryW(module.as_ptr()).is_null()
            {
                panic!("Rich Edit module should load");
            }

            let class_name = utf8_to_wide_null(
                "test Rich Edit class name",
                DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME,
            )
            .expect("Rich Edit class name should convert");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                EMPTY_TEXT.as_ptr(),
                WS_OVERLAPPED | ES_MULTILINE as u32,
                0,
                0,
                width,
                80,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                panic!("test Rich Edit control should be created");
            }

            Self(hwnd, guard)
        }

        unsafe fn set_text(&self, text: &str) {
            let text = utf8_to_wide_null("test Rich Edit text", text)
                .expect("Rich Edit text should convert");
            if SetWindowTextW(self.0, text.as_ptr()) == 0 {
                panic!("test Rich Edit text should be set");
            }
        }

        unsafe fn line_count(&self) -> usize {
            let count = SendMessageW(self.0, EM_GETLINECOUNT_EDIT_CONTROL, 0, 0);
            usize::try_from(count).expect("line count should be non-negative")
        }
    }

    unsafe fn set_test_rich_edit_plain_text_mode(hwnd: HWND) {
        let text_mode_result =
            SendMessageW(hwnd, EM_SETTEXTMODE_RICH_EDIT, TM_PLAINTEXT_RICH_EDIT, 0);
        if text_mode_result != 0 {
            DestroyWindow(hwnd);
            panic!("test Rich Edit control should accept plain text mode");
        }
    }

    impl Drop for TestRichEdit {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn document_editor_word_wrap_configuration_matches_visual_line_wrapping() {
        unsafe {
            let wrapped = TestRichEdit::create(120);
            configure_document_editor_word_wrap(wrapped.0, true)
                .expect("word wrap should configure");
            wrapped.set_text("alpha beta gamma delta epsilon zeta eta theta iota kappa");

            let unwrapped = TestRichEdit::create(120);
            configure_document_editor_word_wrap(unwrapped.0, false)
                .expect("no word wrap should configure");
            unwrapped.set_text("alpha beta gamma delta epsilon zeta eta theta iota kappa");

            assert!(
                wrapped.line_count() > unwrapped.line_count(),
                "enabled word wrap should create more visual lines than disabled word wrap"
            );
        }
    }

    #[test]
    fn document_editor_rich_edit_configuration_sets_plain_text_limits_and_change_events() {
        unsafe {
            let editor = TestRichEdit::create_raw(120);
            configure_document_editor_plain_text_rich_edit(editor.0, true)
                .expect("Rich Edit configuration should succeed");

            let text_mode = SendMessageW(editor.0, EM_GETTEXTMODE_RICH_EDIT, 0, 0) as usize;
            assert_eq!(text_mode & TM_PLAINTEXT_RICH_EDIT, TM_PLAINTEXT_RICH_EDIT);

            let event_mask = SendMessageW(editor.0, EM_GETEVENTMASK_RICH_EDIT, 0, 0);
            assert_eq!(event_mask & ENM_CHANGE_RICH_EDIT, ENM_CHANGE_RICH_EDIT);
            assert_eq!(
                event_mask & ENM_SELCHANGE_RICH_EDIT,
                ENM_SELCHANGE_RICH_EDIT
            );

            let text_limit = SendMessageW(editor.0, EM_GETLIMITTEXT_EDIT_CONTROL, 0, 0) as usize;
            assert_eq!(text_limit, DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT);
        }
    }

    #[test]
    fn win32_global_shortcut_baseline_allows_ctrl_alt_like_menu_accelerators() {
        let body = rust_function_body(include_str!("window.rs"), "handle_keyboard_shortcut");

        assert_body_contains_all(
            body,
            &[
                "VK_S_KEY if is_ctrl_down => Some(save_current_document_from_window(hwnd)),",
                "VK_N_KEY if is_ctrl_down => Some(create_sibling_document_from_selection(hwnd)),",
                "VK_W_KEY if is_ctrl_down => Some(close_active_tab_from_window(hwnd)),",
                "VK_F_KEY if is_ctrl_down => Some(open_find_dialog_from_window(hwnd)),",
                "VK_H_KEY if is_ctrl_down => Some(open_replace_dialog_from_window(hwnd)),",
            ],
        );
    }

    #[test]
    fn win32_focus_scoped_shortcut_baseline_keeps_alt_and_origin_rules() {
        let body = rust_function_body(include_str!("window.rs"), "handle_keyboard_shortcut");

        assert_body_contains_all(
            body,
            &[
                "VK_A_KEY if is_ctrl_down && !is_alt_down && shortcut_origin_is_editor(hwnd, message.hwnd) =>",
                "Some(select_all_editor_text(hwnd))",
                "VK_RETURN_KEY if is_ctrl_down && !is_alt_down && shortcut_origin_is_tree(hwnd, message.hwnd) =>",
                "Some(create_child_document_from_selection(hwnd))",
                "VK_RETURN_KEY if !is_ctrl_down && !is_alt_down && shortcut_origin_is_tree(hwnd, message.hwnd) =>",
                "Some(create_sibling_document_from_selection(hwnd))",
                "VK_F2_KEY if shortcut_origin_is_tree(hwnd, message.hwnd) =>",
                "Some(rename_selected_node(hwnd))",
                "VK_DELETE_KEY if shortcut_origin_is_tree(hwnd, message.hwnd) =>",
                "Some(delete_selected_node_from_keyboard(hwnd))",
                "VK_UP_KEY if is_ctrl_down && shortcut_origin_is_tree(hwnd, message.hwnd) =>",
                "move_selected_node_within_parent(hwnd, SiblingMoveDirection::Up)",
                "VK_DOWN_KEY if is_ctrl_down && shortcut_origin_is_tree(hwnd, message.hwnd) =>",
                "move_selected_node_within_parent(hwnd, SiblingMoveDirection::Down)",
            ],
        );
    }

    #[test]
    fn win32_tree_view_baseline_uses_buttons_lines_edit_and_persistent_selection() {
        let body = rust_function_body(include_str!("window.rs"), "create_tree_view");

        assert_body_contains_all(
            body,
            &[
                "WS_TABSTOP",
                "TVS_HASBUTTONS",
                "TVS_HASLINES",
                "TVS_LINESATROOT",
                "TVS_SHOWSELALWAYS",
                "TVS_EDITLABELS",
                "WC_TREEVIEWW",
            ],
        );
    }
}
