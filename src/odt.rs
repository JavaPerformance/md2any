//! OpenDocument Text (ODT) export.
//!
//! Flattens the slide deck into a continuous text document. Mapping:
//!   - Title slide       → title block (centered heading + subtitle/author/date)
//!   - Section slide     → page break, then Heading 1
//!   - Content slide     → Heading 2, then the slide's blocks as flowing text
//!
//! Reuses ODP's general approach: zip container, content.xml + styles.xml,
//! pictures embedded under `Pictures/`. We do *not* paginate ourselves; ODT
//! lets the consumer (LibreOffice / Word with import) handle pagination.

use crate::document::{DocumentOptions, DocumentStyle};
use crate::image::{self, ImageMeta};
use crate::ir::*;
use crate::theme::Theme;
use anyhow::Result;
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::FileOptions;
use zip::CompressionMethod;

const NS: &str = concat!(
    " xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"",
    " xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"",
    " xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"",
    " xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"",
    " xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"",
    " xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"",
    " xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\"",
    " xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
);

pub fn write(
    slides: &[Slide],
    theme: &Theme,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
) -> Result<Vec<u8>> {
    write_with_options(
        slides,
        theme,
        deck_title,
        author,
        base_dir,
        logo,
        direction,
        &DocumentOptions::default(),
    )
}

pub fn write_with_options(
    slides: &[Slide],
    theme: &Theme,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
    options: &DocumentOptions,
) -> Result<Vec<u8>> {
    let _ = logo;
    let rtl = direction
        .map(|s| s.eq_ignore_ascii_case("rtl"))
        .unwrap_or(false);
    let mut metas: Vec<ImageMeta> = Vec::new();
    let mut by_src: HashMap<String, usize> = HashMap::new();
    for slide in slides {
        collect_block_images(&slide.blocks, base_dir, &mut metas, &mut by_src)?;
    }

    let buf: Vec<u8> = Vec::new();
    let cursor = Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);
    let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/vnd.oasis.opendocument.text")?;

    write_file(
        &mut zip,
        "META-INF/manifest.xml",
        &build_manifest(&metas),
        deflated,
    )?;
    write_file(
        &mut zip,
        "meta.xml",
        &build_meta(deck_title, author),
        deflated,
    )?;
    write_file(
        &mut zip,
        "styles.xml",
        &build_styles(theme, rtl, deck_title, options),
        deflated,
    )?;
    let content = build_content(slides, theme, &by_src, options);
    let content = if rtl {
        apply_rtl_odt(&content)
    } else {
        content
    };
    write_file(&mut zip, "content.xml", &content, deflated)?;

    for (idx, m) in metas.iter().enumerate() {
        zip.start_file(&format!("Pictures/image{}.{}", idx + 1, m.ext), stored)?;
        zip.write_all(&m.bytes)?;
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

fn write_file<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    content: &str,
    opts: FileOptions,
) -> Result<()> {
    zip.start_file(name, opts)?;
    zip.write_all(content.as_bytes())?;
    Ok(())
}

