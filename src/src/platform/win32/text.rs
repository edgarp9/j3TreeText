use std::borrow::Cow;
use std::cell::Cell;
use std::ffi::c_char;
use std::iter;
use std::mem;
use std::ops::Range;
use std::ptr;

use windows_sys::Win32::Foundation::{GetLastError, SetLastError, COLORREF, HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::Controls::{
    EM_GETFIRSTVISIBLELINE, EM_GETLINECOUNT, EM_LINESCROLL, EM_SCROLLCARET, EM_SETREADONLY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowTextLengthW, GetWindowTextW, SendMessageW, SetWindowTextW,
};

use super::common::{
    last_win32_error, win32_error, CFE_AUTOBACKCOLOR_RICH_EDIT, CFM_BACKCOLOR_RICH_EDIT,
    DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB, DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS, EMPTY_TEXT,
    EM_EXGETSEL_RICH_EDIT, EM_EXSETSEL_RICH_EDIT, EM_GETTEXTEX_RICH_EDIT,
    EM_GETTEXTLENGTHEX_RICH_EDIT, EM_LINEFROMCHAR_EDIT_CONTROL, EM_LINEINDEX_EDIT_CONTROL,
    EM_SETCHARFORMAT_RICH_EDIT, GTL_NUMCHARS_RICH_EDIT, GTL_USECRLF_RICH_EDIT,
    GT_USECRLF_RICH_EDIT, RICH_EDIT_CP_UNICODE, SCF_SELECTION_RICH_EDIT,
};
use crate::domain::DocumentTabViewState;
use crate::error::{AppError, PlatformUserMessage, TextFileTooLargeUserMessage};

pub(super) struct ProgrammaticTextUpdateGuard<'a> {
    suppress_change: &'a Cell<bool>,
    previous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentEditorTextPrepareError {
    EmbeddedNul,
    TextLimitExceeded,
    Win32ControlLimitExceeded,
}

impl<'a> ProgrammaticTextUpdateGuard<'a> {
    pub(super) fn enter(suppress_change: &'a Cell<bool>) -> Self {
        let previous = suppress_change.replace(true);
        Self {
            suppress_change,
            previous,
        }
    }
}

impl Drop for ProgrammaticTextUpdateGuard<'_> {
    fn drop(&mut self) {
        self.suppress_change.set(self.previous);
    }
}

#[repr(C)]
struct RichEditGetTextLengthEx {
    flags: u32,
    codepage: u32,
}

#[repr(C)]
struct RichEditGetTextEx {
    cb: u32,
    flags: u32,
    codepage: u32,
    lp_default_char: *const c_char,
    lp_used_def_char: *mut i32,
}

#[repr(C)]
struct RichEditCharRange {
    cp_min: i32,
    cp_max: i32,
}

#[repr(C)]
struct RichEditCharFormat2W {
    cb_size: u32,
    mask: u32,
    effects: u32,
    height: i32,
    offset: i32,
    text_color: COLORREF,
    char_set: u8,
    pitch_and_family: u8,
    face_name: [u16; RICH_EDIT_FACE_NAME_UNITS],
    weight: u16,
    spacing: i16,
    back_color: COLORREF,
    lcid: u32,
    reserved: u32,
    style: i16,
    kerning: u16,
    underline_type: u8,
    animation: u8,
    revision_author: u8,
    underline_color: u8,
}

impl RichEditCharFormat2W {
    fn background_color(color: COLORREF) -> Self {
        Self {
            cb_size: mem::size_of::<Self>() as u32,
            mask: CFM_BACKCOLOR_RICH_EDIT,
            effects: 0,
            height: 0,
            offset: 0,
            text_color: 0,
            char_set: 0,
            pitch_and_family: 0,
            face_name: [0; RICH_EDIT_FACE_NAME_UNITS],
            weight: 0,
            spacing: 0,
            back_color: color,
            lcid: 0,
            reserved: 0,
            style: 0,
            kerning: 0,
            underline_type: 0,
            animation: 0,
            revision_author: 0,
            underline_color: 0,
        }
    }

    fn auto_background() -> Self {
        let mut format = Self::background_color(0);
        format.effects = CFE_AUTOBACKCOLOR_RICH_EDIT;
        format
    }
}

