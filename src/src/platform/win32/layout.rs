use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, LoadCursorW, MoveWindow, SetCursor, SetWindowPos, CW_USEDEFAULT,
    IDC_SIZEWE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

use super::common::{
    last_win32_error, CARET_STATUS_HEIGHT, CARET_STATUS_HORIZONTAL_PADDING, MIN_EDITOR_WIDTH,
    MIN_SPLIT_WIDTH, SEARCH_BOX_HEIGHT, SEARCH_PANEL_PADDING, SPLITTER_WIDTH, TAB_BAR_HEIGHT,
};
use super::dpi::UiScale;
use super::state::{TreeMode, WindowState};
use crate::domain::{SelectionSettings, SplitterSettings, WindowSettings};
use crate::error::AppError;
pub(super) struct InitialWindowPlacement {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

pub(super) unsafe fn initial_window_placement(settings: WindowSettings) -> InitialWindowPlacement {
    let width = settings.width;
    let height = settings.height;

    if let (Some(x), Some(y)) = (settings.x, settings.y) {
        if let (Some(right), Some(bottom)) = (x.checked_add(width), y.checked_add(height)) {
            let rect = RECT {
                left: x,
                top: y,
                right,
                bottom,
            };
            if window_rect_intersects_monitor_work_area(&rect) {
                return InitialWindowPlacement {
                    x,
                    y,
                    width,
                    height,
                };
            }
        }
    }

    InitialWindowPlacement {
        x: CW_USEDEFAULT,
        y: CW_USEDEFAULT,
        width,
        height,
    }
}

unsafe fn window_rect_intersects_monitor_work_area(rect: &RECT) -> bool {
    let monitor = MonitorFromRect(rect, MONITOR_DEFAULTTONULL);
    if monitor.is_null() {
        return false;
    }

    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    if GetMonitorInfoW(monitor, &mut monitor_info) == 0 {
        return false;
    }

    rects_intersect(rect, &monitor_info.rcWork)
}

fn rects_intersect(left: &RECT, right: &RECT) -> bool {
    left.left < right.right
        && left.right > right.left
        && left.top < right.bottom
        && left.bottom > right.top
}

pub(super) unsafe fn layout_children(parent: HWND, state: &WindowState) {
    if state.search.is_null()
        || state.tree.is_null()
        || state.tab_bar.is_null()
        || state.editor.is_null()
        || state.caret_status.is_null()
    {
        return;
    }

    let mut rect = RECT::default();
    if GetClientRect(parent, &mut rect) == 0 {
        return;
    }

    let client_width = (rect.right - rect.left).max(0);
    let client_height = (rect.bottom - rect.top).max(0);
    let scale = state.ui_scale;
    let splitter_width = scale.px(SPLITTER_WIDTH);
    let search_box_height = scale.px(SEARCH_BOX_HEIGHT);
    let search_panel_padding = scale.px(SEARCH_PANEL_PADDING);
    let tree_width = clamp_split_width(state.split_width, client_width, scale);
    let editor_x = (tree_width + splitter_width).min(client_width);
    let editor_width = client_width - editor_x;
    let tab_height = scale.px(TAB_BAR_HEIGHT).min(client_height);
    let caret_status_height = scale
        .px(CARET_STATUS_HEIGHT)
        .min(client_height.saturating_sub(tab_height));
    let editor_height = client_height
        .saturating_sub(tab_height)
        .saturating_sub(caret_status_height);
    let caret_status_padding = scale.px(CARET_STATUS_HORIZONTAL_PADDING).min(editor_width);
    let search_x = search_panel_padding.min(tree_width);
    let search_y = search_panel_padding.min(client_height);
    let search_width = (tree_width - search_panel_padding * 2).max(0);
    let tree_y = tree_top_offset(state).min(client_height);
    let tree_height = client_height - tree_y;

    MoveWindow(
        state.search,
        search_x,
        search_y,
        search_width,
        search_box_height.min(client_height),
        1,
    );
    MoveWindow(state.tree, 0, tree_y, tree_width, tree_height, 1);
    MoveWindow(state.tab_bar, editor_x, 0, editor_width, tab_height, 1);
    MoveWindow(
        state.editor,
        editor_x,
        tab_height,
        editor_width,
        editor_height,
        1,
    );
    MoveWindow(
        state.caret_status,
        editor_x + caret_status_padding,
        tab_height + editor_height,
        (editor_width - caret_status_padding * 2).max(0),
        caret_status_height,
        1,
    );
}

pub(super) unsafe fn center_window_over_owner(window: HWND, owner: HWND) -> Result<(), AppError> {
    let mut owner_rect = RECT::default();
    if GetWindowRect(owner, &mut owner_rect) == 0 {
        return Err(last_win32_error("read owner window bounds"));
    }

    let mut window_rect = RECT::default();
    if GetWindowRect(window, &mut window_rect) == 0 {
        return Err(last_win32_error("read child window bounds"));
    }

    let (x, y) = match monitor_work_area_for_rect(&owner_rect) {
        Some(work_area) => centered_window_position_within(&owner_rect, &window_rect, &work_area),
        None => centered_window_position(&owner_rect, &window_rect),
    };

    if SetWindowPos(
        window,
        ptr::null_mut(),
        x,
        y,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    ) == 0
    {
        return Err(last_win32_error("center child window"));
    }

    Ok(())
}

unsafe fn monitor_work_area_for_rect(rect: &RECT) -> Option<RECT> {
    let monitor = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
    if monitor.is_null() {
        return None;
    }

    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    if GetMonitorInfoW(monitor, &mut monitor_info) == 0 {
        return None;
    }

    Some(monitor_info.rcWork)
}

fn centered_window_position(owner: &RECT, window: &RECT) -> (i32, i32) {
    let width = rect_width(window);
    let height = rect_height(window);

    (
        centered_coordinate(owner.left, owner.right, width),
        centered_coordinate(owner.top, owner.bottom, height),
    )
}

fn centered_window_position_within(owner: &RECT, window: &RECT, bounds: &RECT) -> (i32, i32) {
    let width = rect_width(window);
    let height = rect_height(window);

    (
        clamp_coordinate(
            centered_coordinate(owner.left, owner.right, width),
            width,
            bounds.left,
            bounds.right,
        ),
        clamp_coordinate(
            centered_coordinate(owner.top, owner.bottom, height),
            height,
            bounds.top,
            bounds.bottom,
        ),
    )
}

fn rect_width(rect: &RECT) -> i32 {
    (rect.right - rect.left).max(0)
}

fn rect_height(rect: &RECT) -> i32 {
    (rect.bottom - rect.top).max(0)
}

fn centered_coordinate(owner_start: i32, owner_end: i32, child_size: i32) -> i32 {
    let owner_size = i64::from(owner_end) - i64::from(owner_start);
    let centered = i64::from(owner_start) + (owner_size - i64::from(child_size)) / 2;

    clamp_i64_to_i32(centered)
}

fn clamp_coordinate(position: i32, child_size: i32, bounds_start: i32, bounds_end: i32) -> i32 {
    let min_position = i64::from(bounds_start);
    let max_position = i64::from(bounds_end) - i64::from(child_size);
    let position = i64::from(position);

    if max_position < min_position {
        return bounds_start;
    }

    clamp_i64_to_i32(position.clamp(min_position, max_position))
}

fn clamp_i64_to_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub(super) fn tree_top_offset(state: &WindowState) -> i32 {
    if state.search.is_null() {
        0
    } else {
        state.ui_scale.px(SEARCH_BOX_HEIGHT) + state.ui_scale.px(SEARCH_PANEL_PADDING) * 2
    }
}

fn clamp_split_width(left_width: i32, client_width: i32, scale: UiScale) -> i32 {
    if client_width <= 0 {
        return 0;
    }

    let splitter_width = scale.px(SPLITTER_WIDTH);
    let min_editor_width = scale.px(MIN_EDITOR_WIDTH);
    let min_split_width = scale.px(MIN_SPLIT_WIDTH);
    let full_width = (client_width - splitter_width).max(0);
    let max_with_editor = (client_width - splitter_width - min_editor_width).max(0);
    let max_width = if max_with_editor >= min_split_width {
        max_with_editor
    } else {
        full_width
    };
    let min_width = min_split_width.min(max_width);
    left_width.clamp(min_width, max_width)
}

unsafe fn current_window_settings(hwnd: HWND) -> Result<Option<WindowSettings>, AppError> {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return Err(last_win32_error("read window bounds"));
    }

    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return Ok(None);
    }

    Ok(Some(WindowSettings::new(
        rect.left, rect.top, width, height,
    )))
}

