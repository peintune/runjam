//! Ephemeral pet popup state. Hiding a window never ends its agent session.
#[derive(Default)]
pub struct PetChatState {
    pub pinned: bool,
    pub dialog_depth: usize,
    pub generation: u64,
}

impl PetChatState {
    pub fn invalidate(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
    pub fn should_hide(&self, generation: u64, visible: bool, focused: bool) -> bool {
        generation == self.generation
            && !self.pinned
            && self.dialog_depth == 0
            && visible
            && !focused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_unpinned_and_hides_only_after_losing_focus() {
        let state = PetChatState::default();
        assert!(!state.pinned);
        assert!(state.should_hide(0, true, false));
        assert!(!state.should_hide(0, true, true));
        assert!(!state.should_hide(0, false, false));
    }

    #[test]
    fn pinned_windows_and_native_dialogs_are_not_auto_hidden() {
        let mut state = PetChatState::default();
        state.pinned = true;
        assert!(!state.should_hide(0, true, false));
        state.pinned = false;
        state.dialog_depth = 1;
        assert!(!state.should_hide(0, true, false));
    }

    #[test]
    fn old_blur_cannot_hide_a_reopened_or_reconfigured_window() {
        let mut state = PetChatState::default();
        let old = state.generation;
        let current = state.invalidate();
        assert_ne!(old, current);
        assert!(!state.should_hide(old, true, false));
        assert!(state.should_hide(current, true, false));
    }
}