const RICH_EDIT_FACE_NAME_UNITS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CaretLineColumn {
    pub(super) line: usize,
    pub(super) column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RichEditHighlightColors {
    pub(super) background: COLORREF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RichEditFindMatchDisplay {
    SelectionOnly,
    SelectionWithTemporaryFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EditorOffsetAnchor {
    pub(super) byte_index: usize,
    pub(super) editor_offset_utf16: usize,
}

impl Default for CaretLineColumn {
    fn default() -> Self {
        Self { line: 1, column: 1 }
    }
}

#[cfg(test)]
pub(super) unsafe fn set_editor_for_document(
    editor: HWND,
    content: Option<&str>,
    read_only: bool,
) -> Result<(), AppError> {
    let mut text_buffer = Vec::new();
    set_editor_for_document_reusing(editor, content, read_only, &mut text_buffer)
}

#[cfg(test)]
unsafe fn set_editor_for_document_reusing(
    editor: HWND,
    content: Option<&str>,
    read_only: bool,
    text_buffer: &mut Vec<u16>,
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    set_editor_read_only(editor, read_only);
    let text = match content {
        Some(content) => {
            document_editor_plain_text_to_wide_null_reusing(
                "convert editor text from UTF-8 to UTF-16",
                content,
                text_buffer,
            )?;
            text_buffer.as_slice()
        }
        None => &EMPTY_TEXT,
    };
    set_editor_text(editor, text)
}

pub(super) unsafe fn set_editor_for_normalized_document_reusing(
    editor: HWND,
    content: Option<&mut String>,
    read_only: bool,
    text_buffer: &mut Vec<u16>,
) -> Result<usize, AppError> {
    if editor.is_null() {
        if let Some(content) = content {
            normalize_editor_plain_text_in_place(content);
        }
        return Ok(0);
    }

    set_editor_read_only(editor, read_only);
    let (text, selection_offset_count) = match content {
        Some(content) => {
            let selection_offset_count =
                document_editor_plain_text_to_wide_null_normalizing_reusing(
                    "convert editor text from UTF-8 to UTF-16",
                    content,
                    text_buffer,
                )?;
            (text_buffer.as_slice(), selection_offset_count)
        }
        None => (EMPTY_TEXT.as_slice(), 0),
    };
    set_editor_text(editor, text)?;
    Ok(selection_offset_count)
}

pub(super) unsafe fn set_editor_for_prepared_document(
    editor: HWND,
    text: &[u16],
    read_only: bool,
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    set_editor_read_only(editor, read_only);
    set_editor_text(editor, text)
}

pub(super) unsafe fn set_editor_empty_read_only(editor: HWND) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    set_editor_text(editor, &EMPTY_TEXT)?;
    set_editor_read_only(editor, true);
    Ok(())
}

unsafe fn set_editor_text(editor: HWND, text: &[u16]) -> Result<(), AppError> {
    if SetWindowTextW(editor, text.as_ptr()) == 0 {
        return Err(last_win32_error("set editor text"));
    }
    Ok(())
}

unsafe fn set_editor_read_only(editor: HWND, read_only: bool) {
    SendMessageW(editor, EM_SETREADONLY, usize::from(read_only), 0);
}

pub(super) unsafe fn document_editor_plain_text_utf8_reusing(
    editor: HWND,
    utf16_buffer: &mut Vec<u16>,
    utf8_buffer: &mut String,
) -> Result<(), AppError> {
    let text = document_editor_rich_edit_plain_text_utf16(editor, utf16_buffer)?;
    control_text_from_utf16_normalized_editor_into("editor", text, utf8_buffer)?;
    Ok(())
}

pub(super) unsafe fn editor_selection_utf16(editor: HWND) -> Option<(usize, usize)> {
    let range = editor_selection_char_range(editor)?;

    let start = rich_edit_offset_to_usize(range.cp_min)?;
    let end = rich_edit_offset_to_usize(range.cp_max)?;
    Some((start.min(end), start.max(end)))
}

unsafe fn editor_selection_char_range(editor: HWND) -> Option<RichEditCharRange> {
    if editor.is_null() {
        return None;
    }

    let mut range = RichEditCharRange {
        cp_min: 0,
        cp_max: 0,
    };
    SendMessageW(
        editor,
        EM_EXGETSEL_RICH_EDIT,
        0,
        &mut range as *mut RichEditCharRange as LPARAM,
    );
    Some(range)
}

pub(super) unsafe fn editor_caret_line_column(editor: HWND) -> Option<CaretLineColumn> {
    let caret = editor_caret_utf16(editor)?;
    let line_index = rich_edit_line_index_for_offset(editor, caret)?;
    let line_start = rich_edit_line_start_offset(editor, line_index).unwrap_or(0);

    Some(caret_line_column_from_offsets(
        caret, line_index, line_start,
    ))
}

unsafe fn editor_caret_utf16(editor: HWND) -> Option<usize> {
    let range = editor_selection_char_range(editor)?;
    rich_edit_offset_to_usize(range.cp_max)
}

unsafe fn rich_edit_line_index_for_offset(editor: HWND, offset: usize) -> Option<usize> {
    if editor.is_null() {
        return None;
    }

    let line_index = SendMessageW(editor, EM_LINEFROMCHAR_EDIT_CONTROL, offset as WPARAM, 0);
    rich_edit_message_result_to_usize(line_index)
}

unsafe fn rich_edit_line_start_offset(editor: HWND, line_index: usize) -> Option<usize> {
    if editor.is_null() {
        return None;
    }

    let line_start = SendMessageW(editor, EM_LINEINDEX_EDIT_CONTROL, line_index as WPARAM, 0);
    rich_edit_message_result_to_usize(line_start)
}

fn caret_line_column_from_offsets(
    caret_offset_utf16: usize,
    line_index: usize,
    line_start_offset_utf16: usize,
) -> CaretLineColumn {
    CaretLineColumn {
        line: line_index.saturating_add(1),
        column: caret_offset_utf16
            .saturating_sub(line_start_offset_utf16)
            .saturating_add(1),
    }
}

pub(super) unsafe fn editor_view_state(editor: HWND) -> DocumentTabViewState {
    let (selection_start_utf16, selection_end_utf16) =
        editor_selection_utf16(editor).unwrap_or_default();

    DocumentTabViewState {
        first_visible_line: editor_first_visible_line(editor),
        caret_position_utf16: selection_end_utf16,
        selection_start_utf16,
        selection_end_utf16,
    }
}

fn rich_edit_offset_to_usize(value: i32) -> Option<usize> {
    if value < 0 {
        Some(0)
    } else {
        usize::try_from(value).ok()
    }
}

fn rich_edit_message_result_to_usize(value: isize) -> Option<usize> {
    if value < 0 {
        Some(0)
    } else {
        usize::try_from(value).ok()
    }
}

unsafe fn editor_first_visible_line(editor: HWND) -> usize {
    if editor.is_null() {
        return 0;
    }

    let first_visible_line = SendMessageW(editor, EM_GETFIRSTVISIBLELINE, 0, 0);
    if first_visible_line < 0 {
        0
    } else {
        match usize::try_from(first_visible_line) {
            Ok(value) => value,
            Err(_) => usize::MAX,
        }
    }
}

pub(super) unsafe fn restore_editor_view_state(
    editor: HWND,
    view_state: DocumentTabViewState,
    selection_offset_count: usize,
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    let view_state = view_state.clamped(
        selection_offset_count,
        editor_max_first_visible_line(editor),
    );

    set_editor_selection_utf16(
        editor,
        view_state.selection_start_utf16,
        view_state.selection_end_utf16,
        "restore editor selection",
    )?;
    scroll_editor_to_first_visible_line(editor, view_state.first_visible_line);
    Ok(())
}

fn scroll_line_delta(current: usize, target: usize) -> i32 {
    if target >= current {
        return scroll_line_delta_from_distance(target - current);
    }

    scroll_line_delta_from_distance(current - target).saturating_neg()
}

fn scroll_line_delta_from_distance(distance: usize) -> i32 {
    i32::try_from(distance).unwrap_or(i32::MAX)
}

unsafe fn scroll_editor_to_first_visible_line(editor: HWND, first_visible_line: usize) {
    let line_delta = scroll_line_delta(editor_first_visible_line(editor), first_visible_line);
    if line_delta != 0 {
        SendMessageW(editor, EM_LINESCROLL, 0, line_delta as LPARAM);
    }
}

unsafe fn editor_max_first_visible_line(editor: HWND) -> usize {
    if editor.is_null() {
        return 0;
    }

    let line_count = SendMessageW(editor, EM_GETLINECOUNT, 0, 0);
    if line_count <= 0 {
        return 0;
    }

    match usize::try_from(line_count) {
        Ok(value) => value.saturating_sub(1),
        Err(_) => usize::MAX,
    }
}

#[cfg(test)]
fn editor_selection_offset_count(text: &str) -> usize {
    editor_offset_for_byte_index(text, text.len())
}

pub(super) unsafe fn select_editor_text_utf16(
    editor: HWND,
    start: usize,
    end: usize,
) -> Result<(), AppError> {
    if editor.is_null() {
        return Ok(());
    }

    set_editor_selection_utf16(editor, start, end, "select editor text")?;
    SendMessageW(editor, EM_SCROLLCARET, 0, 0);
    Ok(())
}

pub(super) unsafe fn highlight_editor_find_match_utf16(
    editor: HWND,
    start: usize,
    end: usize,
    colors: RichEditHighlightColors,
) -> Result<RichEditFindMatchDisplay, AppError> {
    if editor.is_null() {
        return Ok(RichEditFindMatchDisplay::SelectionOnly);
    }

    set_editor_selection_utf16(editor, start, end, "select current find match")?;
    let format = RichEditCharFormat2W::background_color(colors.background);
    let formatted = try_apply_editor_char_format(
        editor,
        SCF_SELECTION_RICH_EDIT,
        &format,
        "highlight current find match",
    );
    SendMessageW(editor, EM_SCROLLCARET, 0, 0);
    Ok(if formatted {
        RichEditFindMatchDisplay::SelectionWithTemporaryFormat
    } else {
        RichEditFindMatchDisplay::SelectionOnly
    })
}

pub(super) unsafe fn clear_editor_find_match_highlight(
    editor: HWND,
    start: usize,
    end: usize,
) -> Result<(), AppError> {
    if editor.is_null() || start >= end {
        return Ok(());
    }

    let previous_selection = editor_selection_char_range(editor);
    set_editor_selection_utf16(
        editor,
        start,
        end,
        "select current find match highlight to clear",
    )?;

    let format = RichEditCharFormat2W::auto_background();
    if editor_selection_utf16(editor).is_some_and(|(start, end)| start < end) {
        let _ = try_apply_editor_char_format(
            editor,
            SCF_SELECTION_RICH_EDIT,
            &format,
            "clear current find match highlight",
        );
    }
    if let Some(previous_selection) = previous_selection {
        set_editor_selection_char_range(editor, &previous_selection);
    }
    Ok(())
}

unsafe fn try_apply_editor_char_format(
    editor: HWND,
    scope: WPARAM,
    format: &RichEditCharFormat2W,
    _action: &'static str,
) -> bool {
    SendMessageW(
        editor,
        EM_SETCHARFORMAT_RICH_EDIT,
        scope,
        format as *const RichEditCharFormat2W as LPARAM,
    ) != 0
}

unsafe fn set_editor_selection_utf16(
    editor: HWND,
    start: usize,
    end: usize,
    action: &'static str,
) -> Result<(), AppError> {
    let range = RichEditCharRange {
        cp_min: i32::try_from(start)
            .map_err(|_| AppError::platform(action, "selection start is too large"))?,
        cp_max: i32::try_from(end)
            .map_err(|_| AppError::platform(action, "selection end is too large"))?,
    };
    set_editor_selection_char_range(editor, &range);
    Ok(())
}

unsafe fn set_editor_selection_char_range(editor: HWND, range: &RichEditCharRange) {
    SendMessageW(
        editor,
        EM_EXSETSEL_RICH_EDIT,
        0,
        range as *const RichEditCharRange as LPARAM,
    );
}

pub(super) fn byte_index_from_editor_offset(text: &str, target_units: usize) -> usize {
    byte_index_from_rich_edit_offset(text, target_units)
}

pub(super) fn byte_range_from_editor_offsets(
    text: &str,
    start_units: usize,
    end_units: usize,
) -> (usize, usize) {
    if start_units == end_units {
        let byte_index = byte_index_from_editor_offset(text, start_units);
        return (byte_index, byte_index);
    }

    let start_is_first = start_units <= end_units;
    let first_units = start_units.min(end_units);
    let second_units = start_units.max(end_units);
    let (first, second) = byte_indices_from_rich_edit_offsets(text, first_units, second_units);

    if start_is_first {
        (first, second)
    } else {
        (second, first)
    }
}

fn byte_index_from_rich_edit_offset(text: &str, target_units: usize) -> usize {
    byte_indices_from_rich_edit_offsets(text, target_units, target_units).0
}

fn byte_indices_from_rich_edit_offsets(
    text: &str,
    first_target_units: usize,
    second_target_units: usize,
) -> (usize, usize) {
    debug_assert!(first_target_units <= second_target_units);

    // Rich Edit exposes CRLF text with GT_USECRLF, but selection positions count
    // each CRLF pair as one internal paragraph mark.
    let mut units = 0usize;
    let mut first_byte_index = None;
    let mut chars = text.char_indices().peekable();
    while let Some((byte_index, character)) = chars.next() {
        if first_byte_index.is_none() && units >= first_target_units {
            first_byte_index = Some(byte_index);
        }
        if units >= second_target_units {
            return (first_byte_index.unwrap_or(byte_index), byte_index);
        }

        let next_units = if character == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
            chars.next();
            units + 1
        } else {
            units + character.len_utf16()
        };
        if first_byte_index.is_none() && next_units > first_target_units {
            first_byte_index = Some(byte_index);
        }
        if next_units > second_target_units {
            return (first_byte_index.unwrap_or(byte_index), byte_index);
        }
        units = next_units;
    }

    let end = text.len();
    (first_byte_index.unwrap_or(end), end)
}

pub(super) fn editor_range_for_byte_range(
    text: &str,
    range: Range<usize>,
) -> Result<(usize, usize), AppError> {
    validate_editor_byte_range(text, range.start, range.end)?;

    Ok(editor_offsets_for_byte_range(text, range.start, range.end))
}

pub(super) fn editor_range_for_byte_range_from_anchor(
    text: &str,
    anchor: EditorOffsetAnchor,
    range: Range<usize>,
) -> Result<(usize, usize), AppError> {
    validate_editor_byte_range(text, range.start, range.end)?;
    if anchor.byte_index > range.start {
        return Ok(editor_offsets_for_byte_range(text, range.start, range.end));
    }

    validate_editor_byte_range(text, anchor.byte_index, anchor.byte_index)?;
    let relative_start = range.start - anchor.byte_index;
    let relative_end = range.end - anchor.byte_index;
    let (start, end) =
        editor_offsets_for_byte_range(&text[anchor.byte_index..], relative_start, relative_end);
    Ok((
        anchor_editor_offset(anchor, start)?,
        anchor_editor_offset(anchor, end)?,
    ))
}

fn anchor_editor_offset(
    anchor: EditorOffsetAnchor,
    relative_offset: usize,
) -> Result<usize, AppError> {
    anchor
        .editor_offset_utf16
        .checked_add(relative_offset)
        .ok_or_else(|| AppError::platform("convert editor selection", "selection is too large"))
}

fn validate_editor_byte_range(text: &str, start: usize, end: usize) -> Result<(), AppError> {
    if start > end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return Err(AppError::platform(
            "convert editor selection",
            "selection range is not on UTF-8 character boundaries",
        ));
    }
    if splits_crlf(text, start) || splits_crlf(text, end) {
        return Err(AppError::platform(
            "convert editor selection",
            "selection range splits a CRLF line ending",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn editor_offset_for_byte_index(text: &str, target: usize) -> usize {
    editor_offsets_for_byte_range(text, target, target).0
}

fn editor_offsets_for_byte_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    debug_assert!(start <= end);

    // Match Rich Edit selection positions while keeping the app string normalized to CRLF.
    let mut units = 0usize;
    let mut start_units = None;
    let mut chars = text.char_indices().peekable();
    while let Some((byte_index, character)) = chars.next() {
        if start_units.is_none() && byte_index >= start {
            start_units = Some(units);
        }
        if byte_index >= end {
            return (start_units.unwrap_or(units), units);
        }

        if character == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
            chars.next();
            units += 1;
        } else {
            units += character.len_utf16();
        }
    }
    (start_units.unwrap_or(units), units)
}

fn splits_crlf(text: &str, byte_index: usize) -> bool {
    let bytes = text.as_bytes();
    byte_index > 0
        && byte_index < bytes.len()
        && bytes[byte_index - 1] == b'\r'
        && bytes[byte_index] == b'\n'
}

pub(super) fn normalize_editor_plain_text(value: &str) -> Cow<'_, str> {
    let mut normalized: Option<String> = None;
    let mut last_copied = 0usize;
    let mut chars = value.char_indices().peekable();

    while let Some((byte_index, character)) = chars.next() {
        match character {
            '\r' => {
                if matches!(chars.peek(), Some((_, '\n'))) {
                    chars.next();
                    continue;
                }
                append_normalized_newline(
                    value,
                    &mut normalized,
                    &mut last_copied,
                    byte_index,
                    character.len_utf8(),
                );
            }
            '\n' => {
                append_normalized_newline(
                    value,
                    &mut normalized,
                    &mut last_copied,
                    byte_index,
                    character.len_utf8(),
                );
            }
            _ => {}
        }
    }

    if let Some(mut normalized) = normalized {
        normalized.push_str(&value[last_copied..]);
        Cow::Owned(normalized)
    } else {
        Cow::Borrowed(value)
    }
}

fn append_normalized_newline(
    source: &str,
    normalized: &mut Option<String>,
    last_copied: &mut usize,
    newline_start: usize,
    newline_len: usize,
) {
    match normalized.as_mut() {
        Some(output) => output.push_str(&source[*last_copied..newline_start]),
        None => {
            let mut output = String::with_capacity(source.len() + 1);
            output.push_str(&source[..newline_start]);
            *normalized = Some(output);
        }
    }

    if let Some(output) = normalized.as_mut() {
        output.push_str("\r\n");
    }
    *last_copied = newline_start + newline_len;
}

pub(super) fn normalize_editor_plain_text_in_place(value: &mut String) -> bool {
    let normalized = match normalize_editor_plain_text(value.as_str()) {
        Cow::Borrowed(_) => return false,
        Cow::Owned(normalized) => normalized,
    };
    *value = normalized;
    true
}

#[cfg(test)]
pub(super) fn document_editor_plain_text_to_wide_null(
    action: &'static str,
    value: &str,
) -> Result<Vec<u16>, AppError> {
    let mut buffer = Vec::new();
    document_editor_plain_text_to_wide_null_reusing(action, value, &mut buffer)?;
    Ok(buffer)
}

#[cfg(test)]
fn document_editor_plain_text_to_wide_null_reusing(
    action: &'static str,
    value: &str,
    buffer: &mut Vec<u16>,
) -> Result<(), AppError> {
    let normalized = normalize_editor_plain_text(value);
    document_editor_normalized_plain_text_to_wide_null_reusing(action, normalized.as_ref(), buffer)
        .map(|_| ())
}

#[cfg(test)]
fn document_editor_normalized_plain_text_to_wide_null_reusing(
    action: &'static str,
    value: &str,
    buffer: &mut Vec<u16>,
) -> Result<usize, AppError> {
    prepare_normalized_document_editor_text_with_selection_offset_count_reusing(value, buffer)
        .map_err(|error| document_editor_text_prepare_error(action, error))
}

fn document_editor_plain_text_to_wide_null_normalizing_reusing(
    action: &'static str,
    value: &mut String,
    buffer: &mut Vec<u16>,
) -> Result<usize, AppError> {
    prepare_document_editor_text_normalizing_with_selection_offset_count_reusing(value, buffer)
        .map_err(|error| document_editor_text_prepare_error(action, error))
}

pub(super) fn prepare_normalized_document_editor_text_reusing(
    value: &str,
    buffer: &mut Vec<u16>,
) -> Result<(), DocumentEditorTextPrepareError> {
    prepare_normalized_document_editor_text_with_selection_offset_count_reusing(value, buffer)
        .map(|_| ())
}

fn prepare_normalized_document_editor_text_with_selection_offset_count_reusing(
    value: &str,
    buffer: &mut Vec<u16>,
) -> Result<usize, DocumentEditorTextPrepareError> {
    buffer.clear();
    let mut embedded_nul = false;
    let mut len_utf16 = 0usize;
    let mut selection_offset_count = 0usize;
    let mut previous_was_cr = false;
    for character in value.chars() {
        len_utf16 = len_utf16
            .checked_add(character.len_utf16())
            .ok_or(DocumentEditorTextPrepareError::Win32ControlLimitExceeded)?;
        validate_document_editor_plain_text_len_utf16(len_utf16)?;

        if previous_was_cr && character == '\n' {
            previous_was_cr = false;
        } else {
            selection_offset_count = selection_offset_count
                .checked_add(character.len_utf16())
                .ok_or(DocumentEditorTextPrepareError::Win32ControlLimitExceeded)?;
            previous_was_cr = character == '\r';
        }

        if character == '\0' {
            embedded_nul = true;
            continue;
        }
        if embedded_nul {
            continue;
        }

        let mut units = [0; 2];
        buffer.extend_from_slice(character.encode_utf16(&mut units));
    }

    if embedded_nul {
        return Err(DocumentEditorTextPrepareError::EmbeddedNul);
    }

    let len = buffer
        .len()
        .checked_add(1)
        .ok_or(DocumentEditorTextPrepareError::Win32ControlLimitExceeded)?;
    if len > i32::MAX as usize {
        return Err(DocumentEditorTextPrepareError::Win32ControlLimitExceeded);
    }

    buffer.push(0);
    Ok(selection_offset_count)
}

fn prepare_document_editor_text_normalizing_with_selection_offset_count_reusing(
    value: &mut String,
    buffer: &mut Vec<u16>,
) -> Result<usize, DocumentEditorTextPrepareError> {
    buffer.clear();
    let source = value.as_str();
    let mut normalized: Option<String> = None;
    let mut last_copied = 0usize;
    let mut chars = source.char_indices().peekable();
    let mut embedded_nul = false;
    let mut len_utf16 = 0usize;
    let mut selection_offset_count = 0usize;
    let mut previous_was_cr = false;

    while let Some((byte_index, character)) = chars.next() {
        match character {
            '\r' => {
                if matches!(chars.peek(), Some((_, '\n'))) {
                    chars.next();
                    append_prepared_document_editor_crlf(
                        buffer,
                        &mut embedded_nul,
                        &mut len_utf16,
                        &mut selection_offset_count,
                        &mut previous_was_cr,
                    )?;
                    continue;
                }
                append_normalized_newline(
                    source,
                    &mut normalized,
                    &mut last_copied,
                    byte_index,
                    character.len_utf8(),
                );
                append_prepared_document_editor_crlf(
                    buffer,
                    &mut embedded_nul,
                    &mut len_utf16,
                    &mut selection_offset_count,
                    &mut previous_was_cr,
                )?;
            }
            '\n' => {
                append_normalized_newline(
                    source,
                    &mut normalized,
                    &mut last_copied,
                    byte_index,
                    character.len_utf8(),
                );
                append_prepared_document_editor_crlf(
                    buffer,
                    &mut embedded_nul,
                    &mut len_utf16,
                    &mut selection_offset_count,
                    &mut previous_was_cr,
                )?;
            }
            _ => {
                append_prepared_document_editor_character(
                    character,
                    buffer,
                    &mut embedded_nul,
                    &mut len_utf16,
                    &mut selection_offset_count,
                    &mut previous_was_cr,
                )?;
            }
        }
    }

    if let Some(mut normalized) = normalized {
        normalized.push_str(&source[last_copied..]);
        *value = normalized;
    }

    finish_prepared_document_editor_text(buffer, embedded_nul)?;
    Ok(selection_offset_count)
}

fn append_prepared_document_editor_crlf(
    buffer: &mut Vec<u16>,
    embedded_nul: &mut bool,
    len_utf16: &mut usize,
    selection_offset_count: &mut usize,
    previous_was_cr: &mut bool,
) -> Result<(), DocumentEditorTextPrepareError> {
    append_prepared_document_editor_character(
        '\r',
        buffer,
        embedded_nul,
        len_utf16,
        selection_offset_count,
        previous_was_cr,
    )?;
    append_prepared_document_editor_character(
        '\n',
        buffer,
        embedded_nul,
        len_utf16,
        selection_offset_count,
        previous_was_cr,
    )
}

fn append_prepared_document_editor_character(
    character: char,
    buffer: &mut Vec<u16>,
    embedded_nul: &mut bool,
    len_utf16: &mut usize,
    selection_offset_count: &mut usize,
    previous_was_cr: &mut bool,
) -> Result<(), DocumentEditorTextPrepareError> {
    let character_len_utf16 = character.len_utf16();
    *len_utf16 = len_utf16
        .checked_add(character_len_utf16)
        .ok_or(DocumentEditorTextPrepareError::Win32ControlLimitExceeded)?;
    validate_document_editor_plain_text_len_utf16(*len_utf16)?;

    if *previous_was_cr && character == '\n' {
        *previous_was_cr = false;
    } else {
        *selection_offset_count = selection_offset_count
            .checked_add(character_len_utf16)
            .ok_or(DocumentEditorTextPrepareError::Win32ControlLimitExceeded)?;
        *previous_was_cr = character == '\r';
    }

    if character == '\0' {
        *embedded_nul = true;
        return Ok(());
    }
    if *embedded_nul {
        return Ok(());
    }

    let mut units = [0; 2];
    buffer.extend_from_slice(character.encode_utf16(&mut units));
    Ok(())
}

fn finish_prepared_document_editor_text(
    buffer: &mut Vec<u16>,
    embedded_nul: bool,
) -> Result<(), DocumentEditorTextPrepareError> {
    if embedded_nul {
        return Err(DocumentEditorTextPrepareError::EmbeddedNul);
    }

    let len = buffer
        .len()
        .checked_add(1)
        .ok_or(DocumentEditorTextPrepareError::Win32ControlLimitExceeded)?;
    if len > i32::MAX as usize {
        return Err(DocumentEditorTextPrepareError::Win32ControlLimitExceeded);
    }

    buffer.push(0);
    Ok(())
}

pub(super) fn prepared_document_editor_selection_offset_count(text: &[u16]) -> usize {
    let text = match text.strip_suffix(&[0]) {
        Some(text) => text,
        None => text,
    };
    let mut units = 0usize;
    let mut index = 0usize;
    while index < text.len() {
        if text[index] == b'\r' as u16 && text.get(index + 1) == Some(&(b'\n' as u16)) {
            units += 1;
            index += 2;
        } else {
            units += 1;
            index += 1;
        }
    }
    units
}

fn document_editor_text_prepare_error(
    action: &'static str,
    error: DocumentEditorTextPrepareError,
) -> AppError {
    match error {
        DocumentEditorTextPrepareError::EmbeddedNul => AppError::platform_with_user_message(
            action,
            PlatformUserMessage::Generic,
            "text contains an embedded NUL character",
        ),
        DocumentEditorTextPrepareError::TextLimitExceeded => document_editor_text_too_large(action),
        DocumentEditorTextPrepareError::Win32ControlLimitExceeded => {
            AppError::platform_with_user_message(
                action,
                PlatformUserMessage::Generic,
                "text is too large for a Win32 control",
            )
        }
    }
}

fn validate_document_editor_plain_text_len_utf16(
    len: usize,
) -> Result<(), DocumentEditorTextPrepareError> {
    if len > DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS {
        return Err(DocumentEditorTextPrepareError::TextLimitExceeded);
    }
    Ok(())
}

fn document_editor_text_too_large(_action: &'static str) -> AppError {
    AppError::text_file_too_large(
        TextFileTooLargeUserMessage::Generic,
        DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB,
    )
}

unsafe fn document_editor_rich_edit_plain_text_utf16(
    editor: HWND,
    buffer: &mut Vec<u16>,
) -> Result<&[u16], AppError> {
    if editor.is_null() {
        buffer.clear();
        return Ok(buffer.as_slice());
    }

    let len = document_editor_rich_edit_plain_text_len_utf16(editor)?;
    validate_document_editor_plain_text_len_utf16(len)
        .map_err(|error| document_editor_text_prepare_error("read editor text", error))?;
    let buffer_len = len
        .checked_add(1)
        .ok_or_else(|| AppError::platform("read editor text", "editor text is too large"))?;
    buffer.resize(buffer_len, 0);

    let byte_len = buffer_len
        .checked_mul(mem::size_of::<u16>())
        .ok_or_else(|| AppError::platform("read editor text", "editor text is too large"))?;
    let byte_len = u32::try_from(byte_len)
        .map_err(|_| AppError::platform("read editor text", "editor text is too large"))?;
    let options = RichEditGetTextEx {
        cb: byte_len,
        flags: GT_USECRLF_RICH_EDIT,
        codepage: RICH_EDIT_CP_UNICODE,
        lp_default_char: ptr::null(),
        lp_used_def_char: ptr::null_mut(),
    };

    let copied = SendMessageW(
        editor,
        EM_GETTEXTEX_RICH_EDIT,
        &options as *const RichEditGetTextEx as WPARAM,
        buffer.as_mut_ptr() as LPARAM,
    );
    if copied < 0 {
        return Err(AppError::platform(
            "read editor text",
            format!("Rich Edit text read failed with result {copied}"),
        ));
    }

    let copied = usize::try_from(copied)
        .map_err(|_| AppError::platform("read editor text", "editor text is too large"))?;
    if copied == 0 && len > 0 {
        return Err(AppError::platform(
            "read editor text",
            "editor text read returned no characters after a nonzero length",
        ));
    }

    document_editor_rich_edit_plain_text_copied_utf16(buffer, copied)
}

fn document_editor_rich_edit_plain_text_copied_utf16(
    buffer: &[u16],
    copied: usize,
) -> Result<&[u16], AppError> {
    if copied >= buffer.len() || buffer[copied] != 0 {
        return Err(AppError::platform(
            "read editor text",
            "Rich Edit text read did not include a NUL terminator",
        ));
    }

    let text = &buffer[..copied];
    if text.contains(&0) {
        return Err(document_editor_text_prepare_error(
            "read editor text",
            DocumentEditorTextPrepareError::EmbeddedNul,
        ));
    }

    Ok(text)
}

pub(super) unsafe fn document_editor_plain_text_len_utf16(editor: HWND) -> Result<usize, AppError> {
    document_editor_rich_edit_plain_text_len_utf16(editor)
}

unsafe fn document_editor_rich_edit_plain_text_len_utf16(editor: HWND) -> Result<usize, AppError> {
    let options = RichEditGetTextLengthEx {
        flags: GTL_USECRLF_RICH_EDIT | GTL_NUMCHARS_RICH_EDIT,
        codepage: RICH_EDIT_CP_UNICODE,
    };
    let len = SendMessageW(
        editor,
        EM_GETTEXTLENGTHEX_RICH_EDIT,
        &options as *const RichEditGetTextLengthEx as WPARAM,
        0,
    );
    if len < 0 {
        return Err(AppError::platform(
            "read editor text length",
            format!("Rich Edit text length failed with result {len}"),
        ));
    }

    usize::try_from(len)
        .map_err(|_| AppError::platform("read editor text length", "editor text is too large"))
}

pub(super) unsafe fn window_text_utf8(
    control: HWND,
    label: &'static str,
) -> Result<String, AppError> {
    let mut buffer = Vec::new();
    let text = window_text_utf16(control, label, &mut buffer)?;
    control_text_from_utf16(label, text)
}

unsafe fn window_text_utf16<'a>(
    control: HWND,
    label: &'static str,
    buffer: &'a mut Vec<u16>,
) -> Result<&'a [u16], AppError> {
    if control.is_null() {
        buffer.clear();
        return Ok(buffer.as_slice());
    }

    SetLastError(0);
    let len = GetWindowTextLengthW(control);
    if len == 0 {
        let error_code = GetLastError();
        if error_code != 0 {
            return Err(win32_error("read control text length", error_code));
        }
    }

    let len = usize::try_from(len).map_err(|_| {
        AppError::platform("read control text", format!("{label} text is too large"))
    })?;
    let buffer_len = len.checked_add(1).ok_or_else(|| {
        AppError::platform("read control text", format!("{label} text is too large"))
    })?;
    buffer.resize(buffer_len, 0);
    let max_count = i32::try_from(buffer.len()).map_err(|_| {
        AppError::platform("read control text", format!("{label} text is too large"))
    })?;

    SetLastError(0);
    let copied = GetWindowTextW(control, buffer.as_mut_ptr(), max_count);
    if copied == 0 {
        let error_code = GetLastError();
        if error_code != 0 {
            return Err(win32_error("read control text", error_code));
        }
        if len > 0 {
            return Err(AppError::platform(
                "read control text",
                format!("{label} text read returned no characters after a nonzero length"),
            ));
        }
    }

    let copied = usize::try_from(copied).map_err(|_| {
        AppError::platform("read control text", format!("{label} text is too large"))
    })?;
    Ok(&buffer[..copied])
}

