use std::mem;
use std::ptr;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, FindTextW, ReplaceTextW, FINDMSGSTRINGW, FINDREPLACEW, FR_DIALOGTERM,
    FR_DOWN, FR_FINDNEXT, FR_HIDEMATCHCASE, FR_HIDEUPDOWN, FR_HIDEWHOLEWORD, FR_REPLACE,
    FR_REPLACEALL,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, MessageBoxW, RegisterWindowMessageW, SendMessageW, MB_ICONINFORMATION, MB_OK,
};

use super::super::common::last_win32_error;
use super::super::i18n::ui_text;
use super::super::state::{FindReplaceDialogKind, ReplaceDialogState, WindowState};
use super::super::text::{
    byte_range_from_editor_offsets, editor_range_for_byte_range,
    editor_range_for_byte_range_from_anchor, editor_selection_utf16, select_editor_text_utf16,
    utf8_to_wide_null, wide_null_to_string, EditorOffsetAnchor, ProgrammaticTextUpdateGuard,
};
use super::super::window::window_state;
use super::{refresh_tabs_for_state, update_window_title};
use crate::domain::{
    find_next_literal, replace_all_literal, ReplaceAllError, TextMatch, UiLanguage,
};
use crate::error::AppError;
use crate::infra::text_file::TEXT_FILE_BYTE_LIMIT;

const REPLACE_DIALOG_TEXT_CAPACITY: usize = 1024;
const WM_GETTEXTLENGTH_EDIT_CONTROL: u32 = 0x000E;
const WM_USER: u32 = 0x0400;
const EM_REPLACESEL_EDIT_CONTROL: u32 = 0x00C2;
const EM_GETTEXTRANGE_RICH_EDIT_CONTROL: u32 = WM_USER + 75;
// Replace All constructs the full edited text before applying it, so keep the
// result under the same user-facing byte budget as text file import/export.
const REPLACE_ALL_OUTPUT_BYTE_LIMIT: usize = TEXT_FILE_BYTE_LIMIT;
static FIND_REPLACE_MESSAGE_ID: OnceLock<u32> = OnceLock::new();

#[repr(C)]
struct Win32CharRange {
    cp_min: i32,
    cp_max: i32,
}

#[repr(C)]
struct Win32TextRangeW {
    chrg: Win32CharRange,
    lpstr_text: *mut u16,
}

pub(in crate::platform::win32) unsafe fn open_find_dialog_from_window(
    hwnd: HWND,
) -> Result<(), AppError> {
    open_find_replace_dialog_from_window(hwnd, FindReplaceDialogKind::Find)
}

pub(in crate::platform::win32) unsafe fn open_replace_dialog_from_window(
    hwnd: HWND,
) -> Result<(), AppError> {
    open_find_replace_dialog_from_window(hwnd, FindReplaceDialogKind::Replace)
}

unsafe fn open_find_replace_dialog_from_window(
    hwnd: HWND,
    kind: FindReplaceDialogKind,
) -> Result<(), AppError> {
    let action = find_replace_dialog_action(kind);
    let previous_dialog_action = {
        let mut state = window_state(hwnd)
            .ok_or_else(|| AppError::platform(action, "window state was not attached"))?;
        ensure_active_replace_target(&state)?;
        state.store_editor_content_in_active_tab()?;

        existing_replace_dialog_action(&mut state.replace_dialog, kind)
    };

    if let Some(previous_dialog_action) = previous_dialog_action {
        match previous_dialog_action {
            ExistingReplaceDialogAction::Focus(dialog_hwnd) => {
                SetFocus(dialog_hwnd);
                return Ok(());
            }
            ExistingReplaceDialogAction::Destroy {
                hwnd: dialog_hwnd,
                request,
            } => {
                destroy_previous_find_replace_dialog(hwnd, dialog_hwnd, request)?;
            }
        }
    }

    if find_replace_message_id() == 0 {
        return Err(last_win32_error("register find/replace message"));
    }

    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform(action, "window state was not attached"))?;
    ensure_active_replace_target(&state)?;
    let mut dialog = create_find_replace_dialog_state(hwnd, kind)?;
    let dialog_hwnd = match kind {
        FindReplaceDialogKind::Find => FindTextW(dialog.request.as_mut()),
        FindReplaceDialogKind::Replace => ReplaceTextW(dialog.request.as_mut()),
    };
    if dialog_hwnd.is_null() {
        let code = CommDlgExtendedError();
        return Err(AppError::platform(
            action,
            format!("common dialog error code {code}"),
        ));
    }

    dialog.hwnd = dialog_hwnd;
    state.replace_dialog = Some(dialog);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingReplaceDialogAction {
    Focus(HWND),
    Destroy {
        hwnd: HWND,
        request: *const FINDREPLACEW,
    },
}

