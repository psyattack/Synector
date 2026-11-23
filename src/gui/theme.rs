//! Design-token layer of the GUI: one resolved style per frame.
//!
//! Two independent axes meet here and nowhere else:
//!
//! - [`tokens`] owns the colour palettes and the resolved [`Tokens`] a view reads;
//! - [`skin`] owns the geometry, density, stroke, elevation and motion metrics;
//! - [`style`] derives egui's `Visuals`, spacing and text styles from both;
//! - [`motion`] answers whether the platform wants animated transitions;
//! - [`paint`] holds the shared painting helpers (including the window backdrop);
//! - [`fonts`] registers the bundled and system type stacks.
//!
//! Views read [`Tokens`] for roles and metrics instead of hardcoding colours, radii or control
//! heights, so a new visual generation is a new metrics value rather than a second view tree.
//! The few surfaces that genuinely compose differently per generation — the header tabs, the
//! appearance previews — read `Metrics::skin` for that one branch, never to pick a colour.

mod fonts;
pub mod motion;
mod paint;
mod skin;
mod style;
mod tokens;

pub use fonts::*;
pub use skin::*;
pub use style::*;
pub use tokens::*;

pub(crate) use paint::{paint_backdrop, tag_color, well};

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("view directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// The skin owns the corner radius of every framed surface. A view that picks a literal
    /// radius cannot follow the active generation, so this guard keeps the sweep from eroding.
    #[test]
    fn views_never_pick_a_literal_frame_radius() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gui/views");
        let mut files = Vec::new();
        rust_files(&root, &mut files);
        assert!(!files.is_empty(), "the view directory must exist");

        let mut offenders = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).expect("read a view file");
            for (index, line) in text.lines().enumerate() {
                if line.contains("corner_radius(CornerRadius::") {
                    offenders.push(format!("{}:{}", file.display(), index + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "frames take their corner radius from the skin, not from a literal: {offenders:?}"
        );
    }
}