fn control_text_from_utf16(label: &'static str, text: &[u16]) -> Result<String, AppError> {
    String::from_utf16(text).map_err(|_| {
        AppError::platform(
            "read control text",
            format!("{label} text contains invalid UTF-16"),
        )
    })
}

fn control_text_from_utf16_normalized_editor_into(
    label: &'static str,
    text: &[u16],
    output: &mut String,
) -> Result<(), AppError> {
    output.clear();
    output.reserve(text.len());

    let mut pending_cr = false;
    for item in std::char::decode_utf16(text.iter().copied()) {
        let character = item.map_err(|_| {
            AppError::platform(
                "read control text",
                format!("{label} text contains invalid UTF-16"),
            )
        })?;

        if pending_cr {
            if character == '\n' {
                output.push_str("\r\n");
                pending_cr = false;
                continue;
            }

            output.push_str("\r\n");
            pending_cr = false;
        }

        match character {
            '\r' => pending_cr = true,
            '\n' => output.push_str("\r\n"),
            _ => output.push(character),
        }
    }

    if pending_cr {
        output.push_str("\r\n");
    }
    Ok(())
}

pub(super) unsafe fn set_search_text(search: HWND, text: &str) -> Result<(), AppError> {
    let text = utf8_to_wide_null("convert search text from UTF-8 to UTF-16", text)?;
    let result = SetWindowTextW(search, text.as_ptr());

    if result == 0 {
        return Err(last_win32_error("set search text"));
    }
    Ok(())
}