fn existing_replace_dialog_action(
    replace_dialog: &mut Option<ReplaceDialogState>,
    kind: FindReplaceDialogKind,
) -> Option<ExistingReplaceDialogAction> {
    let (dialog_hwnd, dialog_kind, request) = match replace_dialog.as_ref() {
        Some(dialog) => (
            dialog.hwnd,
            dialog.kind,
            dialog.request.as_ref() as *const FINDREPLACEW,
        ),
        None => return None,
    };

    if dialog_hwnd.is_null() {
        *replace_dialog = None;
        return None;
    }

    if dialog_kind == kind {
        return Some(ExistingReplaceDialogAction::Focus(dialog_hwnd));
    }

    Some(ExistingReplaceDialogAction::Destroy {
        hwnd: dialog_hwnd,
        request,
    })
}

unsafe fn destroy_previous_find_replace_dialog(
    owner_hwnd: HWND,
    dialog_hwnd: HWND,
    request: *const FINDREPLACEW,
) -> Result<(), AppError> {
    if DestroyWindow(dialog_hwnd) == 0 {
        return Err(last_win32_error("destroy previous find/replace dialog"));
    }

    if let Some(mut state) = window_state(owner_hwnd) {
        clear_destroyed_replace_dialog(&mut state.replace_dialog, dialog_hwnd, request);
    }

    Ok(())
}

fn clear_destroyed_replace_dialog(
    replace_dialog: &mut Option<ReplaceDialogState>,
    dialog_hwnd: HWND,
    request: *const FINDREPLACEW,
) {
    let should_clear = replace_dialog.as_ref().is_some_and(|dialog| {
        let active_request = dialog.request.as_ref() as *const FINDREPLACEW;
        dialog.hwnd == dialog_hwnd && ptr::eq(active_request, request)
    });

    if should_clear {
        *replace_dialog = None;
    }
}

pub(in crate::platform::win32) unsafe fn active_replace_dialog(hwnd: HWND) -> Option<HWND> {
    window_state(hwnd)?
        .replace_dialog
        .as_ref()
        .and_then(|dialog| (!dialog.hwnd.is_null()).then_some(dialog.hwnd))
}

pub(in crate::platform::win32) fn find_replace_message_id() -> u32 {
    *FIND_REPLACE_MESSAGE_ID.get_or_init(|| unsafe { RegisterWindowMessageW(FINDMSGSTRINGW) })
}

pub(in crate::platform::win32) unsafe fn handle_find_replace_message(
    hwnd: HWND,
    lparam: isize,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle replace dialog", "window state was not attached")
    })?;
    if lparam == 0 {
        return Ok(());
    }

    let request = lparam as *const FINDREPLACEW;
    if request.is_null() {
        return Ok(());
    }

    let Some(dialog) = state.replace_dialog.as_ref() else {
        return Ok(());
    };
    if dialog.hwnd.is_null() {
        return Ok(());
    }

    let expected_request = dialog.request.as_ref() as *const FINDREPLACEW;
    let expected_find_text = dialog._find_buffer.as_ptr();
    let expected_replace_text = dialog._replace_buffer.as_ptr();
    if request != expected_request {
        return Ok(());
    }

    // SAFETY: The registered find/replace message is only trusted when lparam equals the
    // FINDREPLACEW allocation owned by the active dialog state above.
    let flags = (*request).Flags;
    if flags & FR_DIALOGTERM != 0 {
        state.clear_current_find_match_highlight()?;
        state.replace_dialog = None;
        return Ok(());
    }

    ensure_active_replace_target(&state)?;
    let find_text = (*request).lpstrFindWhat;
    let replace_text = (*request).lpstrReplaceWith;
    if find_text.is_null()
        || replace_text.is_null()
        || !ptr::eq(find_text, expected_find_text)
        || !ptr::eq(replace_text, expected_replace_text)
    {
        return Err(AppError::platform(
            "handle replace dialog",
            "replace dialog text buffer was not available",
        ));
    }

    let needle = wide_null_to_string(find_text);
    if needle.is_empty() {
        return Err(AppError::user(
            ui_text(state.app.ui_settings().language).missing_find_text(),
        ));
    }
    let replacement = wide_null_to_string(replace_text);

    if flags & FR_REPLACEALL != 0 {
        replace_all_in_active_editor(hwnd, &mut state, &needle, &replacement)
    } else if flags & FR_REPLACE != 0 {
        replace_one_in_active_editor(hwnd, &mut state, &needle, &replacement)
    } else if flags & FR_FINDNEXT != 0 {
        find_next_in_active_editor(&mut state, &needle)
    } else {
        Ok(())
    }
}

