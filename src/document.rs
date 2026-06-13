//! Shared controls for flowing document outputs.
//!
//! DOCX and ODT are not slide containers in md2any: they turn the same slide
//! IR into a readable document. The style profile here lets callers choose
//! whether that document should stay close to the old plain flow or gain
//! report/handout/speaker-note structure.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentStyle {
    Plain,
    Report,
    Handout,
    SpeakerNotes,
}

impl Default for DocumentStyle {
    fn default() -> Self {
        DocumentStyle::Report
    }
}

impl DocumentStyle {
    pub fn name(self) -> &'static str {
        match self {
            DocumentStyle::Plain => "plain",
            DocumentStyle::Report => "report",
            DocumentStyle::Handout => "handout",
            DocumentStyle::SpeakerNotes => "speaker-notes",
        }
    }

    pub fn has_title_page(self) -> bool {
        !matches!(self, DocumentStyle::Plain)
    }

    pub fn has_toc(self) -> bool {
        !matches!(self, DocumentStyle::Plain)
    }

    pub fn has_page_chrome(self) -> bool {
        !matches!(self, DocumentStyle::Plain)
    }

    pub fn has_slide_labels(self) -> bool {
        matches!(self, DocumentStyle::Handout | DocumentStyle::SpeakerNotes)
    }

    pub fn has_notes_appendix(self) -> bool {
        matches!(self, DocumentStyle::SpeakerNotes)
    }
}

#[derive(Clone, Debug, Default)]
pub struct DocumentOptions {
    pub style: DocumentStyle,
}

impl DocumentOptions {
    pub fn new(style: DocumentStyle) -> Self {
        Self { style }
    }
}