pub(super) unsafe fn save_current_ui_settings(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    save_current_ui_settings_with_selection_mode(hwnd, state, UiSettingsSelectionMode::Update)
}

pub(super) unsafe fn save_current_tree_refresh_ui_settings(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    save_current_ui_settings_with_selection_mode(hwnd, state, UiSettingsSelectionMode::Preserve)
}

#[derive(Clone, Copy)]
enum UiSettingsSelectionMode {
    Update,
    Preserve,
}

unsafe fn save_current_ui_settings_with_selection_mode(
    hwnd: HWND,
    state: &mut WindowState,
    selection_mode: UiSettingsSelectionMode,
) -> Result<(), AppError> {
    let current_window = current_window_settings(hwnd)?;
    let splitter = SplitterSettings::new(splitter_width_for_client(hwnd, state));
    let tree_mode = state.tree_mode;
    let selected_node_id = state.selected_node_id;
    let settings = {
        let current_settings = state.app.ui_settings_ref();
        let window = current_window.unwrap_or(current_settings.window);
        let selection = selection_for_ui_settings_mode(
            selection_mode,
            tree_mode,
            selected_node_id,
            current_settings.selection,
        );

        if current_settings.window == window
            && current_settings.splitter == splitter
            && current_settings.selection == selection
        {
            return Ok(());
        }

        let mut settings = current_settings.clone();
        settings.window = window;
        settings.splitter = splitter;
        settings.selection = selection;
        settings
    };

    state.app.save_ui_settings(settings)
}