pub(super) fn utf8_to_wide_null(action: &'static str, value: &str) -> Result<Vec<u16>, AppError> {
    utf8_to_wide_null_with_user_message(action, PlatformUserMessage::Generic, value)
}

pub(super) fn utf8_to_wide_null_with_user_message(
    action: &'static str,
    user_message: PlatformUserMessage,
    value: &str,
) -> Result<Vec<u16>, AppError> {
    if value.contains('\0') {
        return Err(AppError::platform_with_user_message(
            action,
            user_message,
            "text contains an embedded NUL character",
        ));
    }

    let wide: Vec<u16> = value.encode_utf16().chain(iter::once(0)).collect();
    if wide.len() > i32::MAX as usize {
        return Err(AppError::platform_with_user_message(
            action,
            user_message,
            "text is too large for a Win32 control",
        ));
    }

    Ok(wide)
}

pub(super) fn utf8_to_wide_null_lossy(value: &str) -> Vec<u16> {
    value
        .chars()
        .map(|character| if character == '\0' { ' ' } else { character })
        .collect::<String>()
        .encode_utf16()
        .chain(iter::once(0))
        .collect()
}

pub(super) unsafe fn wide_null_to_string(value: *const u16) -> String {
    // SAFETY: Callers pass Win32-provided NUL-terminated UTF-16 text pointers from controls.
    let mut len = 0usize;
    while *value.add(len) != 0 {
        len += 1;
    }

    String::from_utf16_lossy(std::slice::from_raw_parts(value, len))
}

