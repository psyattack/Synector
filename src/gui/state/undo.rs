//! One bounded undo/redo implementation shared by the rule draft and the overlay studio.
//!
//! Both histories have to stay public `Vec<T>` fields this session (tests and
//! `overlay_modal.rs` read them directly), so the shared algorithm operates on the stacks
//! themselves instead of wrapping them in a new type.

use super::overlay_studio::OverlayStudioState;
use super::GuiState;
use crate::models::Rule;

/// Maximum number of snapshots kept in one undo history; the oldest entry is dropped first.
const UNDO_LIMIT: usize = 50;

/// Appends one snapshot unless it repeats the newest entry, trims the history to
/// [`UNDO_LIMIT`] and clears the redo branch. Consecutive identical snapshots are ignored so a
/// layout operation and the inspector session that saw it cannot push the same state twice.
pub(crate) fn push_if_new<T: Clone + PartialEq>(undo: &mut Vec<T>, redo: &mut Vec<T>, snapshot: &T) {
    if undo.last() == Some(snapshot) {
        return;
    }
    undo.push(snapshot.clone());
    if undo.len() > UNDO_LIMIT {
        undo.remove(0);
    }
    redo.clear();
}

/// Moves the current value to the redo stack and restores the newest undo snapshot in its
/// place. Returns `false` (and leaves `current` untouched) when the history is empty.
pub(super) fn undo_step<T>(undo: &mut Vec<T>, redo: &mut Vec<T>, current: &mut T) -> bool {
    match undo.pop() {
        Some(previous) => {
            redo.push(std::mem::replace(current, previous));
            true
        }
        None => false,
    }
}

/// Moves the current value to the undo stack and restores the newest redo snapshot in its
/// place. Returns `false` (and leaves `current` untouched) when the redo branch is empty.
pub(super) fn redo_step<T>(undo: &mut Vec<T>, redo: &mut Vec<T>, current: &mut T) -> bool {
    match redo.pop() {
        Some(next) => {
            undo.push(std::mem::replace(current, next));
            true
        }
        None => false,
    }
}

pub(super) fn history_can_undo<T>(undo: &[T]) -> bool {
    !undo.is_empty()
}

pub(super) fn history_can_redo<T>(redo: &[T]) -> bool {
    !redo.is_empty()
}

impl OverlayStudioState {
    pub fn push_undo_snapshot(&mut self) {
        push_if_new(&mut self.undo_stack, &mut self.redo_stack, &self.overlay);
    }

    pub fn can_undo(&self) -> bool {
        history_can_undo(&self.undo_stack)
    }

    pub fn can_redo(&self) -> bool {
        history_can_redo(&self.redo_stack)
    }

    pub fn undo(&mut self) {
        if undo_step(&mut self.undo_stack, &mut self.redo_stack, &mut self.overlay) {
            if let Some(sel) = self.selected_element_idx {
                if sel >= self.overlay.elements.len() {
                    self.selected_element_idx = None;
                }
            }
        }
    }

    pub fn redo(&mut self) {
        if redo_step(&mut self.undo_stack, &mut self.redo_stack, &mut self.overlay) {
            if let Some(sel) = self.selected_element_idx {
                if sel >= self.overlay.elements.len() {
                    self.selected_element_idx = None;
                }
            }
        }
    }
}

impl GuiState {
    pub fn push_undo_snapshot(&mut self) {
        if let Some(r) = &self.editing_rule {
            push_if_new(&mut self.undo_stack, &mut self.redo_stack, r);
        }
    }

    pub fn push_undo_snapshot_of(&mut self, r: &Rule) {
        push_if_new(&mut self.undo_stack, &mut self.redo_stack, r);
    }

    pub fn undo(&mut self) {
        let Some(current) = self.editing_rule.as_mut() else {
            if let Some(prev) = self.undo_stack.pop() {
                self.editing_rule = Some(prev);
            }
            return;
        };
        undo_step(&mut self.undo_stack, &mut self.redo_stack, current);
    }

    pub fn redo(&mut self) {
        let Some(current) = self.editing_rule.as_mut() else {
            if let Some(next) = self.redo_stack.pop() {
                self.editing_rule = Some(next);
            }
            return;
        };
        redo_step(&mut self.undo_stack, &mut self.redo_stack, current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_redo_multi_step() {
        let mut state = GuiState::new();
        let mut r0 = state.initial_new_rule();
        r0.name = "Step 0".to_string();
        state.editing_rule = Some(r0.clone());

        let mut r1 = r0.clone();
        r1.name = "Step 1".to_string();
        state.push_undo_snapshot_of(&r0);
        state.editing_rule = Some(r1.clone());

        let mut r2 = r1.clone();
        r2.name = "Step 2".to_string();
        state.push_undo_snapshot_of(&r1);
        state.editing_rule = Some(r2.clone());

        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 2");
        assert_eq!(state.undo_stack.len(), 2);
        assert_eq!(state.redo_stack.len(), 0);

        // Undo 1 -> Step 1
        state.undo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 1");
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.redo_stack.len(), 1);

        // Undo 2 -> Step 0
        state.undo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 0");
        assert_eq!(state.undo_stack.len(), 0);
        assert_eq!(state.redo_stack.len(), 2);

        // Redo 1 -> Step 1
        state.redo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 1");
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.redo_stack.len(), 1);

        // Redo 2 -> Step 2
        state.redo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 2");
        assert_eq!(state.undo_stack.len(), 2);
        assert_eq!(state.redo_stack.len(), 0);
    }
}
