use std::ptr;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::UI::Controls::{
    TCHITTESTINFO, TCIF_TEXT, TCITEMW, TCM_DELETEALLITEMS, TCM_GETCURSEL, TCM_GETITEMCOUNT,
    TCM_GETITEMRECT, TCM_HITTEST, TCM_INSERTITEMW, TCM_SETCURSEL, TCM_SETITEMW, WC_TABCONTROLW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, LoadCursorW, SendMessageW, SetCursor, HMENU, IDC_HAND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WS_CHILD, WS_CLIPSIBLINGS, WS_TABSTOP, WS_VISIBLE,
};

use super::common::{
    last_win32_error, CONTROL_TAB_ID, EMPTY_TEXT, WM_APP_CLOSE_TAB, WM_APP_MOVE_TAB,
};
use super::text::utf8_to_wide_null;
use crate::domain::OpenTabs;
use crate::error::AppError;

const TAB_CONTROL_SUBCLASS_ID: usize = 1;
const TAB_CLOSE_MARKER: &str = "  x";
const TAB_CLOSE_HIT_WIDTH: i32 = 22;
const TAB_CLOSE_HIT_RIGHT_PADDING: i32 = 2;
const TAB_DRAG_THRESHOLD_PX: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabRefreshMode {
    UpdateInPlace,
    Rebuild,
}

struct TabControlSubclassState {
    parent: HWND,
    drag: Option<TabDragState>,
}

struct TabDragState {
    current_index: usize,
    start_point: POINT,
    dragging: bool,
}

pub(super) unsafe fn create_tab_control(
    parent: HWND,
    instance: HINSTANCE,
) -> Result<HWND, AppError> {
    let hwnd = CreateWindowExW(
        0,
        WC_TABCONTROLW,
        EMPTY_TEXT.as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS,
        0,
        0,
        0,
        0,
        parent,
        CONTROL_TAB_ID as HMENU,
        instance,
        ptr::null(),
    );

    if hwnd.is_null() {
        return Err(last_win32_error("create tab control"));
    }

    subclass_tab_control(hwnd, parent)?;

    Ok(hwnd)
}

pub(super) unsafe fn refresh_tab_control(
    tab_control: HWND,
    tabs: &OpenTabs,
    suppress_tab_change: &mut bool,
) -> Result<(), AppError> {
    if tab_control.is_null() {
        return Ok(());
    }

    *suppress_tab_change = true;
    let result = refresh_tab_control_inner(tab_control, tabs);
    *suppress_tab_change = false;
    result
}

unsafe fn refresh_tab_control_inner(tab_control: HWND, tabs: &OpenTabs) -> Result<(), AppError> {
    match tab_refresh_mode(current_tab_count(tab_control), tabs.tabs().len()) {
        TabRefreshMode::UpdateInPlace => update_existing_tab_items(tab_control, tabs)?,
        TabRefreshMode::Rebuild => rebuild_tab_items(tab_control, tabs)?,
    }

    if let Some(active_index) = tabs.active_index() {
        SendMessageW(tab_control, TCM_SETCURSEL, active_index as WPARAM, 0);
    }
    Ok(())
}

unsafe fn current_tab_count(tab_control: HWND) -> Option<usize> {
    let count = SendMessageW(tab_control, TCM_GETITEMCOUNT, 0, 0);
    usize::try_from(count).ok()
}

fn tab_refresh_mode(current_count: Option<usize>, desired_count: usize) -> TabRefreshMode {
    if current_count == Some(desired_count) {
        TabRefreshMode::UpdateInPlace
    } else {
        TabRefreshMode::Rebuild
    }
}

unsafe fn update_existing_tab_items(tab_control: HWND, tabs: &OpenTabs) -> Result<(), AppError> {
    for (index, tab) in tabs.tabs().iter().enumerate() {
        update_tab_item(tab_control, index, &tab.display_title())?;
    }

    Ok(())
}

unsafe fn update_tab_item(
    tab_control: HWND,
    index: usize,
    display_title: &str,
) -> Result<(), AppError> {
    let mut item = tab_text_item(display_title)?;
    let result = SendMessageW(
        tab_control,
        TCM_SETITEMW,
        index as WPARAM,
        &mut item.item as *mut TCITEMW as LPARAM,
    );
    if result == 0 {
        return Err(last_win32_error("update tab item"));
    }

    Ok(())
}

unsafe fn rebuild_tab_items(tab_control: HWND, tabs: &OpenTabs) -> Result<(), AppError> {
    SendMessageW(tab_control, TCM_DELETEALLITEMS, 0, 0);

    for (index, tab) in tabs.tabs().iter().enumerate() {
        let mut item = tab_text_item(&tab.display_title())?;
        let result = SendMessageW(
            tab_control,
            TCM_INSERTITEMW,
            index as WPARAM,
            &mut item.item as *mut TCITEMW as LPARAM,
        );
        if result == -1 {
            return Err(last_win32_error("insert tab item"));
        }
    }

    Ok(())
}

