use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::{Once, OnceLock};

use windows_sys::Win32::Foundation::{FARPROC, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSY};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExA, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

const DEFAULT_DPI: u32 = 96;
const DPI_AWARENESS_CONTEXT_SYSTEM_AWARE: DpiAwarenessContext = -2isize as _;
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE: DpiAwarenessContext = -3isize as _;
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: DpiAwarenessContext = -4isize as _;
const PROCESS_SYSTEM_DPI_AWARE: ProcessDpiAwareness = 1;
const PROCESS_PER_MONITOR_DPI_AWARE: ProcessDpiAwareness = 2;
const MEM_COMMIT: u32 = 0x1000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_READONLY: u32 = 0x02;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_WRITECOPY: u32 = 0x08;
const PAGE_EXECUTE_READ: u32 = 0x20;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;
const PAGE_GUARD: u32 = 0x100;
const PAGE_READABLE_MASK: u32 = PAGE_READONLY
    | PAGE_READWRITE
    | PAGE_WRITECOPY
    | PAGE_EXECUTE_READ
    | PAGE_EXECUTE_READWRITE
    | PAGE_EXECUTE_WRITECOPY;

type DpiAwarenessContext = *mut c_void;
type ProcessDpiAwareness = i32;
type SetProcessDpiAwarenessContextFn =
    unsafe extern "system" fn(DpiAwarenessContext) -> windows_sys::core::BOOL;
type SetProcessDpiAwarenessFn =
    unsafe extern "system" fn(ProcessDpiAwareness) -> windows_sys::core::HRESULT;
type SetProcessDpiAwareFn = unsafe extern "system" fn() -> windows_sys::core::BOOL;
type EnableNonClientDpiScalingFn = unsafe extern "system" fn(HWND) -> windows_sys::core::BOOL;
type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;

#[repr(C)]
#[allow(dead_code)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    type_: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualQuery(
        address: *const c_void,
        buffer: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;
}

macro_rules! load_function {
    ($library:literal, $function:ident, $function_type:ty) => {{
        let procedure = load_procedure(
            concat!($library, "\0").as_bytes(),
            concat!(stringify!($function), "\0").as_bytes(),
        );
        procedure.map(|procedure| {
            // SAFETY: The symbol name and target function pointer type are paired at each call
            // site with the corresponding Win32 API signature.
            unsafe {
                mem::transmute::<unsafe extern "system" fn() -> isize, $function_type>(procedure)
            }
        })
    }};
}

