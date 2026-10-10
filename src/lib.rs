//! CPU gradient rendering and an embeddable Ratatui view.
//!
//! The host application owns terminal setup, input, layout, and its event loop.
//! Call [`GradientView::update`] with the current time, then render the view
//! into any rectangle. Draw other widgets afterward to put them on top.
pub mod ambient;
mod animation;
pub mod audio;
mod color;
mod digits;
#[cfg(feature = "focus-check")]
pub mod focus;
#[cfg(feature = "generation")]
pub mod generation;
mod gradient;
mod half_blocks;
mod overlay;
pub mod presets;
mod recipe;
pub mod scene;
pub mod tasks;
mod view;
mod warp;

pub use color::Rgb;
pub use digits::Digits;
pub use gradient::{Point, PreparedGradient};
pub use half_blocks::{HalfBlocks, Preview};
pub use overlay::blend_samples;
pub use recipe::{CompiledRecipe, MotionStyle, Recipe};
pub use scene::Scene;
pub use view::{Excitement, GradientView, Playback, Reactivity, RenderStats};
pub use warp::{FoldWave, LayerAnimation, LayerMotion, WarpAxis, WarpLayer};
