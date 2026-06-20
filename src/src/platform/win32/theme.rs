use std::cell::Cell;
use std::ffi::c_void;
use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, GetStockObject, InvalidateRect, SetBkColor,
    SetBkMode, SetDCBrushColor, SetTextColor, DC_BRUSH, HBRUSH, HDC, HGDIOBJ,
};
use windows_sys::Win32::UI::Controls::{
    SetWindowTheme, CLR_DEFAULT, TVM_SETBKCOLOR, TVM_SETLINECOLOR, TVM_SETTEXTCOLOR,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, SendMessageW};

use super::common::{
    last_win32_error, CFE_AUTOCOLOR_RICH_EDIT, CFM_COLOR_RICH_EDIT, EM_SETBKGNDCOLOR_RICH_EDIT,
    EM_SETCHARFORMAT_RICH_EDIT, SCF_ALL_RICH_EDIT, SCF_DEFAULT_RICH_EDIT,
};
use super::state::WindowState;
use super::text::{ProgrammaticTextUpdateGuard, RichEditHighlightColors};
use crate::domain::AppearanceTheme;
use crate::error::AppError;

const DARK_MODE_EXPLORER_THEME: [u16; 18] = [
    'D' as u16, 'a' as u16, 'r' as u16, 'k' as u16, 'M' as u16, 'o' as u16, 'd' as u16, 'e' as u16,
    '_' as u16, 'E' as u16, 'x' as u16, 'p' as u16, 'l' as u16, 'o' as u16, 'r' as u16, 'e' as u16,
    'r' as u16, 0,
];
const GDI_OPAQUE_BACKGROUND_MODE: i32 = 2;
const TREE_VIEW_USE_SYSTEM_COLOR: LPARAM = -1;
const RICH_EDIT_USE_SYSTEM_BACKGROUND: WPARAM = 1;
const RICH_EDIT_USE_EXPLICIT_BACKGROUND: WPARAM = 0;
const RICH_EDIT_FACE_NAME_UNITS: usize = 32;

thread_local! {
    static ACTIVE_THEME: Cell<AppearanceTheme> = const { Cell::new(AppearanceTheme::Light) };
}

#[derive(Clone, Copy)]
struct ThemePalette {
    window_background: COLORREF,
    control_background: COLORREF,
    control_text: COLORREF,
    tree_line: COLORREF,
    find_match_background: COLORREF,
    custom_controls: bool,
}

impl ThemePalette {
    fn for_theme(theme: AppearanceTheme) -> Self {
        match theme {
            AppearanceTheme::Light => Self {
                window_background: rgb(240, 240, 240),
                control_background: rgb(255, 255, 255),
                control_text: rgb(0, 0, 0),
                tree_line: rgb(160, 160, 160),
                find_match_background: rgb(255, 230, 128),
                custom_controls: false,
            },
            AppearanceTheme::ClassicDark => Self {
                window_background: rgb(31, 33, 36),
                control_background: rgb(24, 26, 29),
                control_text: rgb(230, 232, 235),
                tree_line: rgb(92, 97, 105),
                find_match_background: rgb(91, 74, 25),
                custom_controls: true,
            },
            AppearanceTheme::SepiaTeal => Self {
                window_background: rgb(24, 25, 24),
                control_background: rgb(31, 52, 56),
                control_text: rgb(236, 232, 219),
                tree_line: rgb(178, 154, 124),
                find_match_background: rgb(83, 82, 31),
                custom_controls: true,
            },
            AppearanceTheme::Graphite => Self {
                window_background: rgb(24, 25, 26),
                control_background: rgb(50, 55, 63),
                control_text: rgb(239, 236, 229),
                tree_line: rgb(126, 119, 105),
                find_match_background: rgb(89, 80, 31),
                custom_controls: true,
            },
            AppearanceTheme::Forest => Self {
                window_background: rgb(22, 25, 23),
                control_background: rgb(39, 59, 63),
                control_text: rgb(236, 239, 229),
                tree_line: rgb(104, 150, 117),
                find_match_background: rgb(62, 88, 42),
                custom_controls: true,
            },
            AppearanceTheme::SteelBlue => Self {
                window_background: rgb(24, 25, 27),
                control_background: rgb(54, 64, 80),
                control_text: rgb(239, 240, 242),
                tree_line: rgb(104, 139, 171),
                find_match_background: rgb(66, 88, 113),
                custom_controls: true,
            },
        }
    }