fn create_find_replace_dialog_state(
    owner: HWND,
    kind: FindReplaceDialogKind,
) -> Result<ReplaceDialogState, AppError> {
    let mut find_buffer = vec![0u16; REPLACE_DIALOG_TEXT_CAPACITY];
    let mut replace_buffer = vec![0u16; REPLACE_DIALOG_TEXT_CAPACITY];
    let find_len = u16::try_from(find_buffer.len()).map_err(|_| {
        AppError::platform(
            find_replace_dialog_action(kind),
            "find text buffer is too large",
        )
    })?;
    let replace_len = u16::try_from(replace_buffer.len()).map_err(|_| {
        AppError::platform(
            find_replace_dialog_action(kind),
            "replace text buffer is too large",
        )
    })?;

    let request = Box::new(FINDREPLACEW {
        lStructSize: mem::size_of::<FINDREPLACEW>() as u32,
        hwndOwner: owner,
        hInstance: ptr::null_mut(),
        Flags: FR_DOWN | FR_HIDEMATCHCASE | FR_HIDEWHOLEWORD | FR_HIDEUPDOWN,
        lpstrFindWhat: find_buffer.as_mut_ptr(),
        lpstrReplaceWith: replace_buffer.as_mut_ptr(),
        wFindWhatLen: find_len,
        wReplaceWithLen: replace_len,
        lCustData: 0,
        lpfnHook: None,
        lpTemplateName: ptr::null(),
    });

    Ok(ReplaceDialogState {
        kind,
        hwnd: ptr::null_mut(),
        _find_buffer: find_buffer,
        _replace_buffer: replace_buffer,
        request,
    })
}

fn find_replace_dialog_action(kind: FindReplaceDialogKind) -> &'static str {
    match kind {
        FindReplaceDialogKind::Find => "open find dialog",
        FindReplaceDialogKind::Replace => "open replace dialog",
    }
}

fn ensure_active_replace_target(state: &WindowState) -> Result<(), AppError> {
    let text = ui_text(state.app.ui_settings().language);
    let Some(tab) = state.tabs.active() else {
        return Err(AppError::user(text.open_editable_document()));
    };

    if !tab.editable {
        return Err(AppError::user(text.read_only_find_replace()));
    }

    Ok(())
}

unsafe fn stored_active_editor_content<'a>(
    state: &'a mut WindowState,
    context: &'static str,
) -> Result<&'a str, AppError> {
    state
        .store_editor_content_in_active_tab_and_get()?
        .ok_or_else(|| AppError::platform(context, "active tab was not available"))
}

unsafe fn find_next_in_active_editor(
    state: &mut WindowState,
    needle: &str,
) -> Result<(), AppError> {
    let editor = state.editor;
    let language = state.app.ui_settings().language;
    let editor_range = {
        let content = stored_active_editor_content(state, "find text")?;
        let selection = current_editor_selection_range(editor, content);
        let start = selection.map(|selection| selection.byte_end).unwrap_or(0);
        let anchor = selection
            .map(EditorSelectionRange::end_anchor)
            .unwrap_or_else(|| EditorOffsetAnchor {
                byte_index: 0,
                editor_offset_utf16: 0,
            });
        find_next_match_editor_range(content, needle, start, anchor)?
    };

    match editor_range {
        Some((start, end)) => state.show_current_find_match_in_editor(start, end),
        None => Err(AppError::user(ui_text(language).no_match())),
    }
}