fn collect_block_images(
    blocks: &[Block],
    base_dir: &Path,
    metas: &mut Vec<ImageMeta>,
    by_src: &mut HashMap<String, usize>,
) -> Result<()> {
    for b in blocks {
        match b {
            Block::Image { src, .. } => {
                if by_src.contains_key(src) {
                    continue;
                }
                let meta = image::load_any_or_placeholder(base_dir, src);
                by_src.insert(src.clone(), metas.len());
                metas.push(meta);
            }
            Block::Columns { left, right } => {
                collect_block_images(left, base_dir, metas, by_src)?;
                collect_block_images(right, base_dir, metas, by_src)?;
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Static parts
// ---------------------------------------------------------------------------

fn build_manifest(metas: &[ImageMeta]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>
"#,
    );
    for (i, m) in metas.iter().enumerate() {
        let mime = match m.ext {
            "jpeg" => "image/jpeg",
            _ => "image/png",
        };
        s.push_str(&format!(
            r#"<manifest:file-entry manifest:full-path="Pictures/image{}.{}" manifest:media-type="{}"/>
"#,
            i + 1,
            m.ext,
            mime,
        ));
    }
    s.push_str("</manifest:manifest>");
    s
}

fn build_meta(title: &str, author: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" office:version="1.2">
<office:meta>
<dc:title>{title}</dc:title>
<dc:creator>{author}</dc:creator>
<meta:generator>md2any</meta:generator>
</office:meta>
</office:document-meta>"#,
        title = escape_xml(title),
        author = escape_xml(author),
    )
}

fn build_styles(theme: &Theme, rtl: bool, deck_title: &str, options: &DocumentOptions) -> String {
    // ODT's default paragraph style determines reading direction for any
    // paragraph that doesn't override it. Setting `writing-mode=rl-tb`
    // here is the cleanest way to flip the whole document for RTL.
    let writing_mode = if rtl { "rl-tb" } else { "page" };
    let default_align = if rtl { r#"fo:text-align="end" "# } else { "" };
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles{ns} office:version="1.2">
<office:styles>
<style:default-style style:family="paragraph">
<style:paragraph-properties fo:hyphenation-ladder-count="no-limit" style:writing-mode="{writing_mode}"/>
<style:text-properties style:font-name="{body_font}" fo:font-size="11pt" fo:color="#{body_color}" fo:language="en"/>
</style:default-style>

<style:style style:name="Standard" style:family="paragraph" style:class="text">
<style:paragraph-properties {default_align}fo:line-height="135%" fo:margin-top="0.08in" fo:margin-bottom="0.08in"/>
</style:style>

<style:style style:name="Title" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:text-align="center" fo:margin-top="0.5in" fo:margin-bottom="0.2in"/>
<style:text-properties style:font-name="{title_font}" fo:font-size="32pt" fo:font-weight="bold" fo:color="#{title_color}"/>
</style:style>

<style:style style:name="Subtitle" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:text-align="center" fo:margin-bottom="0.4in"/>
<style:text-properties fo:font-size="16pt" fo:color="#{muted_color}" fo:font-style="italic"/>
</style:style>

<style:style style:name="DocMeta" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:text-align="center" fo:margin-top="0.04in" fo:margin-bottom="0.04in"/>
<style:text-properties fo:font-size="9pt" fo:color="#{muted_color}"/>
</style:style>

<style:style style:name="SlideLabel" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:margin-top="0.18in" fo:margin-bottom="0.04in"/>
<style:text-properties style:font-name="{mono_font}" fo:font-size="9pt" fo:font-weight="bold" fo:text-transform="uppercase" fo:color="#{accent}"/>
</style:style>

<style:style style:name="Caption" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:text-align="center" fo:margin-top="0.02in" fo:margin-bottom="0.12in"/>
<style:text-properties fo:font-size="9pt" fo:font-style="italic" fo:color="#{muted_color}"/>
</style:style>

<style:style style:name="NotesBody" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:background-color="#{accent_soft}" fo:padding="0.07in 0.12in" fo:margin-top="0.06in" fo:margin-bottom="0.08in"/>
<style:text-properties fo:font-size="10.5pt"/>
</style:style>

<style:style style:name="TocTitle" style:family="paragraph" style:parent-style-name="Heading_20_1">
<style:paragraph-properties fo:break-before="auto" fo:margin-top="0.2in" fo:margin-bottom="0.15in"/>
</style:style>

<style:style style:name="TocEntry" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:margin-left="0.22in" fo:margin-top="0.03in" fo:margin-bottom="0.03in"/>
</style:style>

<style:style style:name="PageBreak" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:break-after="page"/>
</style:style>

<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="Standard" style:default-outline-level="1">
<style:paragraph-properties fo:margin-top="0.4in" fo:margin-bottom="0.15in" fo:break-before="page"/>
<style:text-properties style:font-name="{title_font}" fo:font-size="22pt" fo:font-weight="bold" fo:color="#{title_color}"/>
</style:style>

<style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="Standard" style:default-outline-level="2">
<style:paragraph-properties fo:margin-top="0.25in" fo:margin-bottom="0.1in"/>
<style:text-properties style:font-name="{title_font}" fo:font-size="16pt" fo:font-weight="bold" fo:color="#{title_color}"/>
</style:style>

<style:style style:name="Heading_20_3" style:display-name="Heading 3" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="Standard" style:default-outline-level="3">
<style:paragraph-properties fo:margin-top="0.18in" fo:margin-bottom="0.08in"/>
<style:text-properties fo:font-size="13pt" fo:font-weight="bold" fo:color="#{title_color}"/>
</style:style>

<style:style style:name="Code" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:background-color="#{code_bg}" fo:padding="0.05in 0.1in" fo:margin-top="0.05in" fo:margin-bottom="0.05in"/>
<style:text-properties style:font-name="{mono_font}" fo:font-size="10pt" fo:color="#{code_text}"/>
</style:style>

<style:style style:name="Quote" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:margin-left="0.4in" fo:border-left="0.06in solid #{accent}" fo:padding-left="0.15in"/>
<style:text-properties fo:font-style="italic" fo:color="#{muted_color}"/>
</style:style>

<style:style style:name="ListBullet" style:family="paragraph" style:parent-style-name="Standard" style:list-style-name="LSBullet"/>
<style:style style:name="ListNumber" style:family="paragraph" style:parent-style-name="Standard" style:list-style-name="LSNumber"/>

<text:list-style style:name="LSBullet">
<text:list-level-style-bullet text:level="1" text:bullet-char="•">
<style:list-level-properties text:space-before="0.25in" text:min-label-width="0.25in"/>
</text:list-level-style-bullet>
<text:list-level-style-bullet text:level="2" text:bullet-char="◦">
<style:list-level-properties text:space-before="0.5in" text:min-label-width="0.25in"/>
</text:list-level-style-bullet>
<text:list-level-style-bullet text:level="3" text:bullet-char="▪">
<style:list-level-properties text:space-before="0.75in" text:min-label-width="0.25in"/>
</text:list-level-style-bullet>
</text:list-style>

<text:list-style style:name="LSNumber">
<text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix=".">
<style:list-level-properties text:space-before="0.25in" text:min-label-width="0.3in"/>
</text:list-level-style-number>
<text:list-level-style-number text:level="2" style:num-format="1" style:num-suffix=".">
<style:list-level-properties text:space-before="0.55in" text:min-label-width="0.3in"/>
</text:list-level-style-number>
</text:list-style>

</office:styles>
<office:automatic-styles>
<style:style style:name="TableDef" style:family="table">
<style:table-properties style:width="6.5in" table:align="left"/>
</style:style>
<style:style style:name="TableCol" style:family="table-column">
<style:table-column-properties style:column-width="1.5in"/>
</style:style>
<style:style style:name="TableCellHead" style:family="table-cell">
<style:table-cell-properties fo:background-color="#{accent_soft}" fo:padding="0.05in" fo:border="0.005in solid #{divider}"/>
</style:style>
<style:style style:name="TableCellBody" style:family="table-cell">
<style:table-cell-properties fo:padding="0.05in" fo:border="0.005in solid #{divider}"/>
</style:style>
<style:style style:name="TableHeadText" style:family="paragraph" style:parent-style-name="Standard">
<style:text-properties fo:font-weight="bold" fo:color="#{title_color}"/>
</style:style>
<style:page-layout style:name="PL1">
<style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1in" fo:margin-right="1in"/>
</style:page-layout>
</office:automatic-styles>
<office:master-styles>
{master_page}
</office:master-styles>
</office:document-styles>"##,
        ns = NS,
        body_font = escape_xml(&theme.body_font),
        title_font = escape_xml(&theme.title_font),
        mono_font = escape_xml(&theme.mono_font),
        title_color = theme.title_color,
        body_color = theme.body_color,
        muted_color = theme.muted_color,
        code_bg = theme.code_bg,
        code_text = theme.code_text,
        accent = theme.accent,
        accent_soft = theme.accent_soft,
        divider = theme.divider,
        master_page = odt_master_page(deck_title, options.style),
    )
}

fn odt_master_page(deck_title: &str, style: DocumentStyle) -> String {
    if !style.has_page_chrome() {
        return r#"<style:master-page style:name="Standard" style:page-layout-name="PL1"/>"#
            .to_string();
    }
    format!(
        r#"<style:master-page style:name="Standard" style:page-layout-name="PL1">
<style:header><text:p text:style-name="DocMeta">{}</text:p></style:header>
<style:footer><text:p text:style-name="DocMeta">{} · <text:page-number text:select-page="current">1</text:page-number></text:p></style:footer>
</style:master-page>"#,
        escape_xml(deck_title),
        escape_xml(style.name()),
    )
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

fn build_content(
    slides: &[Slide],
    theme: &Theme,
    by_src: &HashMap<String, usize>,
    options: &DocumentOptions,
) -> String {
    let mut body = String::new();
    let mut first_section_emitted = false;
    let mut last_section_title: Option<String> = None;

    for (idx, slide) in slides.iter().enumerate() {
        match &slide.kind {
            SlideKind::Title {
                subtitle,
                author,
                date,
            } => {
                render_title_page(
                    &mut body,
                    &slide.title,
                    subtitle,
                    author,
                    date,
                    options.style,
                );
                if options.style.has_toc() {
                    body.push_str(&odt_page_break());
                    body.push_str(&odt_toc(slides));
                }
                if options.style.has_title_page() {
                    body.push_str(&odt_page_break());
                }
            }
            SlideKind::Section => {
                let style = if first_section_emitted {
                    "Heading_20_1"
                } else {
                    "Heading_20_1_first"
                };
                first_section_emitted = true;
                if options.style.has_slide_labels() {
                    body.push_str(&odt_slide_label(idx + 1, &slide.title));
                }
                body.push_str(&format!(
                    r#"<text:h text:style-name="{}" text:outline-level="1">{}</text:h>"#,
                    style,
                    escape_xml(&slide.title),
                ));
                last_section_title = Some(slide.title.clone());
                render_blocks(&mut body, &slide.blocks, theme, by_src);
            }
            SlideKind::Content => {
                let title = trim_cont_suffix(&slide.title);
                // Skip the H2 if the content slide was auto-spawned from the
                // section above (same title). This is the common case in
                // slide decks: `# Section` followed by content paragraphs.
                let skip_heading = last_section_title
                    .as_deref()
                    .map(|s| trim_cont_suffix(s) == title)
                    .unwrap_or(false);
                if options.style.has_slide_labels() {
                    body.push_str(&odt_slide_label(idx + 1, title));
                }
                if !skip_heading {
                    body.push_str(&format!(
                        r#"<text:h text:style-name="Heading_20_2" text:outline-level="2">{}</text:h>"#,
                        escape_xml(title),
                    ));
                }
                last_section_title = None;
                render_blocks(&mut body, &slide.blocks, theme, by_src);
            }
        }
    }
    if options.style.has_notes_appendix() {
        append_notes_appendix(&mut body, slides);
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content{ns} office:version="1.2">
<office:automatic-styles>
<style:style style:name="Heading_20_1_first" style:family="paragraph" style:parent-style-name="Heading_20_1">
<style:paragraph-properties fo:break-before="auto"/>
</style:style>
{runs}
</office:automatic-styles>
<office:body>
<office:text>
{body}
</office:text>
</office:body>
</office:document-content>"#,
        ns = NS,
        runs = RUN_STYLES_XML,
        body = body,
    )
}

fn render_title_page(
    body: &mut String,
    title: &str,
    subtitle: &Option<String>,
    author: &Option<String>,
    date: &Option<String>,
    style: DocumentStyle,
) {
    body.push_str(&format!(
        r#"<text:p text:style-name="Title">{}</text:p>"#,
        escape_xml(title),
    ));
    if let Some(s) = subtitle {
        body.push_str(&format!(
            r#"<text:p text:style-name="Subtitle">{}</text:p>"#,
            escape_xml(s),
        ));
    }
    if let (Some(a), Some(d)) = (author.as_ref(), date.as_ref()) {
        body.push_str(&format!(
            r#"<text:p text:style-name="Subtitle">{} · {}</text:p>"#,
            escape_xml(a),
            escape_xml(d),
        ));
    } else if let Some(a) = author {
        body.push_str(&format!(
            r#"<text:p text:style-name="Subtitle">{}</text:p>"#,
            escape_xml(a),
        ));
    } else if let Some(d) = date {
        body.push_str(&format!(
            r#"<text:p text:style-name="Subtitle">{}</text:p>"#,
            escape_xml(d),
        ));
    }
    if style.has_title_page() {
        body.push_str(&format!(
            r#"<text:p text:style-name="DocMeta">Document style: {}</text:p>"#,
            escape_xml(style.name()),
        ));
    }
}

fn odt_toc(slides: &[Slide]) -> String {
    let mut out = String::new();
    out.push_str(r#"<text:h text:style-name="TocTitle" text:outline-level="1">Contents</text:h>"#);
    for (idx, slide) in slides.iter().enumerate() {
        if matches!(slide.kind, SlideKind::Title { .. }) {
            continue;
        }
        let title = trim_cont_suffix(&slide.title);
        if title.is_empty() {
            continue;
        }
        out.push_str(&format!(
            r#"<text:p text:style-name="TocEntry">{}  {}</text:p>"#,
            idx + 1,
            escape_xml(title),
        ));
    }
    out
}

fn odt_slide_label(idx: usize, title: &str) -> String {
    format!(
        r#"<text:p text:style-name="SlideLabel">Slide {} · {}</text:p>"#,
        idx,
        escape_xml(trim_cont_suffix(title)),
    )
}

fn append_notes_appendix(body: &mut String, slides: &[Slide]) {
    body.push_str(&odt_page_break());
    body.push_str(
        r#"<text:h text:style-name="Heading_20_1_first" text:outline-level="1">Speaker notes</text:h>"#,
    );
    let mut count = 0usize;
    for (idx, slide) in slides.iter().enumerate() {
        let Some(notes) = slide.notes.as_deref() else {
            continue;
        };
        count += 1;
        body.push_str(&format!(
            r#"<text:h text:style-name="Heading_20_2" text:outline-level="2">Slide {}: {}</text:h>"#,
            idx + 1,
            escape_xml(trim_cont_suffix(&slide.title)),
        ));
        for line in notes.lines() {
            if line.trim().is_empty() {
                continue;
            }
            body.push_str(&format!(
                r#"<text:p text:style-name="NotesBody">{}</text:p>"#,
                escape_xml(line.trim()),
            ));
        }
    }
    if count == 0 {
        body.push_str(
            r#"<text:p text:style-name="NotesBody">No speaker notes in this deck.</text:p>"#,
        );
    }
}

fn odt_page_break() -> String {
    r#"<text:p text:style-name="PageBreak"/>"#.to_string()
}

/// Flip ODF paragraph alignment for RTL: `start` becomes `end`, `left`
/// becomes `right`. Most ODF readers infer the reading direction from this
/// plus the script of the text itself.
fn apply_rtl_odt(xml: &str) -> String {
    xml.replace(r#"fo:text-align="start""#, r#"fo:text-align="end""#)
        .replace(r#"fo:text-align="left""#, r#"fo:text-align="right""#)
        .replace("text-align:start", "text-align:end")
        .replace("text-align:left", "text-align:right")
}

fn render_blocks(
    out: &mut String,
    blocks: &[Block],
    theme: &Theme,
    by_src: &HashMap<String, usize>,
) {
    for b in blocks {
        render_block(out, b, theme, by_src);
    }
}

fn render_block(out: &mut String, b: &Block, theme: &Theme, by_src: &HashMap<String, usize>) {
    match b {
        Block::Paragraph(runs) => {
            out.push_str(&format!(
                r#"<text:p text:style-name="Standard">{}</text:p>"#,
                inline_runs(runs, theme),
            ));
        }
        Block::Heading { level, runs } => {
            let lvl = (*level).clamp(3, 6) as i32;
            let style = format!("Heading_20_{}", lvl);
            out.push_str(&format!(
                r#"<text:h text:style-name="{}" text:outline-level="{}">{}</text:h>"#,
                style,
                lvl,
                inline_runs(runs, theme),
            ));
        }
        Block::List(items) => render_list(out, items, theme),
        Block::CodeBlock {
            lang, title, lines, ..
        } => {
            if let Some(t) = title {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Standard"><text:span text:style-name="MetaLabel">{} {}</text:span></text:p>"#,
                    lang.as_deref().map(|l| format!("[{}]", l)).unwrap_or_default(),
                    escape_xml(t),
                ));
            } else if let Some(l) = lang {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Standard"><text:span text:style-name="MetaLabel">{}</text:span></text:p>"#,
                    escape_xml(l),
                ));
            }
            for line in lines {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Code">{}</text:p>"#,
                    if line.is_empty() {
                        String::new()
                    } else {
                        escape_xml_preserve_spaces(line)
                    },
                ));
            }
        }
        Block::Quote(paras) | Block::Callout { body: paras, .. } => {
            for para in paras {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Quote">{}</text:p>"#,
                    inline_runs(para, theme),
                ));
            }
        }
        Block::Table { headers, rows, .. } => render_table(out, headers, rows, theme),
        Block::Columns { left, right } => {
            // Documents flow, so collapse columns into back-to-back blocks.
            render_blocks(out, left, theme, by_src);
            render_blocks(out, right, theme, by_src);
        }
        Block::Cards { cards, .. } => {
            for cb in &crate::ir::cards_as_blocks(cards) {
                render_block(out, cb, theme, by_src);
            }
        }
        Block::ColumnBreak => {}
        Block::Image {
            src,
            alt,
            width_pct: _,
        } => {
            render_image(out, src, alt, by_src);
        }
        Block::Footnotes(items) => {
            out.push_str(
                r#"<text:p text:style-name="Standard"><text:span text:style-name="MetaLabel">Notes</text:span></text:p>"#,
            );
            for item in items {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Standard">{}</text:p>"#,
                    inline_runs(&item.runs, theme),
                ));
            }
        }
    }
}

