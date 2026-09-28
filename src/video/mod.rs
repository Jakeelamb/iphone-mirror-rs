//! Direct HEVC decoding and a bounded, latest-frame GPU presentation path.
mod decoder;
mod frame;
mod layout;
mod renderer;

pub use decoder::{DecodeMode, Decoder};
pub use frame::{DecodedFrame, LatestFrame};
pub use layout::{Rect, ViewerLayout};
pub use renderer::Renderer;
