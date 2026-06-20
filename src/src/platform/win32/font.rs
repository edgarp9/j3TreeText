use std::cell::Cell;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, EnumFontFamiliesExW, GetDC, ReleaseDC, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HFONT, HGDIOBJ,
    LOGFONTW, OUT_DEFAULT_PRECIS, TEXTMETRICW,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    ChooseFontW, CommDlgExtendedError, CF_FORCEFONTEXIST, CF_INITTOLOGFONTSTRUCT, CF_LIMITSIZE,
    CF_NOVERTFONTS, CF_SCREENFONTS, CHOOSEFONTW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_SETFONT};

use super::dpi::dpi_y_for_window;
use super::i18n::ui_text;
use super::text::{utf8_to_wide_null_with_user_message, ProgrammaticTextUpdateGuard};
use crate::domain::{
    EditorFontSettings, UiLanguage, MAX_EDITOR_FONT_SIZE_PT, MIN_EDITOR_FONT_SIZE_PT,
};
use crate::error::{AppError, PlatformUserMessage};

const LF_FACE_SIZE: usize = 32;

pub(super) struct AppliedEditorFont {
    pub(super) settings: EditorFontSettings,
    pub(super) handle: EditorFontHandle,
    pub(super) used_fallback: bool,
}

pub(super) struct EditorFontHandle {
    handle: HFONT,
}

impl EditorFontHandle {
    fn new(handle: HFONT) -> Self {
        Self { handle }
    }

    fn raw(&self) -> HFONT {
        self.handle
    }
}

impl Drop for EditorFontHandle {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }

        // SAFETY: The handle was returned by CreateFontW and ownership is kept in this RAII
        // wrapper. Callers replace the control font before dropping an older handle.
        unsafe {
            DeleteObject(self.handle as HGDIOBJ);
        }
    }
}

pub(super) unsafe fn choose_editor_font(
    owner: HWND,
    current: &EditorFontSettings,
    language: UiLanguage,
) -> Result<Option<EditorFontSettings>, AppError> {
    let mut logfont = logfont_from_settings(owner, current);
    let mut choose_font = CHOOSEFONTW {
        lStructSize: std::mem::size_of::<CHOOSEFONTW>() as u32,
        hwndOwner: owner,
        hDC: ptr::null_mut(),
        lpLogFont: &mut logfont,
        iPointSize: current.size_pt.saturating_mul(10),
        Flags: CF_SCREENFONTS
            | CF_INITTOLOGFONTSTRUCT
            | CF_LIMITSIZE
            | CF_FORCEFONTEXIST
            | CF_NOVERTFONTS,
        rgbColors: 0,
        lCustData: 0,
        lpfnHook: None,
        lpTemplateName: ptr::null(),
        hInstance: ptr::null_mut(),
        lpszStyle: ptr::null_mut(),
        nFontType: 0,
        ___MISSING_ALIGNMENT__: 0,
        nSizeMin: MIN_EDITOR_FONT_SIZE_PT,
        nSizeMax: MAX_EDITOR_FONT_SIZE_PT,
    };

    // SAFETY: choose_font points to a fully initialized CHOOSEFONTW, and lpLogFont points to a
    // stack LOGFONTW that lives until the synchronous dialog returns.
    if ChooseFontW(&mut choose_font) == 0 {
        let code = CommDlgExtendedError();
        if code == 0 {
            return Ok(None);
        }

        return Err(AppError::user(ui_text(language).font_dialog_error(code)));
    }

    let family = wide_fixed_to_string(&logfont.lfFaceName);
    let size_pt = selected_point_size(owner, choose_font.iPointSize, logfont.lfHeight);
    Ok(Some(EditorFontSettings::new(family, size_pt)))
}