unsafe fn replace_one_in_active_editor(
    hwnd: HWND,
    state: &mut WindowState,
    needle: &str,
    replacement: &str,
) -> Result<(), AppError> {
    let editor = state.editor;
    let language = state.app.ui_settings().language;
    let target = {
        let content = stored_active_editor_content(state, "replace text")?;
        let selection = current_editor_selection_range(editor, content);
        let selected_match = selection.filter(|selection| {
            selection.byte_end > selection.byte_start
                && content.get(selection.byte_start..selection.byte_end) == Some(needle)
        });
        match selected_match {
            Some(target) => target,
            None => {
                let start = selection.map(|selection| selection.byte_end).unwrap_or(0);
                let anchor = selection
                    .map(EditorSelectionRange::end_anchor)
                    .unwrap_or_else(|| EditorOffsetAnchor {
                        byte_index: 0,
                        editor_offset_utf16: 0,
                    });
                let editor_range = find_next_match_editor_range(content, needle, start, anchor)?;
                return match editor_range {
                    Some((start, end)) => state.show_current_find_match_in_editor(start, end),
                    None => Err(AppError::user(ui_text(language).no_match())),
                };
            }
        }
    };

    let next_start = checked_replacement_end(target.byte_start, replacement)?;
    if replacement != needle {
        let replacement_wide = utf8_to_wide_null("convert replacement text", replacement)?;
        prepare_active_tab_selected_match_replacement(
            state,
            needle,
            replacement,
            target.byte_start,
        )?;
        validate_editor_replace_range(editor, target.editor_start_utf16, target.editor_end_utf16)?;
        let editor_text_len_before_replace = editor_text_len_utf16(editor)?;
        replace_editor_text_range_utf16(
            editor,
            &state.suppress_editor_change,
            target.editor_start_utf16,
            target.editor_end_utf16,
            &replacement_wide,
        )?;
        if let Err(error) = verify_editor_replaced_range(
            state,
            editor_text_len_before_replace,
            target.editor_start_utf16,
            target.editor_end_utf16,
            &replacement_wide,
        ) {
            state.show_active_tab_in_editor()?;
            return Err(error);
        }
        apply_active_tab_selected_match_replacement(state, needle, replacement, target.byte_start)?;
        state.editor_content_pending_sync = false;
        refresh_tabs_for_state(state)?;
        update_window_title(hwnd, state)?;
    }

    let next_selection = {
        let content = state
            .tabs
            .active()
            .ok_or_else(|| AppError::platform("replace text", "active tab was not available"))?
            .content
            .as_str();
        let next_start_anchor = editor_range_for_byte_range_from_anchor(
            content,
            target.start_anchor(),
            next_start..next_start,
        )
        .ok()
        .map(|(start, _)| EditorOffsetAnchor {
            byte_index: next_start,
            editor_offset_utf16: start,
        });
        replace_one_follow_up_selection(content, needle, next_start, next_start_anchor)?
    };

    match next_selection {
        ReplaceOneEditorSelection::CurrentFindMatch { start, end } => {
            state.show_current_find_match_in_editor(start, end)
        }
        ReplaceOneEditorSelection::Caret { start, end } => {
            select_editor_text_utf16(editor, start, end)
        }
    }
}

unsafe fn replace_all_in_active_editor(
    hwnd: HWND,
    state: &mut WindowState,
    needle: &str,
    replacement: &str,
) -> Result<(), AppError> {
    let (count, next_content) = {
        let language = state.app.ui_settings().language;
        let content = stored_active_editor_content(state, "replace all text")?;
        let result =
            replace_all_literal(content, needle, replacement, REPLACE_ALL_OUTPUT_BYTE_LIMIT)
                .map_err(|error| replace_all_error_to_app_error(error, language))?;
        let next_content = (result.count > 0 && result.content.as_ref() != content)
            .then(|| result.content.into_owned());
        (result.count, next_content)
    };

    if let Some(content) = next_content {
        apply_active_editor_content(hwnd, state, content)?;
    }

    show_replace_all_count(hwnd, count, state.app.ui_settings().language)
}

fn replace_all_error_to_app_error(error: ReplaceAllError, language: UiLanguage) -> AppError {
    let text = ui_text(language);
    match error {
        ReplaceAllError::OutputTooLarge { limit } => {
            let limit_mib = limit / 1024 / 1024;
            AppError::user(text.replace_all_too_large(limit_mib))
        }
        ReplaceAllError::OutputSizeOverflow => AppError::user(text.replace_all_overflow()),
        ReplaceAllError::OutputAllocationFailed { requested } => {
            AppError::user(text.replace_all_allocation_failed(requested))
        }
    }
}

