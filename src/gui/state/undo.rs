//! One bounded undo/redo implementation shared by the rule draft and the overlay studio.
//!
//! Both histories stay plain `Vec<T>` fields on their owners — tests and `overlay_modal.rs`
//! read them directly — so the shared algorithm operates on the stacks themselves instead of
//! wrapping them in a new type.

use super::overlay_studio::OverlayStudioState;
use super::{ExpandedScriptEditorState, GuiState};
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

impl ExpandedScriptEditorState {
    /// Appends the text from before an edit gesture to the dialog's own history and clears its
    /// redo branch. The dialog walks this history with Ctrl+Z / Ctrl+Y while it is open, exactly
    /// like the studios walk theirs.
    pub fn push_undo_snapshot(&mut self, text: String) {
        push_if_new(&mut self.undo_stack, &mut self.redo_stack, &text);
    }

    /// Commits the typing gesture in progress against the text the editor shows now. A gesture
    /// that ended where it started (typed and deleted again) leaves no no-op undo step behind.
    pub(crate) fn commit_pending_undo(&mut self, current: &str) {
        if let Some(pending) = self.pending_undo.take() {
            if pending != current {
                self.push_undo_snapshot(pending);
            }
        }
    }

    /// Restores the newest snapshot and returns the text the editor must show now, or `None`
    /// while the history has nothing to undo.
    pub fn undo(&mut self, current: &str) -> Option<String> {
        let mut text = current.to_string();
        undo_step(&mut self.undo_stack, &mut self.redo_stack, &mut text).then_some(text)
    }

    /// Restores the newest redo snapshot and returns the text the editor must show now, or `None`
    /// while the redo branch is empty.
    pub fn redo(&mut self, current: &str) -> Option<String> {
        let mut text = current.to_string();
        redo_step(&mut self.undo_stack, &mut self.redo_stack, &mut text).then_some(text)
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

    /// The script dialog's history is the same bounded stack, with the script text as the
    /// snapshot: repeated snapshots are ignored, undo/redo round-trip, and typing after an undo
    /// starts a new branch.
    #[test]
    fn test_script_dialog_history_round_trips_and_drops_the_redo_branch() {
        let mut editor = ExpandedScriptEditorState::new(
            0,
            crate::gui::components::script_highlighter::ScriptLanguage::PowerShell,
        );
        editor.push_undo_snapshot("a".to_string());
        editor.push_undo_snapshot("a".to_string());
        editor.push_undo_snapshot("b".to_string());
        assert_eq!(editor.undo_stack.len(), 2, "a repeated snapshot is not a new step");

        assert_eq!(editor.undo("c").as_deref(), Some("b"));
        assert_eq!(editor.undo("b").as_deref(), Some("a"));
        assert_eq!(editor.undo("a"), None, "nothing left to undo");
        assert_eq!(editor.redo("a").as_deref(), Some("b"));
        assert_eq!(editor.redo("b").as_deref(), Some("c"));
        assert_eq!(editor.redo("c"), None, "nothing left to redo");

        // Typing after an undo abandons the redo branch.
        assert_eq!(editor.undo("c").as_deref(), Some("b"));
        editor.push_undo_snapshot("typed".to_string());
        assert!(editor.redo_stack.is_empty());
        assert_eq!(editor.undo("new text").as_deref(), Some("typed"));

        // A gesture that ended where it started is not recorded as a no-op step, while a real
        // gesture commits the text from before it.
        let committed = editor.undo_stack.len();
        editor.pending_undo = Some("same text".to_string());
        editor.commit_pending_undo("same text");
        assert_eq!(editor.undo_stack.len(), committed);
        editor.pending_undo = Some("typed".to_string());
        editor.commit_pending_undo("typed more");
        assert_eq!(editor.undo_stack.len(), committed + 1);
    }
}
