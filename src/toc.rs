//! Auto Table-of-Contents slide.
//!
//! When front-matter sets `toc: true`, we inject a content slide titled
//! "Contents" listing every H1 section in the deck. Inserted after the
//! title slide (or at the very start if there's no title slide).

use crate::ir::*;

pub fn inject(slides: Vec<Slide>) -> Vec<Slide> {
    let entries: Vec<String> = slides
        .iter()
        .filter(|s| matches!(s.kind, SlideKind::Section))
        .map(|s| s.title.clone())
        .collect();
    if entries.is_empty() {
        return slides;
    }

    let items: Vec<ListItem> = entries
        .into_iter()
        .map(|title| ListItem {
            runs: vec![Run::plain(title)],
            level: 0,
            ordered: true,
        })
        .collect();

    let toc_slide = Slide {
        kind: SlideKind::Content,
        title: "Contents".into(),
        blocks: vec![Block::List(items)],
        notes: None,
        bg_image: None,
        layout_hint: None,
    };

    let title_idx = slides
        .iter()
        .position(|s| matches!(s.kind, SlideKind::Title { .. }));
    let mut out = slides;
    let insert_at = match title_idx {
        Some(i) => i + 1,
        None => 0,
    };
    out.insert(insert_at, toc_slide);
    out
}
