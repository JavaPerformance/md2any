//! Layout selection — the visual frame a slide is dressed in.
//!
//! Themes (colours/fonts) and layouts (geometry/chrome) are deliberately
//! orthogonal: any layout can pair with any theme. The four layouts shipped
//! here are what's exposed via `--layout`; renderers consult [`LayoutKind`]
//! to decide where the title goes, whether a vertical accent rail is drawn,
//! how section dividers look, and so on.

use anyhow::{bail, Result};
use serde::Deserialize;

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

/// Customizable layout geometry/chrome, applied over a base layout via a
/// `layout:` block in a `--theme-file`. Only keys present are applied, so a
/// user can e.g. widen the `frame` sidebar or turn off the `clean` underline
/// without redefining the whole layout. Widths are in EMU (914,400 per inch).
#[derive(Debug, Default, Deserialize, Clone)]
pub struct LayoutOverride {
    pub rail_width: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub title_underline: Option<bool>,
    pub section_full_bg: Option<bool>,
}

/// A layout is a base [`LayoutKind`] (which fixes the structural chrome) plus
/// data-driven geometry knobs the renderers consult. The knobs start from the
/// kind's preset and can be overridden by a [`LayoutOverride`].
#[derive(Debug, Clone)]
pub struct Layout {
    pub kind: LayoutKind,
    rail_width: u32,
    sidebar_width: u32,
    title_underline: bool,
    section_full_bg: bool,
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
        Ok(Layout::from_kind(kind))
    }

    /// The default geometry preset for each kind.
    fn from_kind(kind: LayoutKind) -> Self {
        let (rail_width, sidebar_width, title_underline, section_full_bg) = match kind {
            LayoutKind::Clean => (0, 0, true, true),
            LayoutKind::Studio => (90_000, 0, false, false),
            LayoutKind::Frame => (0, 2_200_000, false, true),
            LayoutKind::Bold => (0, 0, false, false),
        };
        Layout {
            kind,
            rail_width,
            sidebar_width,
            title_underline,
            section_full_bg,
        }
    }

    /// Layer a `layout:` overlay onto the resolved layout.
    pub fn apply_override(&mut self, ov: &LayoutOverride) {
        if let Some(v) = ov.rail_width {
            self.rail_width = v;
        }
        if let Some(v) = ov.sidebar_width {
            self.sidebar_width = v;
        }
        if let Some(v) = ov.title_underline {
            self.title_underline = v;
        }
        if let Some(v) = ov.section_full_bg {
            self.section_full_bg = v;
        }
    }

    pub fn rail_width(&self) -> u32 {
        self.rail_width
    }

    pub fn sidebar_width(&self) -> u32 {
        self.sidebar_width
    }

    pub fn content_left_offset(&self) -> u32 {
        self.rail_width().max(self.sidebar_width())
    }

    pub fn title_underline(&self) -> bool {
        self.title_underline
    }

    pub fn title_block_bg(&self) -> bool {
        matches!(self.kind, LayoutKind::Bold)
    }

    pub fn section_full_bg(&self) -> bool {
        self.section_full_bg
    }

    pub fn shows_sidebar(&self) -> bool {
        self.sidebar_width > 0
    }

    pub fn shows_rail(&self) -> bool {
        self.rail_width > 0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_kinds() {
        let clean = Layout::resolve("clean").unwrap();
        assert!(clean.title_underline() && clean.section_full_bg() && !clean.shows_rail());
        let frame = Layout::resolve("frame").unwrap();
        assert!(frame.shows_sidebar() && frame.sidebar_width() == 2_200_000);
        assert!(Layout::resolve("studio").unwrap().shows_rail());
    }

    #[test]
    fn override_layers_onto_base() {
        let mut clean = Layout::resolve("clean").unwrap();
        clean.apply_override(&LayoutOverride {
            title_underline: Some(false),
            ..Default::default()
        });
        assert!(!clean.title_underline());
        // untouched knobs keep the preset
        assert!(clean.section_full_bg());

        let mut frame = Layout::resolve("frame").unwrap();
        frame.apply_override(&LayoutOverride {
            sidebar_width: Some(3_400_000),
            ..Default::default()
        });
        assert_eq!(frame.sidebar_width(), 3_400_000);
        assert!(frame.shows_sidebar());
    }
}