fn render_list(out: &mut String, items: &[ListItem], theme: &Theme) {
    if items.is_empty() {
        return;
    }
    let max_level = items.iter().map(|i| i.level).max().unwrap_or(0);

    // ODF carries bullet vs. number on the <text:list> style, not on the
    // item — so a mixed-ordering list needs to close and re-open the
    // wrapper when the flag flips. Track both the current depth and the
    // ordered flag at each depth so the right wrapper exists for each item.
    let style_pair = |ordered: bool| -> (&'static str, &'static str) {
        if ordered {
            ("LSNumber", "ListNumber")
        } else {
            ("LSBullet", "ListBullet")
        }
    };

    let (root_list, _) = style_pair(items[0].ordered);
    let mut stack: Vec<&'static str> = vec![root_list];
    out.push_str(&format!(r#"<text:list text:style-name="{}">"#, root_list));

    let mut current_level: u8 = 0;
    for item in items {
        let lvl = item.level.min(max_level);
        let (list_style, para_style) = style_pair(item.ordered);

        // Pop nested lists until we're at or above the item's level.
        while current_level > lvl {
            out.push_str("</text:list></text:list-item>");
            stack.pop();
            current_level -= 1;
        }
        // If we're at the right level but the ordering flipped at this
        // depth, close and re-open the wrapper to switch styles.
        if current_level == lvl && stack.last() != Some(&list_style) {
            if current_level == 0 {
                out.push_str("</text:list>");
                out.push_str(&format!(r#"<text:list text:style-name="{}">"#, list_style));
                stack[0] = list_style;
            } else {
                out.push_str("</text:list></text:list-item>");
                out.push_str(&format!(
                    r#"<text:list-item><text:list text:style-name="{}">"#,
                    list_style
                ));
                if let Some(slot) = stack.last_mut() {
                    *slot = list_style;
                }
            }
        }
        // Open nested lists down to the item's level, picking up the
        // item's style on the way in.
        while current_level < lvl {
            out.push_str(&format!(
                r#"<text:list-item><text:list text:style-name="{}">"#,
                list_style
            ));
            stack.push(list_style);
            current_level += 1;
        }
        out.push_str(&format!(
            r#"<text:list-item><text:p text:style-name="{}">{}</text:p></text:list-item>"#,
            para_style,
            inline_runs(&item.runs, theme),
        ));
    }
    while current_level > 0 {
        out.push_str("</text:list></text:list-item>");
        current_level -= 1;
    }
    out.push_str("</text:list>");
}

fn render_table(out: &mut String, headers: &[Vec<Run>], rows: &[Vec<Vec<Run>>], theme: &Theme) {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if cols == 0 {
        return;
    }
    out.push_str(r#"<table:table table:style-name="TableDef">"#);
    for _ in 0..cols {
        out.push_str(r#"<table:table-column table:style-name="TableCol"/>"#);
    }
    if !headers.is_empty() {
        out.push_str("<table:table-row>");
        for c in headers
            .iter()
            .chain(std::iter::repeat(&Vec::new()))
            .take(cols)
        {
            out.push_str(&format!(
                r#"<table:table-cell table:style-name="TableCellHead"><text:p text:style-name="TableHeadText">{}</text:p></table:table-cell>"#,
                inline_runs(c, theme),
            ));
        }
        out.push_str("</table:table-row>");
    }
    for row in rows {
        out.push_str("<table:table-row>");
        for c in row.iter().chain(std::iter::repeat(&Vec::new())).take(cols) {
            out.push_str(&format!(
                r#"<table:table-cell table:style-name="TableCellBody"><text:p text:style-name="Standard">{}</text:p></table:table-cell>"#,
                inline_runs(c, theme),
            ));
        }
        out.push_str("</table:table-row>");
    }
    out.push_str("</table:table>");
}

fn render_image(out: &mut String, src: &str, alt: &str, by_src: &HashMap<String, usize>) {
    let Some(&idx) = by_src.get(src) else { return };
    let href = format!("Pictures/image{}", idx + 1);
    let caption_alt = crate::math::visible_image_alt(src, alt);
    out.push_str(&format!(
        r#"<text:p text:style-name="Standard"><draw:frame text:anchor-type="paragraph" svg:width="6in" svg:height="auto" draw:z-index="0"><draw:image xlink:href="{}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame></text:p>"#,
        escape_xml(&href),
    ));
    if !caption_alt.is_empty() {
        out.push_str(&format!(
            r#"<text:p text:style-name="Caption">{}</text:p>"#,
            escape_xml(caption_alt),
        ));
    }
}

// ---------------------------------------------------------------------------
// Inline runs
// ---------------------------------------------------------------------------

fn inline_runs(runs: &[Run], _theme: &Theme) -> String {
    let mut s = String::new();
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        let style = inline_style_name(r);
        let body = expand_breaks(&escape_xml_preserve_spaces(&r.text));
        let span = format!(
            r#"<text:span text:style-name="{}">{}</text:span>"#,
            style, body,
        );
        if let Some(url) = &r.link {
            s.push_str(&format!(
                r#"<text:a xlink:type="simple" xlink:href="{}">{}</text:a>"#,
                escape_xml(url),
                span,
            ));
        } else {
            s.push_str(&span);
        }
    }
    s
}

/// Pick a pre-registered run style based on the run's formatting flags.
/// The actual style definitions live in the automatic-styles section of
/// content.xml (added below in `register_styles`); we use the same names.
fn inline_style_name(r: &Run) -> &'static str {
    let bold = r.bold;
    let italic = r.italic;
    let code = r.code;
    let strike = r.strike;
    match (bold, italic, code, strike) {
        (false, false, false, false) => "RunPlain",
        (true, false, false, false) => "RunBold",
        (false, true, false, false) => "RunItalic",
        (true, true, false, false) => "RunBoldItalic",
        (_, _, true, _) => "RunCode",
        (_, _, _, true) => "RunStrike",
    }
}

// We bake the inline span styles into a small block we always prepend to
// content.xml's automatic-styles section.
const RUN_STYLES_XML: &str = r##"
<style:style style:name="RunPlain" style:family="text"/>
<style:style style:name="RunBold" style:family="text">
<style:text-properties fo:font-weight="bold"/>
</style:style>
<style:style style:name="RunItalic" style:family="text">
<style:text-properties fo:font-style="italic"/>
</style:style>
<style:style style:name="RunBoldItalic" style:family="text">
<style:text-properties fo:font-weight="bold" fo:font-style="italic"/>
</style:style>
<style:style style:name="RunCode" style:family="text">
<style:text-properties style:font-name="Consolas" fo:font-size="10pt" fo:background-color="#F1F5F9" fo:color="#0369A1"/>
</style:style>
<style:style style:name="RunStrike" style:family="text">
<style:text-properties style:text-line-through-style="solid"/>
</style:style>
<style:style style:name="MetaLabel" style:family="text">
<style:text-properties fo:font-size="9pt" fo:color="#64748B" style:font-name="Consolas"/>
</style:style>
"##;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn escape_xml_preserve_spaces(s: &str) -> String {
    // Replace runs of 2+ spaces with <text:s text:c="N"/> to preserve indentation.
    let escaped = escape_xml(s);
    let mut out = String::with_capacity(escaped.len());
    let mut iter = escaped.chars().peekable();
    while let Some(c) = iter.next() {
        if c == ' ' && iter.peek() == Some(&' ') {
            let mut count = 1;
            while iter.peek() == Some(&' ') {
                iter.next();
                count += 1;
            }
            out.push(' ');
            if count > 1 {
                out.push_str(&format!(r#"<text:s text:c="{}"/>"#, count - 1));
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn expand_breaks(s: &str) -> String {
    s.replace('\t', r#"<text:tab/>"#)
}

fn trim_cont_suffix(s: &str) -> &str {
    s.strip_suffix(" (cont.)").unwrap_or(s)
}