struct TabTextItem {
    item: TCITEMW,
    _title: Vec<u16>,
}

fn tab_text_item(display_title: &str) -> Result<TabTextItem, AppError> {
    let title = tab_control_item_title(display_title);
    let mut title = utf8_to_wide_null("convert tab title from UTF-8 to UTF-16", &title)?;
    let text_len = i32::try_from(title.len())
        .map_err(|_| AppError::platform("prepare tab item", "tab title is too long"))?;
    Ok(TabTextItem {
        item: TCITEMW {
            mask: TCIF_TEXT,
            dwState: 0,
            dwStateMask: 0,
            pszText: title.as_mut_ptr(),
            cchTextMax: text_len,
            iImage: 0,
            lParam: 0,
        },
        _title: title,
    })
}

pub(super) unsafe fn selected_tab_index(tab_control: HWND) -> Option<usize> {
    if tab_control.is_null() {
        return None;
    }

    let result = SendMessageW(tab_control, TCM_GETCURSEL, 0, 0);
    if result < 0 {
        return None;
    }

    usize::try_from(result).ok()
}

unsafe fn subclass_tab_control(tab_control: HWND, parent: HWND) -> Result<(), AppError> {
    let state = Box::new(TabControlSubclassState { parent, drag: None });
    let state_ptr = Box::into_raw(state);
    if SetWindowSubclass(
        tab_control,
        Some(tab_control_subclass_proc),
        TAB_CONTROL_SUBCLASS_ID,
        state_ptr as usize,
    ) == 0
    {
        drop(Box::from_raw(state_ptr));
        return Err(last_win32_error("subclass tab control"));
    }

    Ok(())
}

unsafe extern "system" fn tab_control_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    subclass_data: usize,
) -> isize {
    let state_ptr = subclass_data as *mut TabControlSubclassState;
    if state_ptr.is_null() {
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }

    match message {
        WM_NCDESTROY => {
            (*state_ptr).drag = None;
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            RemoveWindowSubclass(
                hwnd,
                Some(tab_control_subclass_proc),
                TAB_CONTROL_SUBCLASS_ID,
            );
            // SAFETY: state_ptr was created with Box::into_raw for this subclass and is dropped
            // once when the tab control is destroyed.
            drop(Box::from_raw(state_ptr));
            return result;
        }
        WM_LBUTTONDOWN => {
            if tab_close_hit_index(hwnd, lparam).is_some() {
                return 0;
            }

            if let Some(index) = tab_hit_index(hwnd, point_from_lparam(lparam)) {
                (*state_ptr).drag = Some(TabDragState {
                    current_index: index,
                    start_point: point_from_lparam(lparam),
                    dragging: false,
                });
                SetCapture(hwnd);
            }
        }
        WM_LBUTTONUP => {
            if let Some(drag) = (*state_ptr).drag.take() {
                ReleaseCapture();
                if drag.dragging {
                    return 0;
                }
                return DefSubclassProc(hwnd, message, wparam, lparam);
            }

            if let Some(index) = tab_close_hit_index(hwnd, lparam) {
                let parent = (*state_ptr).parent;
                send_tab_close_message(parent, index);
                return 0;
            }
        }
        WM_MOUSEMOVE => {
            let point = point_from_lparam(lparam);
            let dragging = {
                let state = &mut *state_ptr;
                match state.drag.as_mut() {
                    Some(drag) => {
                        if !drag.dragging && tab_drag_threshold_exceeded(drag.start_point, point) {
                            drag.dragging = true;
                        }

                        drag.dragging
                    }
                    None => false,
                }
            };

            if dragging {
                if let Some(target_index) = tab_hit_index(hwnd, point) {
                    let move_request = {
                        let state = &mut *state_ptr;
                        let parent = state.parent;
                        let Some(drag) = state.drag.as_mut() else {
                            return 0;
                        };

                        if target_index != drag.current_index {
                            let from_index = drag.current_index;
                            drag.current_index = target_index;
                            Some((parent, from_index, target_index))
                        } else {
                            None
                        }
                    };

                    if let Some((parent, from_index, target_index)) = move_request {
                        // The parent handler refreshes this same subclassed tab control
                        // synchronously, so no &mut state/drag borrow may span this call.
                        send_tab_move_message(parent, from_index, target_index);
                    }
                }
                return 0;
            }

            if tab_close_hit_index(hwnd, lparam).is_some() {
                let cursor = LoadCursorW(ptr::null_mut(), IDC_HAND);
                if !cursor.is_null() {
                    SetCursor(cursor);
                }
            }
        }
        _ => {}
    }

    DefSubclassProc(hwnd, message, wparam, lparam)
}

