#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SizeMoveState {
    in_loop: bool,
    dpi_changed: bool,
    search_debounce_pending: bool,
}

impl SizeMoveState {
    pub(super) fn enter(&mut self) {
        self.in_loop = true;
        self.dpi_changed = false;
        self.search_debounce_pending = false;
    }

    pub(super) fn exit(&mut self) -> SizeMoveExit {
        let exit = SizeMoveExit {
            dpi_changed: self.dpi_changed,
            search_debounce_pending: self.search_debounce_pending,
        };
        self.in_loop = false;
        self.dpi_changed = false;
        self.search_debounce_pending = false;
        exit
    }

    pub(super) fn in_loop(self) -> bool {
        self.in_loop
    }

    pub(super) fn dpi_changed(self) -> bool {
        self.dpi_changed
    }

    pub(super) fn defer_dpi_change(&mut self) {
        self.dpi_changed = true;
    }

    pub(super) fn defer_search_debounce(&mut self) {
        self.search_debounce_pending = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SizeMoveExit {
    pub(super) dpi_changed: bool,
    pub(super) search_debounce_pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_resets_pending_work_and_marks_loop_active() {
        let mut state = SizeMoveState::default();
        state.defer_dpi_change();
        state.defer_search_debounce();

        state.enter();

        assert!(state.in_loop());
        assert!(!state.dpi_changed());
        assert_eq!(
            state.exit(),
            SizeMoveExit {
                dpi_changed: false,
                search_debounce_pending: false,
            }
        );
    }

    #[test]
    fn exit_returns_pending_work_and_clears_state() {
        let mut state = SizeMoveState::default();
        state.enter();
        state.defer_dpi_change();
        state.defer_search_debounce();

        assert_eq!(
            state.exit(),
            SizeMoveExit {
                dpi_changed: true,
                search_debounce_pending: true,
            }
        );
        assert!(!state.in_loop());
        assert!(!state.dpi_changed());
        assert_eq!(
            state.exit(),
            SizeMoveExit {
                dpi_changed: false,
                search_debounce_pending: false,
            }
        );
    }
}
