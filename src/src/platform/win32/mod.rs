mod commands;
mod common;
mod dpi;
mod file_dialog;
mod font;
mod i18n;
mod layout;
mod menu;
mod size_move;
mod state;
mod tabs;
mod text;
mod theme;
mod tree;
mod window;

pub use window::{run_message_loop, show_error_message};

#[cfg(test)]
mod test_support {
    use std::cell::Cell;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    thread_local! {
        static WIN32_CONTROL_TEST_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) struct Win32ControlTestGuard {
        _guard: Option<MutexGuard<'static, ()>>,
    }

    pub(super) fn enter_win32_control_test() -> Win32ControlTestGuard {
        let nested = WIN32_CONTROL_TEST_LOCK_DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(current.saturating_add(1));
            current > 0
        });
        let guard = if nested {
            None
        } else {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            Some(
                LOCK.get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            )
        };

        Win32ControlTestGuard { _guard: guard }
    }

    impl Drop for Win32ControlTestGuard {
        fn drop(&mut self) {
            WIN32_CONTROL_TEST_LOCK_DEPTH.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }
}

// SAFETY: This module is the Win32 FFI boundary. Unsafe functions here assume HWNDs and message
// pointers come from windows and controls created in this module on the UI thread. Raw pointers are
// checked for null before dereference, and the WindowState pointer stored in GWLP_USERDATA is owned
// by the main window until WM_NCDESTROY releases it.