pub(super) unsafe fn create_editor_font(
    editor: HWND,
    requested: &EditorFontSettings,
) -> Result<AppliedEditorFont, AppError> {
    if editor.is_null() {
        return Err(AppError::platform_with_user_message(
            "create editor font",
            PlatformUserMessage::Font,
            "editor control was not created",
        ));
    }

    let default_settings = EditorFontSettings::default();
    let resolved = if font_family_available(editor, &requested.family) {
        requested.clone()
    } else {
        default_settings.clone()
    };
    let mut used_fallback = resolved != *requested;

    match create_font_handle(editor, &resolved) {
        Ok(handle) => Ok(AppliedEditorFont {
            settings: resolved,
            handle,
            used_fallback,
        }),
        Err(_) if resolved != default_settings => {
            let handle = create_font_handle(editor, &default_settings)?;
            used_fallback = true;
            Ok(AppliedEditorFont {
                settings: default_settings,
                handle,
                used_fallback,
            })
        }
        Err(error) => Err(error),
    }
}

pub(super) unsafe fn set_editor_font(
    editor: HWND,
    suppress_editor_change: &Cell<bool>,
    handle: &EditorFontHandle,
) {
    if editor.is_null() {
        return;
    }

    // SAFETY: The HFONT remains owned by WindowState after this call and is not deleted while it is
    // selected by the editor control.
    let _guard = ProgrammaticTextUpdateGuard::enter(suppress_editor_change);
    SendMessageW(editor, WM_SETFONT, handle.raw() as usize, 1);
}

unsafe fn create_font_handle(
    editor: HWND,
    settings: &EditorFontSettings,
) -> Result<EditorFontHandle, AppError> {
    let height = logical_height_from_point_size(settings.size_pt, dpi_y_for_window(editor));
    let family = utf8_to_wide_null_with_user_message(
        "convert editor font family",
        PlatformUserMessage::Font,
        &settings.family,
    )?;

    // SAFETY: family is a NUL-terminated UTF-16 face name. Other parameters are bounded domain
    // values or documented GDI defaults.
    let handle = CreateFontW(
        height,
        0,
        0,
        0,
        FW_NORMAL as i32,
        0,
        0,
        0,
        u32::from(DEFAULT_CHARSET),
        u32::from(OUT_DEFAULT_PRECIS),
        u32::from(CLIP_DEFAULT_PRECIS),
        u32::from(CLEARTYPE_QUALITY),
        u32::from(DEFAULT_PITCH | FF_DONTCARE),
        family.as_ptr(),
    );

    if handle.is_null() {
        return Err(AppError::platform_with_user_message(
            "create editor font",
            PlatformUserMessage::Font,
            "CreateFontW returned NULL",
        ));
    }

    Ok(EditorFontHandle::new(handle))
}

unsafe fn font_family_available(hwnd: HWND, family: &str) -> bool {
    let hdc = GetDC(hwnd);
    if hdc.is_null() {
        return false;
    }

    let mut query = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    copy_face_name(&mut query.lfFaceName, family);

    let mut found = false;
    let found_ptr = &mut found as *mut bool;
    // SAFETY: hdc is valid until ReleaseDC below. query is initialized, and found_ptr remains valid
    // for the synchronous EnumFontFamiliesExW callback.
    EnumFontFamiliesExW(hdc, &query, Some(mark_font_found), found_ptr as LPARAM, 0);
    ReleaseDC(hwnd, hdc);

    found
}

unsafe extern "system" fn mark_font_found(
    _logfont: *const LOGFONTW,
    _textmetric: *const TEXTMETRICW,
    _font_type: u32,
    lparam: LPARAM,
) -> i32 {
    let found = (lparam as *mut bool).as_mut();
    if let Some(found) = found {
        *found = true;
    }
    0
}