unsafe fn apply_active_editor_content(
    hwnd: HWND,
    state: &mut WindowState,
    content: String,
) -> Result<(), AppError> {
    state.tabs.update_active_content(content);
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(state)?;
    update_window_title(hwnd, state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EditorSelectionRange {
    byte_start: usize,
    byte_end: usize,
    editor_start_utf16: usize,
    editor_end_utf16: usize,
}

impl EditorSelectionRange {
    fn start_anchor(self) -> EditorOffsetAnchor {
        EditorOffsetAnchor {
            byte_index: self.byte_start,
            editor_offset_utf16: self.editor_start_utf16,
        }
    }

    fn end_anchor(self) -> EditorOffsetAnchor {
        EditorOffsetAnchor {
            byte_index: self.byte_end,
            editor_offset_utf16: self.editor_end_utf16,
        }
    }
}

unsafe fn current_editor_selection_range(
    editor: HWND,
    content: &str,
) -> Option<EditorSelectionRange> {
    let (editor_start_utf16, editor_end_utf16) = editor_selection_utf16(editor)?;
    let (byte_start, byte_end) =
        byte_range_from_editor_offsets(content, editor_start_utf16, editor_end_utf16);
    Some(EditorSelectionRange {
        byte_start,
        byte_end,
        editor_start_utf16,
        editor_end_utf16,
    })
}

fn checked_replacement_end(start: usize, replacement: &str) -> Result<usize, AppError> {
    start
        .checked_add(replacement.len())
        .ok_or_else(|| AppError::platform("replace text", "replacement position is too large"))
}

fn validate_editor_replace_range(
    editor: HWND,
    start_utf16: usize,
    end_utf16: usize,
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    i32::try_from(start_utf16)
        .map_err(|_| AppError::platform("replace editor text", "selection start is too large"))?;
    i32::try_from(end_utf16)
        .map_err(|_| AppError::platform("replace editor text", "selection end is too large"))?;
    Ok(())
}

fn prepare_active_tab_selected_match_replacement(
    state: &mut WindowState,
    needle: &str,
    replacement: &str,
    start: usize,
) -> Result<(), AppError> {
    let Some(tab) = state.tabs.active_mut() else {
        return Err(AppError::platform(
            "replace text",
            "active tab was not available",
        ));
    };
    if !tab.editable {
        return Err(AppError::platform(
            "replace text",
            "active tab was read-only",
        ));
    }

    prepare_selected_match_replacement(&mut tab.content, needle, replacement, start)?;
    Ok(())
}

fn apply_active_tab_selected_match_replacement(
    state: &mut WindowState,
    needle: &str,
    replacement: &str,
    start: usize,
) -> Result<(), AppError> {
    let Some(tab) = state.tabs.active_mut() else {
        return Err(AppError::platform(
            "replace text",
            "active tab was not available",
        ));
    };
    if !tab.editable {
        return Err(AppError::platform(
            "replace text",
            "active tab was read-only",
        ));
    }

    let mut content = mem::take(&mut tab.content);
    if let Err(error) =
        replace_selected_match_content_in_place(&mut content, needle, replacement, start)
    {
        tab.content = content;
        return Err(error);
    }
    tab.set_content(content);
    Ok(())
}

fn selected_match_range(
    content: &str,
    needle: &str,
    start: usize,
) -> Result<std::ops::Range<usize>, AppError> {
    let stale_selection = || AppError::platform("replace text", "selected text no longer matches");
    if needle.is_empty() || start > content.len() || !content.is_char_boundary(start) {
        return Err(stale_selection());
    }

    let end = start
        .checked_add(needle.len())
        .ok_or_else(stale_selection)?;
    if end > content.len()
        || !content.is_char_boundary(end)
        || content.get(start..end) != Some(needle)
    {
        return Err(stale_selection());
    }

    Ok(start..end)
}

fn prepare_selected_match_replacement(
    content: &mut String,
    needle: &str,
    replacement: &str,
    start: usize,
) -> Result<std::ops::Range<usize>, AppError> {
    let range = selected_match_range(content, needle, start)?;
    let next_len = content
        .len()
        .checked_sub(needle.len())
        .and_then(|len| len.checked_add(replacement.len()))
        .ok_or_else(|| AppError::platform("replace text", "replacement text is too large"))?;
    let additional_capacity = next_len.saturating_sub(content.len());
    if additional_capacity > 0 {
        content
            .try_reserve(additional_capacity)
            .map_err(|_| AppError::platform("replace text", "replacement text is too large"))?;
    }
    Ok(range)
}

fn replace_selected_match_content_in_place(
    content: &mut String,
    needle: &str,
    replacement: &str,
    start: usize,
) -> Result<(), AppError> {
    let range = prepare_selected_match_replacement(content, needle, replacement, start)?;
    content.replace_range(range, replacement);
    Ok(())
}

unsafe fn editor_text_len_utf16(editor: HWND) -> Result<Option<usize>, AppError> {
    if editor.is_null() {
        return Ok(None);
    }

    let len = SendMessageW(editor, WM_GETTEXTLENGTH_EDIT_CONTROL, 0, 0);
    usize::try_from(len)
        .map(Some)
        .map_err(|_| AppError::platform("replace editor text", "editor text length was invalid"))
}

unsafe fn verify_editor_replaced_range(
    state: &mut WindowState,
    before_len_utf16: Option<usize>,
    start_utf16: usize,
    end_utf16: usize,
    replacement_wide: &[u16],
) -> Result<(), AppError> {
    if state.editor.is_null() {
        return Ok(());
    }

    let replacement = replacement_utf16_units(replacement_wide)?;
    verify_editor_text_len_after_replace(
        state.editor,
        before_len_utf16,
        start_utf16,
        end_utf16,
        replacement.len(),
    )?;
    verify_editor_text_range_utf16(
        state.editor,
        &mut state.editor_text_utf16_buffer,
        start_utf16,
        replacement,
    )
}

fn replacement_utf16_units(replacement_wide: &[u16]) -> Result<&[u16], AppError> {
    let Some((&terminator, replacement)) = replacement_wide.split_last() else {
        return Err(AppError::platform(
            "replace editor text",
            "replacement text was not null terminated",
        ));
    };
    if terminator != 0 {
        return Err(AppError::platform(
            "replace editor text",
            "replacement text was not null terminated",
        ));
    }
    Ok(replacement)
}

unsafe fn verify_editor_text_len_after_replace(
    editor: HWND,
    before_len_utf16: Option<usize>,
    start_utf16: usize,
    end_utf16: usize,
    replacement_len_utf16: usize,
) -> Result<(), AppError> {
    let Some(before_len_utf16) = before_len_utf16 else {
        return Ok(());
    };
    let selected_len_utf16 = end_utf16
        .checked_sub(start_utf16)
        .ok_or_else(editor_replace_mismatch_error)?;
    let expected_len_utf16 = before_len_utf16
        .checked_sub(selected_len_utf16)
        .and_then(|len| len.checked_add(replacement_len_utf16))
        .ok_or_else(editor_replace_mismatch_error)?;
    if editor_text_len_utf16(editor)? == Some(expected_len_utf16) {
        Ok(())
    } else {
        Err(editor_replace_mismatch_error())
    }
}

unsafe fn verify_editor_text_range_utf16(
    editor: HWND,
    buffer: &mut Vec<u16>,
    start_utf16: usize,
    expected: &[u16],
) -> Result<(), AppError> {
    let (start_utf16, end_utf16) = editor_replacement_range_i32(start_utf16, expected.len())?;
    let buffer_len = expected.len().checked_add(1).ok_or_else(|| {
        AppError::platform("replace editor text", "replacement text is too large")
    })?;
    buffer.clear();
    buffer
        .try_reserve(buffer_len)
        .map_err(|_| AppError::platform("replace editor text", "replacement text is too large"))?;
    buffer.resize(buffer_len, 0);

    let mut text_range = Win32TextRangeW {
        chrg: Win32CharRange {
            cp_min: start_utf16,
            cp_max: end_utf16,
        },
        lpstr_text: buffer.as_mut_ptr(),
    };
    let copied = SendMessageW(
        editor,
        EM_GETTEXTRANGE_RICH_EDIT_CONTROL,
        0,
        (&mut text_range as *mut Win32TextRangeW) as LPARAM,
    );
    if copied == expected.len() as isize && &buffer[..expected.len()] == expected {
        Ok(())
    } else {
        Err(editor_replace_mismatch_error())
    }
}

fn editor_replacement_range_i32(
    start_utf16: usize,
    replacement_len_utf16: usize,
) -> Result<(i32, i32), AppError> {
    let end_utf16 = start_utf16
        .checked_add(replacement_len_utf16)
        .ok_or_else(|| {
            AppError::platform("replace editor text", "replacement range is too large")
        })?;
    let start_utf16 = i32::try_from(start_utf16)
        .map_err(|_| AppError::platform("replace editor text", "selection start is too large"))?;
    let end_utf16 = i32::try_from(end_utf16)
        .map_err(|_| AppError::platform("replace editor text", "selection end is too large"))?;
    Ok((start_utf16, end_utf16))
}

fn editor_replace_mismatch_error() -> AppError {
    AppError::platform(
        "replace editor text",
        "editor content did not match the requested replacement",
    )
}

unsafe fn replace_editor_text_range_utf16(
    editor: HWND,
    suppress_editor_change: &std::cell::Cell<bool>,
    start_utf16: usize,
    end_utf16: usize,
    replacement: &[u16],
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    let _guard = ProgrammaticTextUpdateGuard::enter(suppress_editor_change);
    select_editor_text_utf16(editor, start_utf16, end_utf16)?;
    let _ = SendMessageW(
        editor,
        EM_REPLACESEL_EDIT_CONTROL,
        0,
        replacement.as_ptr() as LPARAM,
    );
    Ok(())
}

fn find_next_wrapping(content: &str, needle: &str, start: usize) -> Option<TextMatch> {
    find_next_literal(content, needle, start).or_else(|| {
        (start > 0)
            .then(|| find_next_literal(content, needle, 0))
            .flatten()
    })
}

fn find_next_match_editor_range(
    content: &str,
    needle: &str,
    start: usize,
    anchor: EditorOffsetAnchor,
) -> Result<Option<(usize, usize)>, AppError> {
    find_next_wrapping(content, needle, start)
        .map(|found| {
            editor_range_for_byte_range_from_anchor(content, anchor, found.start..found.end)
        })
        .transpose()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplaceOneEditorSelection {
    CurrentFindMatch { start: usize, end: usize },
    Caret { start: usize, end: usize },
}

fn replace_one_follow_up_selection(
    content: &str,
    needle: &str,
    next_start: usize,
    next_start_anchor: Option<EditorOffsetAnchor>,
) -> Result<ReplaceOneEditorSelection, AppError> {
    if let Some(found) = find_next_wrapping(content, needle, next_start) {
        let (start, end) = match next_start_anchor {
            Some(anchor) => {
                editor_range_for_byte_range_from_anchor(content, anchor, found.start..found.end)?
            }
            None => editor_range_for_byte_range(content, found.start..found.end)?,
        };
        return Ok(ReplaceOneEditorSelection::CurrentFindMatch { start, end });
    }

    let (start, end) = match next_start_anchor {
        Some(anchor) => (anchor.editor_offset_utf16, anchor.editor_offset_utf16),
        None => editor_range_for_byte_range(content, next_start..next_start)?,
    };
    Ok(ReplaceOneEditorSelection::Caret { start, end })
}

unsafe fn show_replace_all_count(
    hwnd: HWND,
    count: usize,
    language: UiLanguage,
) -> Result<(), AppError> {
    let message = ui_text(language).replace_all_count(count);
    let title = utf8_to_wide_null("convert replace result title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert replace result message", &message)?;
    MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::replace_literal_at;

    #[test]
    fn existing_replace_dialog_action_keeps_destroy_target_state_owned() {
        let mut dialog_token = ();
        let dialog_hwnd = (&mut dialog_token as *mut ()).cast();
        let mut replace_dialog = Some(test_replace_dialog_state(
            FindReplaceDialogKind::Find,
            dialog_hwnd,
        ));
        let request = replace_dialog
            .as_ref()
            .map(|dialog| dialog.request.as_ref() as *const FINDREPLACEW)
            .expect("dialog state should exist");

        assert_eq!(
            existing_replace_dialog_action(&mut replace_dialog, FindReplaceDialogKind::Replace),
            Some(ExistingReplaceDialogAction::Destroy {
                hwnd: dialog_hwnd,
                request
            })
        );
        assert!(replace_dialog.is_some());
    }

    #[test]
    fn clear_destroyed_replace_dialog_requires_matching_request() {
        let mut dialog_token = ();
        let dialog_hwnd = (&mut dialog_token as *mut ()).cast();
        let mut replace_dialog = Some(test_replace_dialog_state(
            FindReplaceDialogKind::Find,
            dialog_hwnd,
        ));
        let other_dialog = test_replace_dialog_state(FindReplaceDialogKind::Find, dialog_hwnd);
        let other_request = other_dialog.request.as_ref() as *const FINDREPLACEW;

        clear_destroyed_replace_dialog(&mut replace_dialog, dialog_hwnd, other_request);
        assert!(replace_dialog.is_some());

        let request = replace_dialog
            .as_ref()
            .map(|dialog| dialog.request.as_ref() as *const FINDREPLACEW)
            .expect("dialog state should still exist");
        clear_destroyed_replace_dialog(&mut replace_dialog, dialog_hwnd, request);
        assert!(replace_dialog.is_none());
    }

    #[test]
    fn find_next_wrapping_handles_crlf_emoji_and_empty_content() {
        let content = "ASCII\r\n🚀 target\r\n한글 target";
        let second_start = content.find("한글").expect("second line should exist");

        assert_eq!(
            find_next_wrapping(content, "target", second_start),
            Some(TextMatch { start: 27, end: 33 })
        );
        assert_eq!(
            find_next_wrapping(content, "target", 33),
            Some(TextMatch { start: 12, end: 18 })
        );
        assert_eq!(find_next_wrapping("", "target", 0), None);
        assert_eq!(find_next_wrapping(content, "", 0), None);
    }

    #[test]
    fn find_next_match_editor_range_converts_wrapped_match_for_rich_edit() {
        let content = "ASCII\r\n🚀 target\r\n한글 target";
        let second_start = content.find("한글").expect("second line should exist");

        assert_eq!(
            find_next_match_editor_range(
                content,
                "target",
                second_start,
                EditorOffsetAnchor {
                    byte_index: second_start,
                    editor_offset_utf16: 16,
                },
            )
            .expect("match range should convert"),
            Some((19, 25))
        );
        assert_eq!(
            find_next_match_editor_range(
                content,
                "target",
                content.len(),
                EditorOffsetAnchor {
                    byte_index: content.len(),
                    editor_offset_utf16: 25,
                },
            )
            .expect("wrapped match range should convert"),
            Some((9, 15))
        );
        assert_eq!(
            find_next_match_editor_range(
                content,
                "missing",
                0,
                EditorOffsetAnchor {
                    byte_index: 0,
                    editor_offset_utf16: 0,
                },
            )
            .expect("missing range should convert"),
            None
        );
    }

    #[test]
    fn replace_one_follow_up_selection_converts_next_match_before_apply() {
        let content = "ASCII\r\n🚀 one\r\n한글 one";
        let first_start = content.find("one").expect("first match should exist");
        let replaced =
            replace_literal_at(content, "one", "two", first_start).expect("match should replace");

        assert_eq!(
            replace_one_follow_up_selection(
                &replaced.content,
                "one",
                first_start + "two".len(),
                Some(EditorOffsetAnchor {
                    byte_index: first_start + "two".len(),
                    editor_offset_utf16: 12,
                }),
            )
            .expect("follow-up match range should convert"),
            ReplaceOneEditorSelection::CurrentFindMatch { start: 16, end: 19 }
        );
    }

    #[test]
    fn replace_one_follow_up_selection_restores_caret_when_no_next_match() {
        let content = "ASCII\r\n🚀 one\r\n한글 one";
        let first_start = content.find("one").expect("first match should exist");
        let replaced =
            replace_literal_at(content, "one", "two", first_start).expect("match should replace");

        assert_eq!(
            replace_one_follow_up_selection(
                &replaced.content,
                "missing",
                first_start + "two".len(),
                Some(EditorOffsetAnchor {
                    byte_index: first_start + "two".len(),
                    editor_offset_utf16: 12,
                }),
            )
            .expect("caret range should convert"),
            ReplaceOneEditorSelection::Caret { start: 12, end: 12 }
        );
    }

    #[test]
    fn prepare_selected_match_replacement_reserves_without_mutating_content() {
        let mut content = String::with_capacity("ASCII\r\n🚀 one\r\n한글 one".len());
        content.push_str("ASCII\r\n🚀 one\r\n한글 one");
        let first_start = content.find("one").expect("first match should exist");
        let original = content.clone();

        prepare_selected_match_replacement(&mut content, "one", "three", first_start)
            .expect("selected match should prepare");

        assert_eq!(content, original);
        assert!(content.capacity() >= original.len() - "one".len() + "three".len());
    }

    #[test]
    fn replace_selected_match_content_in_place_updates_model_buffer() {
        let mut content = String::with_capacity(64);
        content.push_str("ASCII\r\n🚀 one\r\n한글 one");
        let first_start = content.find("one").expect("first match should exist");
        let original_capacity = content.capacity();

        replace_selected_match_content_in_place(&mut content, "one", "two", first_start)
            .expect("selected match should replace");

        assert_eq!(content, "ASCII\r\n🚀 two\r\n한글 one");
        assert_eq!(content.capacity(), original_capacity);
    }

    #[test]
    fn replace_selected_match_content_in_place_rejects_stale_selection() {
        let mut content = String::from("ASCII one");
        let start = content.find("one").expect("match should exist");

        assert!(
            replace_selected_match_content_in_place(&mut content, "missing", "two", start).is_err()
        );
        assert_eq!(content, "ASCII one");
    }

    fn test_replace_dialog_state(kind: FindReplaceDialogKind, hwnd: HWND) -> ReplaceDialogState {
        let mut find_buffer = vec![0u16; 2];
        let mut replace_buffer = vec![0u16; 2];
        let request = Box::new(FINDREPLACEW {
            lStructSize: mem::size_of::<FINDREPLACEW>() as u32,
            hwndOwner: ptr::null_mut(),
            hInstance: ptr::null_mut(),
            Flags: 0,
            lpstrFindWhat: find_buffer.as_mut_ptr(),
            lpstrReplaceWith: replace_buffer.as_mut_ptr(),
            wFindWhatLen: 2,
            wReplaceWithLen: 2,
            lCustData: 0,
            lpfnHook: None,
            lpTemplateName: ptr::null(),
        });

        ReplaceDialogState {
            kind,
            hwnd,
            _find_buffer: find_buffer,
            _replace_buffer: replace_buffer,
            request,
        }
    }
}
