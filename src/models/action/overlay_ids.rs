use crate::models::Action;

impl Action {
    pub fn refresh_overlay_ids(&mut self) {
        match self {
            Action::Overlay { overlay_id, custom_overlay, .. } => {
                if custom_overlay.is_some() {
                    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let fresh_id = format!(
                        "overlay_{:x}_{:x}",
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
                        count
                    );
                    *overlay_id = fresh_id.clone();
                    if let Some(def) = custom_overlay.as_deref_mut() {
                        def.id = fresh_id;
                    }
                }
            }
            Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                for a in then_actions {
                    a.refresh_overlay_ids();
                }
                for b in else_if_branches {
                    for a in &mut b.actions {
                        a.refresh_overlay_ids();
                    }
                }
                for a in else_actions {
                    a.refresh_overlay_ids();
                }
            }
            _ => {}
        }
    }

    /// Makes the two overlay identities of an [`Action::Overlay`] agree: the embedded
    /// definition takes the action's `overlay_id` (or donates its id when that is empty).
    ///
    /// The runner shows the embedded definition while the engine binds `close_on_exit`
    /// under the action's id; a mismatch between them leaks the overlay. Called on every
    /// path that accepts a rule definition (config load, share import, CLI apply).
    pub fn normalize_overlay_ids(&mut self) {
        match self {
            Action::Overlay {
                overlay_id,
                custom_overlay,
                ..
            } => {
                if let Some(definition) = custom_overlay.as_deref_mut() {
                    if overlay_id.trim().is_empty() {
                        *overlay_id = definition.id.clone();
                    } else {
                        definition.id = overlay_id.clone();
                    }
                }
            }
            Action::IfElse {
                then_actions,
                else_if_branches,
                else_actions,
                ..
            } => {
                for action in then_actions {
                    action.normalize_overlay_ids();
                }
                for branch in else_if_branches {
                    for action in &mut branch.actions {
                        action.normalize_overlay_ids();
                    }
                }
                for action in else_actions {
                    action.normalize_overlay_ids();
                }
            }
            _ => {}
        }
    }

    pub fn duplicate_with_fresh_ids(&self) -> Self {
        let mut cloned = self.clone();
        cloned.normalize_overlay_ids();
        cloned.refresh_overlay_ids();
        cloned
    }
}
