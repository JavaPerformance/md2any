//! Layout selection — the visual frame a slide is dressed in.
//!
//! Themes (colours/fonts) and layouts (geometry/chrome) are deliberately
//! orthogonal: any layout can pair with any theme. The four layouts shipped
//! here are what's exposed via `--layout`; renderers consult [`LayoutKind`]
//! to decide where the title goes, whether a vertical accent rail is drawn,
//! how section dividers look, and so on.

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    /// Minimal — title underline accent, full-bleed content. The default.
    Clean,
    /// Editorial — vertical accent rail at left edge, no title underline,
    /// section slides stay on the page background.
    Studio,
    /// Sidebar — left accent panel holds deck title and slide number;
    /// content lives on the right.
    Frame,
    /// Magazine — big accent block behind every title.
    Bold,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub kind: LayoutKind,
}

impl Layout {
    pub fn resolve(name: &str) -> Result<Self> {
        let kind = match name.to_lowercase().as_str() {
            "clean" | "default" | "" => LayoutKind::Clean,
            "studio" | "editorial" => LayoutKind::Studio,
            "frame" | "sidebar" => LayoutKind::Frame,
            "bold" | "magazine" => LayoutKind::Bold,
            other => bail!(
                "unknown layout: {} (try clean, studio, frame, or bold)",
                other
            ),
        };
        Ok(Layout { kind })
    }

    pub fn rail_width(&self) -> u32 {
        match self.kind {
            LayoutKind::Studio => 90000,
            _ => 0,
        }
    }

    pub fn sidebar_width(&self) -> u32 {
        match self.kind {
            LayoutKind::Frame => 2_200_000,
            _ => 0,
        }
    }

    pub fn content_left_offset(&self) -> u32 {
        self.rail_width().max(self.sidebar_width())
    }

    pub fn title_underline(&self) -> bool {
        matches!(self.kind, LayoutKind::Clean)
    }

    pub fn title_block_bg(&self) -> bool {
        matches!(self.kind, LayoutKind::Bold)
    }

    pub fn section_full_bg(&self) -> bool {
        matches!(self.kind, LayoutKind::Clean | LayoutKind::Frame)
    }

    pub fn shows_sidebar(&self) -> bool {
        matches!(self.kind, LayoutKind::Frame)
    }

    pub fn shows_rail(&self) -> bool {
        matches!(self.kind, LayoutKind::Studio)
    }

    pub fn shows_corner_decoration(&self) -> bool {
        matches!(self.kind, LayoutKind::Studio)
    }

    pub fn name(&self) -> &'static str {
        match self.kind {
            LayoutKind::Clean => "clean",
            LayoutKind::Studio => "studio",
            LayoutKind::Frame => "frame",
            LayoutKind::Bold => "bold",
        }
    }
}