fn selection_for_ui_settings_mode(
    mode: UiSettingsSelectionMode,
    tree_mode: TreeMode,
    selected_node_id: Option<i64>,
    current_selection: SelectionSettings,
) -> SelectionSettings {
    match mode {
        UiSettingsSelectionMode::Update => {
            selection_for_ui_settings(tree_mode, selected_node_id, current_selection)
        }
        UiSettingsSelectionMode::Preserve => current_selection,
    }
}

pub(super) fn preferred_node_after_search_change(
    selected_node_id: Option<i64>,
    saved_selection: SelectionSettings,
) -> Option<i64> {
    selected_node_id.or(saved_selection.node_id)
}

pub(super) fn selection_for_ui_settings(
    tree_mode: TreeMode,
    selected_node_id: Option<i64>,
    current_selection: SelectionSettings,
) -> SelectionSettings {
    match (tree_mode, selected_node_id) {
        (TreeMode::Active, Some(node_id)) => SelectionSettings {
            node_id: Some(node_id),
        },
        _ => current_selection,
    }
}

pub(super) unsafe fn splitter_width_for_client(hwnd: HWND, state: &WindowState) -> i32 {
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return state.split_width;
    }

    clamp_split_width(
        state.split_width,
        (rect.right - rect.left).max(0),
        state.ui_scale,
    )
}

pub(super) unsafe fn point_is_on_splitter(hwnd: HWND, state: &WindowState, lparam: LPARAM) -> bool {
    let point = client_point_from_lparam(lparam);
    let split_width = splitter_width_for_client(hwnd, state);
    point.x >= split_width
        && point.x < split_width + state.ui_scale.px(SPLITTER_WIDTH)
        && point.y >= 0
}

pub(super) unsafe fn set_splitter_cursor() {
    let cursor = LoadCursorW(ptr::null_mut(), IDC_SIZEWE);
    if !cursor.is_null() {
        SetCursor(cursor);
    }
}