unsafe fn tab_close_hit_index(tab_control: HWND, lparam: LPARAM) -> Option<usize> {
    let point = point_from_lparam(lparam);
    let index = tab_hit_index(tab_control, point)?;
    let item_rect = tab_item_rect(tab_control, index)?;
    if point_in_rect(point, close_button_rect(item_rect)) {
        Some(index)
    } else {
        None
    }
}

unsafe fn tab_hit_index(tab_control: HWND, point: POINT) -> Option<usize> {
    let mut hit_test = TCHITTESTINFO {
        pt: point,
        flags: 0,
    };
    let hit = SendMessageW(
        tab_control,
        TCM_HITTEST,
        0,
        &mut hit_test as *mut TCHITTESTINFO as LPARAM,
    );
    if hit < 0 {
        return None;
    }

    usize::try_from(hit).ok()
}

unsafe fn send_tab_close_message(parent: HWND, index: usize) {
    if !parent.is_null() {
        SendMessageW(parent, WM_APP_CLOSE_TAB, index, 0);
    }
}

unsafe fn send_tab_move_message(parent: HWND, from_index: usize, to_index: usize) {
    if !parent.is_null() {
        SendMessageW(parent, WM_APP_MOVE_TAB, from_index, to_index as LPARAM);
    }
}

unsafe fn tab_item_rect(tab_control: HWND, index: usize) -> Option<RECT> {
    let mut rect = RECT::default();
    let result = SendMessageW(
        tab_control,
        TCM_GETITEMRECT,
        index,
        &mut rect as *mut RECT as LPARAM,
    );
    (result != 0).then_some(rect)
}

fn tab_control_item_title(display_title: &str) -> String {
    let mut title = String::with_capacity(display_title.len() + TAB_CLOSE_MARKER.len());
    title.push_str(display_title);
    title.push_str(TAB_CLOSE_MARKER);
    title
}

fn close_button_rect(item_rect: RECT) -> RECT {
    let right = item_rect.right - TAB_CLOSE_HIT_RIGHT_PADDING;
    let left = (right - TAB_CLOSE_HIT_WIDTH).max(item_rect.left);
    RECT {
        left,
        top: item_rect.top,
        right,
        bottom: item_rect.bottom,
    }
}

fn point_in_rect(point: POINT, rect: RECT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn tab_drag_threshold_exceeded(start: POINT, current: POINT) -> bool {
    (current.x - start.x).abs() >= TAB_DRAG_THRESHOLD_PX
        || (current.y - start.y).abs() >= TAB_DRAG_THRESHOLD_PX
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let value = lparam as u32;
    POINT {
        x: (value & 0xffff) as i16 as i32,
        y: ((value >> 16) & 0xffff) as i16 as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_control_item_title_appends_close_marker() {
        assert_eq!(tab_control_item_title("Draft"), "Draft  x");
    }

    #[test]
    fn tab_refresh_updates_in_place_when_tab_count_is_stable() {
        assert_eq!(tab_refresh_mode(Some(2), 2), TabRefreshMode::UpdateInPlace);
        assert_eq!(tab_refresh_mode(Some(0), 0), TabRefreshMode::UpdateInPlace);
        assert_eq!(tab_refresh_mode(Some(1), 2), TabRefreshMode::Rebuild);
        assert_eq!(tab_refresh_mode(None, 2), TabRefreshMode::Rebuild);
    }

    #[test]
    fn close_button_rect_stays_inside_tab_item() {
        let item = RECT {
            left: 10,
            top: 2,
            right: 90,
            bottom: 26,
        };
        let close = close_button_rect(item);

        assert_eq!(close.left, 66);
        assert_eq!(close.top, 2);
        assert_eq!(close.right, 88);
        assert_eq!(close.bottom, 26);
    }

    #[test]
    fn point_in_rect_uses_exclusive_right_and_bottom_edges() {
        let rect = RECT {
            left: 1,
            top: 2,
            right: 4,
            bottom: 5,
        };

        assert!(point_in_rect(POINT { x: 1, y: 2 }, rect));
        assert!(point_in_rect(POINT { x: 3, y: 4 }, rect));
        assert!(!point_in_rect(POINT { x: 4, y: 4 }, rect));
        assert!(!point_in_rect(POINT { x: 3, y: 5 }, rect));
    }

    #[test]
    fn tab_drag_threshold_uses_horizontal_or_vertical_distance() {
        let start = POINT { x: 10, y: 10 };

        assert!(!tab_drag_threshold_exceeded(start, POINT { x: 13, y: 10 }));
        assert!(tab_drag_threshold_exceeded(start, POINT { x: 14, y: 10 }));
        assert!(tab_drag_threshold_exceeded(start, POINT { x: 10, y: 6 }));
    }
}