pub(super) fn enable_process_dpi_awareness() {
    static ENABLE_DPI_AWARENESS: Once = Once::new();

    ENABLE_DPI_AWARENESS.call_once(|| {
        if let Some(set_awareness_context) = load_function!(
            "user32.dll",
            SetProcessDpiAwarenessContext,
            SetProcessDpiAwarenessContextFn
        ) {
            for context in [
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
            ] {
                if unsafe { set_awareness_context(context) } != 0 {
                    return;
                }
            }
        }

        if let Some(set_awareness) = load_function!(
            "shcore.dll",
            SetProcessDpiAwareness,
            SetProcessDpiAwarenessFn
        ) {
            for awareness in [PROCESS_PER_MONITOR_DPI_AWARE, PROCESS_SYSTEM_DPI_AWARE] {
                if hresult_succeeded(unsafe { set_awareness(awareness) }) {
                    return;
                }
            }
        }

        if let Some(set_dpi_aware) =
            load_function!("user32.dll", SetProcessDPIAware, SetProcessDpiAwareFn)
        {
            let _ = unsafe { set_dpi_aware() };
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DpiMetrics {
    dpi_y: i32,
}

impl DpiMetrics {
    pub(super) fn system() -> Self {
        // SAFETY: A null HWND asks GDI for a screen DC; dpi_y_for_window falls back to 96 DPI if
        // the DC cannot be acquired.
        unsafe { Self::from_dpi_y(dpi_y_for_window(ptr::null_mut())) }
    }

    pub(super) unsafe fn for_window(hwnd: HWND) -> Self {
        Self::from_dpi_y(dpi_y_for_window(hwnd))
    }

    pub(super) fn ui_scale(self) -> UiScale {
        UiScale { dpi_y: self.dpi_y }
    }

    fn from_dpi_y(dpi_y: i32) -> Self {
        Self {
            dpi_y: dpi_y.max(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiScale {
    dpi_y: i32,
}

impl UiScale {
    pub(super) fn px(self, value: i32) -> i32 {
        scale_pixels(value, self.dpi_y)
    }
}

pub(super) unsafe fn enable_non_client_dpi_scaling(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }

    if let Some(enable_scaling) = load_function!(
        "user32.dll",
        EnableNonClientDpiScaling,
        EnableNonClientDpiScalingFn
    ) {
        let _ = enable_scaling(hwnd);
    }
}

pub(super) unsafe fn dpi_y_for_window(hwnd: HWND) -> i32 {
    if !hwnd.is_null() {
        if let Some(dpi) = dpi_for_window(hwnd) {
            return dpi as i32;
        }
    }

    let hdc = GetDC(hwnd);
    if hdc.is_null() {
        return DEFAULT_DPI as i32;
    }

    let dpi_y = GetDeviceCaps(hdc, LOGPIXELSY as i32);
    ReleaseDC(hwnd, hdc);
    if dpi_y > 0 {
        dpi_y
    } else {
        DEFAULT_DPI as i32
    }
}

pub(super) unsafe fn suggested_rect_from_dpi_change(lparam: LPARAM) -> Option<RECT> {
    let pointer = rect_pointer_from_dpi_change_lparam(lparam)?;

    // SAFETY: rect_pointer_from_dpi_change_lparam rejects null, unaligned, overflowing,
    // and non-readable RECT-sized ranges before this raw pointer read.
    Some(unsafe { ptr::read(pointer) })
}

fn rect_pointer_from_dpi_change_lparam(lparam: LPARAM) -> Option<*const RECT> {
    let Ok(address) = usize::try_from(lparam) else {
        return None;
    };
    if address == 0 {
        return None;
    }

    let size = mem::size_of::<RECT>();
    address.checked_add(size)?;

    let pointer = address as *const RECT;
    if !(address.is_multiple_of(mem::align_of::<RECT>())) {
        return None;
    }

    pointer_is_readable(pointer.cast::<c_void>(), size).then_some(pointer)
}

fn pointer_is_readable(pointer: *const c_void, size: usize) -> bool {
    let mut memory = MemoryBasicInformation {
        base_address: ptr::null_mut(),
        allocation_base: ptr::null_mut(),
        allocation_protect: 0,
        partition_id: 0,
        region_size: 0,
        state: 0,
        protect: 0,
        type_: 0,
    };

    // SAFETY: VirtualQuery reads memory metadata for the supplied address and writes it
    // into the initialized local MEMORY_BASIC_INFORMATION-compatible buffer.
    let queried = unsafe {
        VirtualQuery(
            pointer,
            &mut memory,
            mem::size_of::<MemoryBasicInformation>(),
        )
    };
    if queried == 0
        || memory.state != MEM_COMMIT
        || memory.protect & (PAGE_NOACCESS | PAGE_GUARD) != 0
        || memory.protect & PAGE_READABLE_MASK == 0
    {
        return false;
    }

    let start = pointer as usize;
    let Some(end) = start.checked_add(size) else {
        return false;
    };
    let region_start = memory.base_address as usize;
    let Some(region_end) = region_start.checked_add(memory.region_size) else {
        return false;
    };

    region_start <= start && end <= region_end
}

fn scale_pixels(value: i32, dpi_y: i32) -> i32 {
    if value == 0 {
        return 0;
    }

    let sign = value.signum();
    let magnitude = i64::from(value).abs();
    let dpi_y = i64::from(dpi_y.max(1));
    let scaled = (magnitude * dpi_y + i64::from(DEFAULT_DPI / 2)) / i64::from(DEFAULT_DPI);
    let scaled = scaled.clamp(1, i64::from(i32::MAX)) as i32;
    scaled * sign
}

fn hresult_succeeded(result: windows_sys::core::HRESULT) -> bool {
    result >= 0
}

unsafe fn dpi_for_window(hwnd: HWND) -> Option<u32> {
    static GET_DPI_FOR_WINDOW: OnceLock<Option<GetDpiForWindowFn>> = OnceLock::new();

    let get_dpi = (*GET_DPI_FOR_WINDOW
        .get_or_init(|| load_function!("user32.dll", GetDpiForWindow, GetDpiForWindowFn)))?;
    let dpi = get_dpi(hwnd);
    (dpi > 0).then_some(dpi)
}

fn load_procedure(library: &'static [u8], function: &'static [u8]) -> FARPROC {
    debug_assert_eq!(library.last(), Some(&0));
    debug_assert_eq!(function.last(), Some(&0));

    let module = unsafe {
        LoadLibraryExA(
            library.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return None;
    }

    unsafe { GetProcAddress(module, function.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_scale_rounds_pixels_from_96_dpi_baseline() {
        assert_eq!(UiScale { dpi_y: 96 }.px(24), 24);
        assert_eq!(UiScale { dpi_y: 120 }.px(24), 30);
        assert_eq!(UiScale { dpi_y: 144 }.px(5), 8);
    }

    #[test]
    fn ui_scale_preserves_zero_and_negative_values() {
        assert_eq!(UiScale { dpi_y: 144 }.px(0), 0);
        assert_eq!(UiScale { dpi_y: 144 }.px(-4), -6);
    }
}