#[cfg(test)]
mod tests {
    use super::super::common::{
        CFE_AUTOBACKCOLOR_RICH_EDIT, CFM_BACKCOLOR_RICH_EDIT, DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME,
        EM_SETTEXTMODE_RICH_EDIT, RICH_EDIT_MODULE_NAME, SCF_SELECTION_RICH_EDIT,
        TM_PLAINTEXT_RICH_EDIT,
    };
    use super::super::test_support::{enter_win32_control_test, Win32ControlTestGuard};
    use super::*;
    use crate::domain::find_next_literal;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, ES_MULTILINE, WM_CHAR, WM_CLEAR, WM_CUT,
        WS_OVERLAPPED,
    };

    const EM_GETSELTEXT_RICH_EDIT: u32 = 0x043E;
    const EM_GETCHARFORMAT_RICH_EDIT: u32 = 0x043A;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestBackgroundColor {
        Auto,
        Explicit(COLORREF),
    }

    fn range_for(text: &str, needle: &str) -> Range<usize> {
        match text.find(needle) {
            Some(start) => start..start + needle.len(),
            None => panic!("test needle should exist"),
        }
    }

    fn checked_editor_range(text: &str, range: Range<usize>) -> (usize, usize) {
        match editor_range_for_byte_range(text, range) {
            Ok(value) => value,
            Err(error) => panic!("range should convert: {error}"),
        }
    }

    fn wide_from_editor_text(text: &str) -> Vec<u16> {
        match document_editor_plain_text_to_wide_null("test editor text conversion", text) {
            Ok(value) => value,
            Err(error) => panic!("editor text should convert: {error}"),
        }
    }

    fn assert_byte_index_for_editor_offsets(text: &str, expected: &[(usize, usize)]) {
        for &(editor_offset, byte_index) in expected {
            assert_eq!(
                byte_index_from_editor_offset(text, editor_offset),
                byte_index,
                "text={text:?}, editor_offset={editor_offset}"
            );
        }
    }

    struct TestRichEdit(HWND, #[allow(dead_code)] Win32ControlTestGuard);

    impl TestRichEdit {
        unsafe fn create() -> Self {
            Self::create_with_plain_text_mode(true)
        }

        unsafe fn create_rich_text() -> Self {
            Self::create_with_plain_text_mode(false)
        }

        unsafe fn create_with_plain_text_mode(plain_text: bool) -> Self {
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
                200,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                panic!("test Rich Edit control should be created");
            }

            if plain_text {
                let text_mode_result =
                    SendMessageW(hwnd, EM_SETTEXTMODE_RICH_EDIT, TM_PLAINTEXT_RICH_EDIT, 0);
                if text_mode_result != 0 {
                    DestroyWindow(hwnd);
                    panic!("test Rich Edit control should accept plain text mode");
                }
            }

            Self(hwnd, guard)
        }
    }

    impl Drop for TestRichEdit {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    unsafe fn selected_rich_edit_text(editor: HWND) -> String {
        let mut buffer = vec![0u16; 64];
        let copied = SendMessageW(
            editor,
            EM_GETSELTEXT_RICH_EDIT,
            0,
            buffer.as_mut_ptr() as LPARAM,
        );
        assert!(copied >= 0, "selected Rich Edit text should be readable");
        let len = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        control_text_from_utf16("selected editor text", &buffer[..len])
            .expect("selected Rich Edit text should be valid UTF-16")
    }

    unsafe fn editor_text(editor: HWND) -> String {
        let mut utf16 = Vec::new();
        let mut utf8 = String::new();
        document_editor_plain_text_utf8_reusing(editor, &mut utf16, &mut utf8)
            .expect("editor text should be readable");
        utf8
    }

    unsafe fn selected_background_color(editor: HWND) -> TestBackgroundColor {
        let mut format = RichEditCharFormat2W::auto_background();
        let mask = SendMessageW(
            editor,
            EM_GETCHARFORMAT_RICH_EDIT,
            SCF_SELECTION_RICH_EDIT,
            &mut format as *mut RichEditCharFormat2W as LPARAM,
        ) as u32;

        assert_eq!(mask & CFM_BACKCOLOR_RICH_EDIT, CFM_BACKCOLOR_RICH_EDIT);
        if format.effects & CFE_AUTOBACKCOLOR_RICH_EDIT != 0 {
            TestBackgroundColor::Auto
        } else {
            TestBackgroundColor::Explicit(format.back_color)
        }
    }

    #[test]
    fn control_text_from_utf16_decodes_valid_surrogate_pairs() {
        let units = [b'a' as u16, 0xD83D, 0xDE80];

        match control_text_from_utf16("editor", &units) {
            Ok(value) => assert_eq!(value, "a🚀"),
            Err(error) => panic!("valid UTF-16 should decode: {error}"),
        }
    }

    #[test]
    fn control_text_from_utf16_rejects_unpaired_surrogate() {
        let units = [b'a' as u16, 0xD800, b'b' as u16];

        let error = match control_text_from_utf16("editor", &units) {
            Ok(value) => panic!("invalid UTF-16 should be rejected, got {value:?}"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("invalid UTF-16"));
    }

    #[test]
    fn editor_utf16_text_decodes_and_normalizes_newlines_in_one_pass() {
        let units: Vec<u16> = "첫줄\n둘째\r셋째\r\n넷째🚀".encode_utf16().collect();
        let mut output = String::from("stale");

        match control_text_from_utf16_normalized_editor_into("editor", &units, &mut output) {
            Ok(()) => assert_eq!(output, "첫줄\r\n둘째\r\n셋째\r\n넷째🚀"),
            Err(error) => panic!("valid UTF-16 should decode and normalize: {error}"),
        }
    }

    #[test]
    fn editor_utf16_text_normalized_decode_rejects_unpaired_surrogate() {
        let units = [b'a' as u16, 0xD800, b'\n' as u16];
        let mut output = String::new();

        let error =
            match control_text_from_utf16_normalized_editor_into("editor", &units, &mut output) {
                Ok(()) => panic!("invalid UTF-16 should be rejected"),
                Err(error) => error,
            };

        assert!(error.to_string().contains("invalid UTF-16"));
    }

    #[test]
    fn rich_edit_copied_text_returns_copied_units_before_terminator() {
        let buffer = [b'a' as u16, b'b' as u16, 0, b'c' as u16];

        match document_editor_rich_edit_plain_text_copied_utf16(&buffer, 2) {
            Ok(text) => assert_eq!(text, &[b'a' as u16, b'b' as u16]),
            Err(error) => panic!("copied Rich Edit text should be accepted: {error}"),
        }
    }

    #[test]
    fn rich_edit_copied_text_rejects_embedded_nul_without_truncation() {
        let buffer = [b'a' as u16, 0, b'b' as u16, 0];

        let error = match document_editor_rich_edit_plain_text_copied_utf16(&buffer, 3) {
            Ok(text) => panic!("embedded NUL Rich Edit text should be rejected, got {text:?}"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("embedded NUL"));
    }

    #[test]
    fn editor_plain_text_normalizes_lf_and_lone_cr_to_crlf() {
        assert_eq!(normalize_editor_plain_text("").as_ref(), "");
        assert_eq!(normalize_editor_plain_text("ASCII").as_ref(), "ASCII");
        assert_eq!(
            normalize_editor_plain_text("첫줄\n둘째\r셋째\r\n넷째").as_ref(),
            "첫줄\r\n둘째\r\n셋째\r\n넷째"
        );
        assert_eq!(
            normalize_editor_plain_text("A\r\nB\r\nC").as_ref(),
            "A\r\nB\r\nC"
        );

        let mut text = "한글\r\n🚀".to_owned();
        assert!(!normalize_editor_plain_text_in_place(&mut text));
        assert_eq!(text, "한글\r\n🚀");

        let mut text = "한글\n🚀".to_owned();
        assert!(normalize_editor_plain_text_in_place(&mut text));
        assert_eq!(text, "한글\r\n🚀");
    }

    #[test]
    fn document_editor_plain_text_to_wide_null_keeps_plain_text_and_crlf_policy() {
        let actual = wide_from_editor_text("한글\n🚀\r끝");
        let expected: Vec<u16> = "한글\r\n🚀\r\n끝\0".encode_utf16().collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn document_editor_plain_text_len_limit_rejects_oversized_sync() {
        assert!(validate_document_editor_plain_text_len_utf16(
            DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS
        )
        .is_ok());

        let error = match validate_document_editor_plain_text_len_utf16(
            DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS + 1,
        ) {
            Ok(()) => panic!("oversized editor text should be rejected"),
            Err(error) => error,
        };

        match error {
            DocumentEditorTextPrepareError::TextLimitExceeded => {}
            other => panic!("expected text-too-large error, got {other:?}"),
        }

        let error = document_editor_text_prepare_error(
            "read editor text",
            DocumentEditorTextPrepareError::TextLimitExceeded,
        );
        match error {
            AppError::TextFileTooLarge { limit_mib, .. } => {
                assert_eq!(limit_mib, DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB);
            }
            other => panic!("expected text-too-large error, got {other}"),
        }
    }

    #[test]
    fn programmatic_text_update_guard_restores_previous_suppression_state() {
        let suppress_change = Cell::new(false);

        {
            let _outer_guard = ProgrammaticTextUpdateGuard::enter(&suppress_change);
            assert!(suppress_change.get());
            {
                let _inner_guard = ProgrammaticTextUpdateGuard::enter(&suppress_change);
                assert!(suppress_change.get());
            }
            assert!(suppress_change.get());
        }

        assert!(!suppress_change.get());

        suppress_change.set(true);
        {
            let _guard = ProgrammaticTextUpdateGuard::enter(&suppress_change);
            assert!(suppress_change.get());
        }
        assert!(suppress_change.get());
    }

    #[test]
    fn read_only_document_editor_rejects_cut_delete_and_typing() {
        unsafe {
            let editor = TestRichEdit::create();
            set_editor_for_document(editor.0, Some("alpha beta"), true)
                .expect("read-only editor text should load");

            select_editor_text_utf16(editor.0, 0, 5).expect("selection should be set");
            SendMessageW(editor.0, WM_CLEAR, 0, 0);
            assert_eq!(editor_text(editor.0), "alpha beta");

            select_editor_text_utf16(editor.0, 0, 5).expect("selection should be set");
            SendMessageW(editor.0, WM_CUT, 0, 0);
            assert_eq!(editor_text(editor.0), "alpha beta");

            select_editor_text_utf16(editor.0, 10, 10).expect("selection should be set");
            SendMessageW(editor.0, WM_CHAR, '!' as WPARAM, 0);
            assert_eq!(editor_text(editor.0), "alpha beta");
        }
    }

    #[test]
    fn editor_offsets_cover_required_utf8_and_rich_edit_cases() {
        assert_eq!(checked_editor_range("", 0..0), (0, 0));
        assert_byte_index_for_editor_offsets("", &[(0, 0), (1, 0)]);

        let ascii = "Alpha";
        assert_eq!(checked_editor_range(ascii, 0..ascii.len()), (0, 5));
        assert_byte_index_for_editor_offsets(ascii, &[(0, 0), (1, 1), (5, 5), (99, 5)]);

        let hangul = "A한글Z";
        assert_eq!(checked_editor_range(hangul, 1..7), (1, 3));
        assert_byte_index_for_editor_offsets(hangul, &[(0, 0), (1, 1), (2, 4), (3, 7), (4, 8)]);

        let emoji = "A🚀Z";
        assert_eq!(checked_editor_range(emoji, 1..5), (1, 3));
        assert_byte_index_for_editor_offsets(emoji, &[(0, 0), (1, 1), (2, 1), (3, 5), (4, 6)]);

        let crlf = "A\r\nB\r\n한🚀";
        assert_eq!(checked_editor_range(crlf, 3..crlf.len()), (2, 7));
        assert_byte_index_for_editor_offsets(
            crlf,
            &[
                (0, 0),
                (1, 1),
                (2, 3),
                (3, 4),
                (4, 6),
                (5, 9),
                (6, 9),
                (7, 13),
            ],
        );
    }

    #[test]
    fn editor_offsets_cover_empty_ascii_and_very_long_text() {
        assert_eq!(checked_editor_range("", 0..0), (0, 0));
        assert_eq!(byte_index_from_editor_offset("", 100), 0);

        let ascii = "ASCII text";
        let ascii_range = range_for(ascii, "text");
        assert_eq!(checked_editor_range(ascii, ascii_range.clone()), (6, 10));
        assert_eq!(byte_index_from_editor_offset(ascii, 6), ascii_range.start);
        assert_eq!(byte_index_from_editor_offset(ascii, 10), ascii_range.end);

        let prefix = "a".repeat(20_000);
        let long = format!("{prefix}\r\n끝🚀");
        let range = range_for(&long, "끝🚀");
        assert_eq!(checked_editor_range(&long, range.clone()), (20_001, 20_004));
        assert_eq!(byte_index_from_editor_offset(&long, 20_001), range.start);
        assert_eq!(byte_index_from_editor_offset(&long, 20_004), range.end);
    }

    #[test]
    fn editor_offset_range_conversions_cover_edge_cases() {
        assert_eq!(byte_range_from_editor_offsets("", 0, 100), (0, 0));

        let text = "A\r\n한🚀B";
        let hangul_rocket = range_for(text, "한🚀");
        assert_eq!(checked_editor_range(text, hangul_rocket.clone()), (2, 5));
        assert_eq!(
            byte_range_from_editor_offsets(text, 2, 5),
            (hangul_rocket.start, hangul_rocket.end)
        );

        let rocket = range_for(text, "🚀");
        assert_eq!(
            byte_range_from_editor_offsets(text, 4, 5),
            (rocket.start, rocket.end)
        );

        let prefix = "a".repeat(20_000);
        let long = format!("{prefix}\r\n끝🚀");
        let long_range = range_for(&long, "끝🚀");
        assert_eq!(
            checked_editor_range(&long, long_range.clone()),
            (20_001, 20_004)
        );
        assert_eq!(
            byte_range_from_editor_offsets(&long, 20_001, 20_004),
            (long_range.start, long_range.end)
        );
    }

    #[test]
    fn editor_range_from_anchor_converts_relative_suffix() {
        let prefix = "a".repeat(20_000);
        let text = format!("{prefix}\r\n앞\r\n끝🚀");
        let anchor_byte_index = prefix.len();
        let target = range_for(&text, "끝🚀");

        assert_eq!(
            editor_range_for_byte_range_from_anchor(
                &text,
                EditorOffsetAnchor {
                    byte_index: anchor_byte_index,
                    editor_offset_utf16: 20_000,
                },
                target,
            )
            .expect("anchored range should convert"),
            (20_003, 20_006)
        );
    }

    #[test]
    fn editor_offsets_fold_crlf_for_rich_edit_selection_positions() {
        let text = "첫줄\r\n둘째 한글\r\n셋째";
        let range = range_for(text, "한글");

        assert_eq!(checked_editor_range(text, range.clone()), (6, 8));
        assert_eq!(byte_index_from_editor_offset(text, 6), range.start);
        assert_eq!(byte_index_from_editor_offset(text, 8), range.end);
    }

    #[test]
    fn find_result_range_converts_to_rich_edit_utf16_selection_offsets() {
        let text = "첫줄\r\n둘째 🚀 한글\r\n끝";
        let found = match find_next_literal(text, "🚀 한글", 0) {
            Some(found) => found,
            None => panic!("find target should exist"),
        };

        assert_eq!(checked_editor_range(text, found.start..found.end), (6, 11));
        assert_eq!(byte_index_from_editor_offset(text, 6), found.start);
        assert_eq!(byte_index_from_editor_offset(text, 11), found.end);
    }

    #[test]
    fn editor_offsets_keep_surrogate_pairs_as_two_units() {
        let text = "a\r\n🚀한글";
        let rocket = range_for(text, "🚀");
        let korean = range_for(text, "한글");

        assert_eq!(checked_editor_range(text, rocket.clone()), (2, 4));
        assert_eq!(checked_editor_range(text, korean.clone()), (4, 6));
        assert_eq!(byte_index_from_editor_offset(text, 3), rocket.start);
        assert_eq!(byte_index_from_editor_offset(text, 4), korean.start);
    }

    #[test]
    fn editor_offsets_round_trip_hangul_emoji_and_multiple_crlf_lines() {
        let text = "한글\r\nemoji 🚀\r\n끝";
        let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
        boundaries.push(text.len());

        for byte_index in boundaries {
            if splits_crlf(text, byte_index) {
                continue;
            }
            let units = editor_offset_for_byte_index(text, byte_index);
            assert_eq!(byte_index_from_editor_offset(text, units), byte_index);
            assert_eq!(
                checked_editor_range(text, byte_index..byte_index),
                (units, units)
            );
        }

        let rocket = range_for(text, "🚀");
        assert_eq!(checked_editor_range(text, rocket), (9, 11));
    }

    #[test]
    fn editor_range_rejects_boundaries_inside_crlf() {
        let text = "A\r\nB";
        let split = text.find('\n').expect("LF should exist");
        let error = match editor_range_for_byte_range(text, split..split) {
            Ok(value) => panic!("CRLF split boundary should be rejected, got {value:?}"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("CRLF"));
    }

    #[test]
    fn editor_range_rejects_utf8_character_boundary_mismatch() {
        let hangul = "A한B";
        let error = match editor_range_for_byte_range(hangul, 2..2) {
            Ok(value) => panic!("non-boundary Hangul range should be rejected, got {value:?}"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("UTF-8 character boundaries"));

        let emoji = "A🚀B";
        let error = match editor_range_for_byte_range(emoji, 1..2) {
            Ok(value) => panic!("non-boundary emoji range should be rejected, got {value:?}"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("UTF-8 character boundaries"));
    }

    #[test]
    fn normalized_editor_text_keeps_lf_input_selection_compatible_with_crlf() {
        let normalized = normalize_editor_plain_text("첫줄\n둘째 🚀\n끝");
        let text = normalized.as_ref();
        let rocket = range_for(text, "🚀");

        assert_eq!(text, "첫줄\r\n둘째 🚀\r\n끝");
        assert_eq!(checked_editor_range(text, rocket.clone()), (6, 8));
        assert_eq!(byte_index_from_editor_offset(text, 6), rocket.start);
        assert_eq!(byte_index_from_editor_offset(text, 8), rocket.end);
    }

    #[test]
    fn rich_edit_selection_offsets_select_crlf_plain_text_ranges() {
        let editor = unsafe { TestRichEdit::create() };
        let content = "A\r\nB🚀\r\n끝";
        let wide = wide_from_editor_text(content);
        let target = range_for(content, "B🚀");
        let (start, end) = checked_editor_range(content, target);

        unsafe {
            assert_ne!(
                SetWindowTextW(editor.0, wide.as_ptr()),
                0,
                "test Rich Edit text should be set"
            );
            select_editor_text_utf16(editor.0, start, end)
                .expect("test Rich Edit selection should be set");

            assert_eq!(editor_selection_utf16(editor.0), Some((start, end)));
            assert_eq!(selected_rich_edit_text(editor.0), "B🚀");
        }
    }

    #[test]
    fn current_find_match_highlight_is_temporary_formatting_only() {
        let editor = unsafe { TestRichEdit::create_rich_text() };
        let content = "A\r\nB🚀 target\r\n끝";
        let target = range_for(content, "B🚀 target");
        let (start, end) = checked_editor_range(content, target);
        let colors = RichEditHighlightColors {
            background: 0x0040_80ff,
        };
        let existing_background = 0x0011_2233;

        unsafe {
            set_editor_for_document(editor.0, Some(content), false)
                .expect("test Rich Edit text should load");
            select_editor_text_utf16(editor.0, 0, 1).expect("existing selection should be set");
            let existing_format = RichEditCharFormat2W::background_color(existing_background);
            assert!(
                try_apply_editor_char_format(
                    editor.0,
                    SCF_SELECTION_RICH_EDIT,
                    &existing_format,
                    "apply existing test background"
                ),
                "existing background should apply"
            );

            let display = highlight_editor_find_match_utf16(editor.0, start, end, colors)
                .expect("current find match should highlight");

            assert_eq!(editor_text(editor.0), content);
            assert_eq!(editor_selection_utf16(editor.0), Some((start, end)));
            if display == RichEditFindMatchDisplay::SelectionWithTemporaryFormat {
                assert_eq!(
                    selected_background_color(editor.0),
                    TestBackgroundColor::Explicit(colors.background)
                );
            }

            select_editor_text_utf16(editor.0, 0, 1).expect("selection should move");
            assert_eq!(
                selected_background_color(editor.0),
                TestBackgroundColor::Explicit(existing_background)
            );
            clear_editor_find_match_highlight(editor.0, start, end)
                .expect("highlight should clear");
            assert_eq!(editor_text(editor.0), content);
            assert_eq!(editor_selection_utf16(editor.0), Some((0, 1)));
            assert_eq!(
                selected_background_color(editor.0),
                TestBackgroundColor::Explicit(existing_background)
            );

            select_editor_text_utf16(editor.0, start, end)
                .expect("target selection should restore");
            assert_eq!(
                selected_background_color(editor.0),
                TestBackgroundColor::Auto
            );
        }
    }

    #[test]
    fn normalizing_prepare_reuses_already_crlf_content() {
        let mut content = "A\r\nB🚀\r\n끝".repeat(1024);
        let original = content.clone();
        let original_ptr = content.as_ptr();
        let mut buffer = Vec::new();

        let selection_offset_count =
            prepare_document_editor_text_normalizing_with_selection_offset_count_reusing(
                &mut content,
                &mut buffer,
            )
            .expect("already-normalized content should convert");

        let expected_buffer: Vec<u16> = original.encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(content, original);
        assert_eq!(content.as_ptr(), original_ptr);
        assert_eq!(buffer, expected_buffer);
        assert_eq!(selection_offset_count, 7 * 1024);
    }

    #[test]
    fn normalizing_prepare_converts_lf_and_lone_cr_to_crlf() {
        let mut content = String::from("A\nB\rC\r\nD");
        let mut buffer = Vec::new();

        let selection_offset_count =
            prepare_document_editor_text_normalizing_with_selection_offset_count_reusing(
                &mut content,
                &mut buffer,
            )
            .expect("plain text should normalize and convert");

        let expected = "A\r\nB\r\nC\r\nD";
        let expected_buffer: Vec<u16> = expected.encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(content, expected);
        assert_eq!(buffer, expected_buffer);
        assert_eq!(selection_offset_count, 7);
    }

    #[test]
    fn editor_selection_offset_count_matches_rich_edit_selection_units() {
        assert_eq!(editor_selection_offset_count(""), 0);
        assert_eq!(editor_selection_offset_count("ASCII"), 5);
        assert_eq!(editor_selection_offset_count("한🚀"), 3);
        assert_eq!(editor_selection_offset_count("A\r\nB🚀\r\n끝"), 7);

        let mut buffer = Vec::new();
        let prepared_count =
            prepare_normalized_document_editor_text_with_selection_offset_count_reusing(
                "A\r\nB🚀\r\n끝",
                &mut buffer,
            )
            .expect("prepared editor text should convert");
        assert_eq!(prepared_count, 7);
        assert_eq!(prepared_document_editor_selection_offset_count(&buffer), 7);
    }

    #[test]
    fn caret_line_column_from_offsets_is_one_based_and_utf16_based() {
        assert_eq!(
            caret_line_column_from_offsets(0, 0, 0),
            CaretLineColumn { line: 1, column: 1 }
        );
        assert_eq!(
            caret_line_column_from_offsets(5, 1, 2),
            CaretLineColumn { line: 2, column: 4 }
        );
    }

    #[test]
    fn editor_caret_line_column_tracks_rich_edit_utf16_columns() {
        let editor = unsafe { TestRichEdit::create() };
        let content = "A\r\nB🚀\r\n끝";
        let rocket_end = checked_editor_range(content, range_for(content, "B🚀")).1;

        unsafe {
            set_editor_for_document(editor.0, Some(content), false)
                .expect("test Rich Edit text should load");
            select_editor_text_utf16(editor.0, rocket_end, rocket_end)
                .expect("test Rich Edit caret should be set");

            assert_eq!(
                editor_caret_line_column(editor.0),
                Some(CaretLineColumn { line: 2, column: 4 })
            );
        }
    }

    #[test]
    fn editor_view_state_captures_selection_and_restore_clamps_to_content() {
        let editor = unsafe { TestRichEdit::create() };
        let content = "A\r\nB🚀\r\n끝";
        let target = range_for(content, "B🚀");
        let (start, end) = checked_editor_range(content, target);

        unsafe {
            set_editor_for_document(editor.0, Some(content), false)
                .expect("test Rich Edit text should load");
            select_editor_text_utf16(editor.0, start, end)
                .expect("test Rich Edit selection should be set");

            let view_state = editor_view_state(editor.0);
            assert_eq!(view_state.selection_start_utf16, start);
            assert_eq!(view_state.selection_end_utf16, end);
            assert_eq!(view_state.caret_position_utf16, end);

            set_editor_for_document(editor.0, Some("A"), false)
                .expect("replacement Rich Edit text should load");
            restore_editor_view_state(editor.0, view_state, editor_selection_offset_count("A"))
                .expect("editor view state should restore");

            assert_eq!(editor_selection_utf16(editor.0), Some((1, 1)));
        }
    }

    #[test]
    fn scroll_line_delta_moves_from_current_to_target() {
        assert_eq!(scroll_line_delta(0, 24), 24);
        assert_eq!(scroll_line_delta(24, 3), -21);
        assert_eq!(scroll_line_delta(5, 5), 0);
    }

    #[test]
    fn scroll_line_delta_clamps_oversized_values() {
        assert_eq!(scroll_line_delta(0, usize::MAX), i32::MAX);
        assert_eq!(scroll_line_delta(usize::MAX, 0), i32::MIN + 1);
    }
}