    fn uses_custom_controls(self) -> bool {
        self.custom_controls
    }
}

pub(super) struct ThemeResources {
    window_brush: GdiBrush,
    control_brush: GdiBrush,
}

impl ThemeResources {
    pub(super) fn new(theme: AppearanceTheme) -> Result<Self, AppError> {
        let palette = ThemePalette::for_theme(theme);
        Ok(Self {
            window_brush: GdiBrush::new(palette.window_background)?,
            control_brush: GdiBrush::new(palette.control_background)?,
        })
    }

    fn window_brush(&self) -> HBRUSH {
        self.window_brush.handle()
    }

    fn control_brush(&self) -> HBRUSH {
        self.control_brush.handle()
    }
}

struct GdiBrush {
    handle: HBRUSH,
}

impl GdiBrush {
    fn new(color: COLORREF) -> Result<Self, AppError> {
        let handle = unsafe { CreateSolidBrush(color) };
        if handle.is_null() {
            return Err(last_win32_error("create theme brush"));
        }

        Ok(Self { handle })
    }

    fn handle(&self) -> HBRUSH {
        self.handle
    }
}

impl Drop for GdiBrush {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                DeleteObject(self.handle as HGDIOBJ);
            }
        }
    }
}

pub(super) unsafe fn apply_window_theme(hwnd: HWND, state: &mut WindowState) {
    let theme = state.app.ui_settings().appearance.theme;
    let palette = ThemePalette::for_theme(theme);
    let dark_mode = theme.uses_dark_mode();

    remember_active_theme(theme);
    apply_title_bar_theme(hwnd, dark_mode);
    set_preferred_control_theme(state.search, dark_mode);
    set_preferred_control_theme(state.tree, dark_mode);
    set_preferred_control_theme(state.tab_bar, dark_mode);
    set_preferred_control_theme(state.editor, dark_mode);
    set_preferred_control_theme(state.caret_status, dark_mode);
    apply_tree_theme(state.tree, palette);
    {
        let _guard = ProgrammaticTextUpdateGuard::enter(&state.suppress_editor_change);
        apply_rich_edit_theme(state.editor, palette);
    }
    invalidate(hwnd);
    invalidate(state.search);
    invalidate(state.tree);
    invalidate(state.tab_bar);
    invalidate(state.editor);
    invalidate(state.caret_status);
}

pub(super) fn rich_edit_find_highlight_colors(theme: AppearanceTheme) -> RichEditHighlightColors {
    let palette = ThemePalette::for_theme(theme);
    RichEditHighlightColors {
        background: palette.find_match_background,
    }
}

pub(super) unsafe fn erase_window_background(
    hwnd: HWND,
    state: &WindowState,
    wparam: WPARAM,
) -> Option<LRESULT> {
    let palette = ThemePalette::for_theme(state.app.ui_settings().appearance.theme);
    if !palette.uses_custom_controls() {
        return None;
    }

    let hdc = wparam as HDC;
    if hdc.is_null() {
        return None;
    }

    let mut rect = Default::default();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return None;
    }

    FillRect(hdc, &rect, state.theme_resources.window_brush());
    Some(1)
}

pub(super) unsafe fn erase_window_background_for_active_theme(
    hwnd: HWND,
    wparam: WPARAM,
) -> Option<LRESULT> {
    let palette = ThemePalette::for_theme(active_theme());
    if !palette.uses_custom_controls() {
        return None;
    }

    let hdc = wparam as HDC;
    if hdc.is_null() {
        return None;
    }

    let mut rect = Default::default();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return None;
    }

    let brush = dc_brush(hdc, palette.window_background)?;
    FillRect(hdc, &rect, brush);
    Some(1)
}

pub(super) unsafe fn control_color_brush(state: &WindowState, wparam: WPARAM) -> Option<LRESULT> {
    let palette = ThemePalette::for_theme(state.app.ui_settings().appearance.theme);
    if !palette.uses_custom_controls() {
        return None;
    }

    let hdc = wparam as HDC;
    if hdc.is_null() {
        return None;
    }

    SetTextColor(hdc, palette.control_text);
    SetBkColor(hdc, palette.control_background);
    SetBkMode(hdc, GDI_OPAQUE_BACKGROUND_MODE);
    Some(state.theme_resources.control_brush() as LRESULT)
}