pub(super) unsafe fn begin_splitter_drag(
    hwnd: HWND,
    state: &mut WindowState,
    lparam: LPARAM,
) -> bool {
    if !point_is_on_splitter(hwnd, state, lparam) {
        return false;
    }

    state.dragging_splitter = true;
    set_splitter_cursor();
    SetCapture(hwnd);
    true
}

pub(super) unsafe fn update_splitter_drag(hwnd: HWND, state: &mut WindowState, lparam: LPARAM) {
    let point = client_point_from_lparam(lparam);
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return;
    }

    let client_width = (rect.right - rect.left).max(0);
    state.split_width = clamp_split_width(point.x, client_width, state.ui_scale);
    layout_children(hwnd, state);
}

pub(super) unsafe fn finish_splitter_drag(hwnd: HWND, state: &mut WindowState, lparam: LPARAM) {
    update_splitter_drag(hwnd, state, lparam);
    state.dragging_splitter = false;
    ReleaseCapture();
}

pub(super) fn client_point_from_lparam(lparam: LPARAM) -> POINT {
    let value = lparam as u32;
    POINT {
        x: (value & 0xffff) as i16 as i32,
        y: ((value >> 16) & 0xffff) as i16 as i32,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DEFAULT_DOCUMENT_ID;

    #[test]
    fn selection_settings_preserve_previous_value_when_active_selection_is_empty() {
        let current = SelectionSettings {
            node_id: Some(DEFAULT_DOCUMENT_ID),
        };

        assert_eq!(
            selection_for_ui_settings(TreeMode::Active, None, current),
            current
        );
    }

    #[test]
    fn selection_settings_update_from_active_selection_only() {
        let current = SelectionSettings {
            node_id: Some(DEFAULT_DOCUMENT_ID),
        };

        assert_eq!(
            selection_for_ui_settings(TreeMode::Active, Some(42), current),
            SelectionSettings { node_id: Some(42) }
        );
        assert_eq!(
            selection_for_ui_settings(TreeMode::Trash, Some(42), current),
            current
        );
    }

    #[test]
    fn tree_refresh_settings_preserve_saved_selection() {
        let current = SelectionSettings {
            node_id: Some(DEFAULT_DOCUMENT_ID),
        };

        assert_eq!(
            selection_for_ui_settings_mode(
                UiSettingsSelectionMode::Preserve,
                TreeMode::Active,
                Some(42),
                current
            ),
            current
        );
    }

    #[test]
    fn search_refresh_prefers_current_selection_then_saved_selection() {
        let saved = SelectionSettings {
            node_id: Some(DEFAULT_DOCUMENT_ID),
        };

        assert_eq!(
            preferred_node_after_search_change(Some(42), saved),
            Some(42)
        );
        assert_eq!(
            preferred_node_after_search_change(None, saved),
            Some(DEFAULT_DOCUMENT_ID)
        );
    }

    #[test]
    fn centered_window_position_places_child_over_owner_center() {
        let owner = RECT {
            left: 100,
            top: 100,
            right: 500,
            bottom: 300,
        };
        let child = RECT {
            left: 0,
            top: 0,
            right: 200,
            bottom: 100,
        };

        assert_eq!(centered_window_position(&owner, &child), (200, 150));
    }

    #[test]
    fn centered_window_position_is_clamped_to_bounds() {
        let owner = RECT {
            left: 700,
            top: 500,
            right: 900,
            bottom: 700,
        };
        let child = RECT {
            left: 0,
            top: 0,
            right: 300,
            bottom: 200,
        };
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 600,
        };

        assert_eq!(
            centered_window_position_within(&owner, &child, &bounds),
            (500, 400)
        );
    }

    #[test]
    fn centered_window_position_uses_bounds_origin_when_child_exceeds_bounds() {
        let owner = RECT {
            left: 100,
            top: 100,
            right: 300,
            bottom: 300,
        };
        let child = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 900,
        };
        let bounds = RECT {
            left: 20,
            top: 30,
            right: 820,
            bottom: 630,
        };

        assert_eq!(
            centered_window_position_within(&owner, &child, &bounds),
            (20, 30)
        );
    }
}
