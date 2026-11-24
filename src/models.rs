//! Domain model facade. Every `crate::models::…` path resolves through the re-exports below.
//!
//! Rule types live next to their own validators; the internally tagged wire enums
//! (`Action`, `Trigger`, `RuleCondition`, `OverlayElement`, `InteractiveControlKind`,
//! `OverlayClickAction`) stay in exactly one module each so the serde/schemars shape is the
//! one the JSON schema has always published. Serde default helpers stay private to this tree.

mod action;
mod analytics;
mod condition;
mod macros;
mod overlay;
mod quota;
mod rule;
mod schema;
mod serde_defaults;
mod state;
mod trigger;
mod validation;

#[cfg(test)]
mod tests;

pub use action::*;
pub use analytics::*;
pub use condition::*;
pub use macros::*;
pub use overlay::*;
pub use quota::*;
pub use rule::*;
pub use schema::rule_schema;
pub use state::*;
pub use trigger::*;
pub use validation::*;
