//! CPU gradient rendering and an embeddable Ratatui view.
//!
//! The host application owns terminal setup, input, layout, and its event loop.
//! Call [`GradientView::update`] with the current time, then render the view
//! into any rectangle. Draw other widgets afterward to put them on top.
mod animation;
mod color;
#[cfg(feature = "generation")]
pub mod generation;
mod gradient;
mod half_blocks;
pub mod presets;
mod recipe;
pub mod scene;
mod view;
mod warp;

pub use color::Rgb;
pub use gradient::{Point, PreparedGradient};
pub use half_blocks::{HalfBlocks, Preview};
pub use recipe::{CompiledRecipe, MotionStyle, Recipe};
pub use scene::Scene;
pub use view::{GradientView, Playback, RenderStats};
pub use warp::{FoldWave, LayerAnimation, LayerMotion, WarpAxis, WarpLayer};
