//! Direct HEVC decoding and a bounded, latest-frame GPU presentation path.
mod decoder;
mod frame;
mod layout;
mod orientation;
mod presentation;
mod renderer;

pub use decoder::{DecodeMode, Decoder};
pub use frame::{DecodedFrame, LatestFrame};
pub use layout::{Rect, ViewerLayout};
pub use orientation::{displayed_size, visual_rotation};
pub use presentation::{PresentPreference, PresentationOptions, RendererGpu};
pub use renderer::{RenderSample, Renderer};
