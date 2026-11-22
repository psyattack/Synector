//! The four card-grid catalog modals: trigger, condition, action and overlay element.
//!
//! Each module owns its model table, categories, chrome constants and a short commit adapter.
//! The shared shell — entry definition, chrome, filtering, modal lifecycle and the
//! close-animation gate — lives in `crate::gui::components::catalog_modal`.

pub mod action;
pub mod condition;
pub mod element;
pub mod trigger;