pub(super) unsafe fn control_color_brush_for_active_theme(wparam: WPARAM) -> Option<LRESULT> {
    let palette = ThemePalette::for_theme(active_theme());
    if !palette.uses_custom_controls() {
        return None;
    }

    let hdc = wparam as HDC;
    if hdc.is_null() {
        return None;
    }

    SetTextColor(hdc, palette.control_text);
    SetBkColor(hdc, palette.control_background);
    SetBkMode(hdc, GDI_OPAQUE_BACKGROUND_MODE);
    Some(dc_brush(hdc, palette.control_background)? as LRESULT)
}

fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)
}

fn remember_active_theme(theme: AppearanceTheme) {
    ACTIVE_THEME.with(|active| active.set(theme));
}

fn active_theme() -> AppearanceTheme {
    ACTIVE_THEME.with(Cell::get)
}

unsafe fn dc_brush(hdc: HDC, color: COLORREF) -> Option<HBRUSH> {
    SetDCBrushColor(hdc, color);
    let brush = GetStockObject(DC_BRUSH);
    if brush.is_null() {
        None
    } else {
        Some(brush as HBRUSH)
    }
}

unsafe fn apply_title_bar_theme(hwnd: HWND, dark_theme: bool) {
    if hwnd.is_null() {
        return;
    }

    let enabled: i32 = if dark_theme { 1 } else { 0 };
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
        &enabled as *const i32 as *const c_void,
        mem::size_of::<i32>() as u32,
    );
}

unsafe fn set_preferred_control_theme(hwnd: HWND, dark_theme: bool) {
    if hwnd.is_null() {
        return;
    }

    let theme_name = if dark_theme {
        DARK_MODE_EXPLORER_THEME.as_ptr()
    } else {
        ptr::null()
    };
    let _ = SetWindowTheme(hwnd, theme_name, ptr::null());
}

unsafe fn apply_tree_theme(tree: HWND, palette: ThemePalette) {
    if tree.is_null() {
        return;
    }

    let colors = TreeViewThemeColors::for_theme(palette);
    SendMessageW(tree, TVM_SETBKCOLOR, 0, colors.background);
    SendMessageW(tree, TVM_SETTEXTCOLOR, 0, colors.text);
    SendMessageW(tree, TVM_SETLINECOLOR, 0, colors.line);
}