fn logfont_from_settings(hwnd: HWND, settings: &EditorFontSettings) -> LOGFONTW {
    // SAFETY: hwnd is only used to query display DPI; dpi_y_for_window falls back to 96 DPI if a
    // device context cannot be acquired.
    let mut logfont = LOGFONTW {
        lfHeight: choose_font_logical_height_from_point_size(settings.size_pt, unsafe {
            dpi_y_for_window(hwnd)
        }),
        lfWeight: FW_NORMAL as i32,
        lfCharSet: DEFAULT_CHARSET,
        lfOutPrecision: OUT_DEFAULT_PRECIS,
        lfClipPrecision: CLIP_DEFAULT_PRECIS,
        lfQuality: CLEARTYPE_QUALITY,
        lfPitchAndFamily: DEFAULT_PITCH | FF_DONTCARE,
        ..Default::default()
    };
    copy_face_name(&mut logfont.lfFaceName, &settings.family);
    logfont
}

fn logical_height_from_point_size(size_pt: i32, dpi_y: i32) -> i32 {
    let clamped_size = size_pt.clamp(MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT);
    let dpi_y = dpi_y.max(1);
    -(((i64::from(clamped_size) * i64::from(dpi_y)) + 36) / 72) as i32
}

fn choose_font_logical_height_from_point_size(size_pt: i32, dpi_y: i32) -> i32 {
    let clamped_size = size_pt.clamp(MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT);
    let dpi_y = dpi_y.max(1);
    let pixel_height = ceil_div_positive(i64::from(clamped_size) * i64::from(dpi_y), 72);
    -(pixel_height as i32)
}

fn selected_point_size(hwnd: HWND, point_size_tenths: i32, logical_height: i32) -> i32 {
    if point_size_tenths > 0 {
        return point_size_from_tenths(point_size_tenths);
    }

    // SAFETY: hwnd is only used to query display DPI; dpi_y_for_window falls back to 96 DPI if a
    // device context cannot be acquired.
    let dpi_y = unsafe { dpi_y_for_window(hwnd) }.max(1);
    ((logical_height.abs() * 72 + (dpi_y / 2)) / dpi_y)
        .clamp(MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT)
}

fn point_size_from_tenths(point_size_tenths: i32) -> i32 {
    (point_size_tenths.saturating_add(9) / 10)
        .clamp(MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT)
}

fn ceil_div_positive(numerator: i64, denominator: i64) -> i64 {
    (numerator + denominator - 1) / denominator
}

fn copy_face_name(target: &mut [u16; LF_FACE_SIZE], family: &str) {
    target.fill(0);
    for (index, unit) in family.encode_utf16().take(LF_FACE_SIZE - 1).enumerate() {
        target[index] = unit;
    }
}

fn wide_fixed_to_string(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_font_height_keeps_integer_point_size_visible_at_common_dpi() {
        for dpi_y in [96, 120, 144, 192] {
            for size_pt in MIN_EDITOR_FONT_SIZE_PT..=MAX_EDITOR_FONT_SIZE_PT {
                let height = choose_font_logical_height_from_point_size(size_pt, dpi_y);
                assert_eq!(dialog_floor_point_size(height, dpi_y), size_pt);
            }
        }
    }

    #[test]
    fn choose_font_height_keeps_ten_points_from_falling_to_nine_at_96_dpi() {
        assert_eq!(logical_height_from_point_size(10, 96), -13);
        assert_eq!(dialog_floor_point_size(-13, 96), 9);

        let dialog_height = choose_font_logical_height_from_point_size(10, 96);
        assert_eq!(dialog_height, -14);
        assert_eq!(dialog_floor_point_size(dialog_height, 96), 10);
    }

    #[test]
    fn selected_point_size_ceil_handles_dialog_tenths_below_integer() {
        assert_eq!(point_size_from_tenths(90), 9);
        assert_eq!(point_size_from_tenths(91), 10);
        assert_eq!(point_size_from_tenths(99), 10);
        assert_eq!(point_size_from_tenths(100), 10);
    }

    fn dialog_floor_point_size(logical_height: i32, dpi_y: i32) -> i32 {
        ((logical_height.abs() * 72) / dpi_y)
            .clamp(MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT)
    }
}