unsafe fn apply_rich_edit_theme(editor: HWND, palette: ThemePalette) {
    if editor.is_null() {
        return;
    }

    let colors = RichEditThemeColors::for_theme(palette);
    SendMessageW(
        editor,
        EM_SETBKGNDCOLOR_RICH_EDIT,
        colors.background_mode,
        colors.background,
    );

    let format = RichEditCharFormatW::text_color(colors.text);
    let format_ptr = &format as *const RichEditCharFormatW as LPARAM;
    SendMessageW(
        editor,
        EM_SETCHARFORMAT_RICH_EDIT,
        SCF_DEFAULT_RICH_EDIT,
        format_ptr,
    );
    SendMessageW(
        editor,
        EM_SETCHARFORMAT_RICH_EDIT,
        SCF_ALL_RICH_EDIT,
        format_ptr,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TreeViewThemeColors {
    background: LPARAM,
    text: LPARAM,
    line: LPARAM,
}

impl TreeViewThemeColors {
    fn for_theme(palette: ThemePalette) -> Self {
        if palette.uses_custom_controls() {
            Self {
                background: palette.control_background as LPARAM,
                text: palette.control_text as LPARAM,
                line: palette.tree_line as LPARAM,
            }
        } else {
            Self {
                background: TREE_VIEW_USE_SYSTEM_COLOR,
                text: TREE_VIEW_USE_SYSTEM_COLOR,
                line: CLR_DEFAULT as LPARAM,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RichEditThemeColors {
    background_mode: WPARAM,
    background: LPARAM,
    text: RichEditTextColor,
}

impl RichEditThemeColors {
    fn for_theme(palette: ThemePalette) -> Self {
        if palette.uses_custom_controls() {
            Self {
                background_mode: RICH_EDIT_USE_EXPLICIT_BACKGROUND,
                background: palette.control_background as LPARAM,
                text: RichEditTextColor::Explicit(palette.control_text),
            }
        } else {
            Self {
                background_mode: RICH_EDIT_USE_SYSTEM_BACKGROUND,
                background: 0,
                text: RichEditTextColor::Auto,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RichEditTextColor {
    Auto,
    Explicit(COLORREF),
}

#[repr(C)]
struct RichEditCharFormatW {
    cb_size: u32,
    mask: u32,
    effects: u32,
    height: i32,
    offset: i32,
    text_color: COLORREF,
    char_set: u8,
    pitch_and_family: u8,
    face_name: [u16; RICH_EDIT_FACE_NAME_UNITS],
}

impl RichEditCharFormatW {
    fn text_color(color: RichEditTextColor) -> Self {
        let (effects, text_color) = match color {
            RichEditTextColor::Auto => (CFE_AUTOCOLOR_RICH_EDIT, 0),
            RichEditTextColor::Explicit(color) => (0, color),
        };

        Self {
            cb_size: mem::size_of::<Self>() as u32,
            mask: CFM_COLOR_RICH_EDIT,
            effects,
            height: 0,
            offset: 0,
            text_color,
            char_set: 0,
            pitch_and_family: 0,
            face_name: [0; RICH_EDIT_FACE_NAME_UNITS],
        }
    }
}

unsafe fn invalidate(hwnd: HWND) {
    if !hwnd.is_null() {
        InvalidateRect(hwnd, ptr::null(), TRUE);
    }
}

#[cfg(test)]
mod tests {
    use super::super::common::{
        DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME, EMPTY_TEXT, EM_SETTEXTMODE_RICH_EDIT,
        RICH_EDIT_MODULE_NAME, TM_PLAINTEXT_RICH_EDIT,
    };
    use super::super::test_support::{enter_win32_control_test, Win32ControlTestGuard};
    use super::super::text::utf8_to_wide_null;
    use super::*;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, ES_MULTILINE, WS_OVERLAPPED,
    };

    const EM_GETCHARFORMAT_RICH_EDIT: u32 = 0x043A;

    struct TestRichEdit(HWND, #[allow(dead_code)] Win32ControlTestGuard);

    impl TestRichEdit {
        unsafe fn create() -> Self {
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
                120,
                80,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if hwnd.is_null() {
                panic!("test Rich Edit control should be created");
            }

            let text_mode_result =
                SendMessageW(hwnd, EM_SETTEXTMODE_RICH_EDIT, TM_PLAINTEXT_RICH_EDIT, 0);
            if text_mode_result != 0 {
                DestroyWindow(hwnd);
                panic!("test Rich Edit control should accept plain text mode");
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

    unsafe fn rich_edit_default_text_color(editor: HWND) -> RichEditTextColor {
        let mut format = RichEditCharFormatW::text_color(RichEditTextColor::Auto);
        let mask = SendMessageW(
            editor,
            EM_GETCHARFORMAT_RICH_EDIT,
            SCF_DEFAULT_RICH_EDIT,
            &mut format as *mut RichEditCharFormatW as LPARAM,
        ) as u32;

        assert_eq!(mask & CFM_COLOR_RICH_EDIT, CFM_COLOR_RICH_EDIT);
        if format.effects & CFE_AUTOCOLOR_RICH_EDIT != 0 {
            RichEditTextColor::Auto
        } else {
            RichEditTextColor::Explicit(format.text_color)
        }
    }

    #[test]
    fn light_tree_theme_restores_system_text_and_background_colors() {
        let palette = ThemePalette::for_theme(AppearanceTheme::Light);
        let colors = TreeViewThemeColors::for_theme(palette);

        assert_eq!(colors.background, -1);
        assert_eq!(colors.text, -1);
        assert_eq!(colors.line, CLR_DEFAULT as LPARAM);
    }

    #[test]
    fn custom_tree_themes_use_explicit_palette_colors() {
        for theme in AppearanceTheme::options()
            .iter()
            .copied()
            .filter(|theme| theme.uses_dark_mode())
        {
            let palette = ThemePalette::for_theme(theme);
            let colors = TreeViewThemeColors::for_theme(palette);

            assert_eq!(colors.background, palette.control_background as LPARAM);
            assert_eq!(colors.text, palette.control_text as LPARAM);
            assert_eq!(colors.line, palette.tree_line as LPARAM);
        }
    }

    #[test]
    fn light_rich_edit_theme_restores_system_background_and_text_colors() {
        let palette = ThemePalette::for_theme(AppearanceTheme::Light);
        let colors = RichEditThemeColors::for_theme(palette);

        assert_eq!(colors.background_mode, RICH_EDIT_USE_SYSTEM_BACKGROUND);
        assert_eq!(colors.background, 0);
        assert_eq!(colors.text, RichEditTextColor::Auto);
    }

    #[test]
    fn custom_rich_edit_themes_use_explicit_palette_colors() {
        for theme in AppearanceTheme::options()
            .iter()
            .copied()
            .filter(|theme| theme.uses_dark_mode())
        {
            let palette = ThemePalette::for_theme(theme);
            let colors = RichEditThemeColors::for_theme(palette);

            assert_eq!(colors.background_mode, RICH_EDIT_USE_EXPLICIT_BACKGROUND);
            assert_eq!(colors.background, palette.control_background as LPARAM);
            assert_eq!(
                colors.text,
                RichEditTextColor::Explicit(palette.control_text)
            );
        }
    }

    #[test]
    fn rich_edit_text_color_format_sets_only_color_fields() {
        let format = RichEditCharFormatW::text_color(RichEditTextColor::Explicit(rgb(1, 2, 3)));

        assert_eq!(format.cb_size, mem::size_of::<RichEditCharFormatW>() as u32);
        assert_eq!(format.mask, CFM_COLOR_RICH_EDIT);
        assert_eq!(format.effects, 0);
        assert_eq!(format.text_color, rgb(1, 2, 3));

        let auto = RichEditCharFormatW::text_color(RichEditTextColor::Auto);
        assert_eq!(auto.mask, CFM_COLOR_RICH_EDIT);
        assert_eq!(auto.effects, CFE_AUTOCOLOR_RICH_EDIT);
    }

    #[test]
    fn find_match_highlight_colors_stay_readable_for_each_theme() {
        for theme in AppearanceTheme::options().iter().copied() {
            let palette = ThemePalette::for_theme(theme);
            let colors = rich_edit_find_highlight_colors(theme);

            assert_eq!(colors.background, palette.find_match_background);
            assert!(
                contrast_ratio(palette.control_text, colors.background) >= 4.5,
                "find highlight text contrast should be readable for {theme:?}"
            );
            assert!(
                color_distance(palette.control_background, colors.background) >= 40,
                "find highlight should stand apart from editor background for {theme:?}"
            );
        }
    }

    #[test]
    fn applying_rich_edit_theme_changes_actual_background_and_text_color() {
        unsafe {
            let editor = TestRichEdit::create();
            let palette = ThemePalette::for_theme(AppearanceTheme::Graphite);

            apply_rich_edit_theme(editor.0, palette);

            assert_eq!(
                rich_edit_default_text_color(editor.0),
                RichEditTextColor::Explicit(palette.control_text)
            );
            let previous_background = SendMessageW(
                editor.0,
                EM_SETBKGNDCOLOR_RICH_EDIT,
                RICH_EDIT_USE_SYSTEM_BACKGROUND,
                0,
            );
            assert_eq!(previous_background as COLORREF, palette.control_background);

            apply_rich_edit_theme(editor.0, ThemePalette::for_theme(AppearanceTheme::Light));
            assert_eq!(
                rich_edit_default_text_color(editor.0),
                RichEditTextColor::Auto
            );
        }
    }

    fn color_distance(left: COLORREF, right: COLORREF) -> u32 {
        let (left_red, left_green, left_blue) = color_channels(left);
        let (right_red, right_green, right_blue) = color_channels(right);
        u32::from(left_red.abs_diff(right_red))
            + u32::from(left_green.abs_diff(right_green))
            + u32::from(left_blue.abs_diff(right_blue))
    }

    fn contrast_ratio(left: COLORREF, right: COLORREF) -> f64 {
        let left = relative_luminance(left);
        let right = relative_luminance(right);
        let lighter = left.max(right);
        let darker = left.min(right);
        (lighter + 0.05) / (darker + 0.05)
    }

    fn relative_luminance(color: COLORREF) -> f64 {
        let (red, green, blue) = color_channels(color);
        0.2126 * srgb_channel_luminance(red)
            + 0.7152 * srgb_channel_luminance(green)
            + 0.0722 * srgb_channel_luminance(blue)
    }

    fn srgb_channel_luminance(channel: u8) -> f64 {
        let value = f64::from(channel) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn color_channels(color: COLORREF) -> (u8, u8, u8) {
        (
            (color & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            ((color >> 16) & 0xff) as u8,
        )
    }
}
