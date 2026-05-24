//! Microsoft Word DOCX export.
//!
//! Mirrors the ODT writer: walks the slide deck and produces a flowing
//! document. OOXML container holds:
//!   - `[Content_Types].xml`
//!   - `_rels/.rels` and `word/_rels/document.xml.rels`
//!   - `docProps/app.xml`, `docProps/core.xml`
//!   - `word/styles.xml`, `word/numbering.xml`, `word/settings.xml`
//!   - `word/document.xml` (the actual content)
//!   - `word/media/imageN.png` (embedded images)
//!
//! Slide → document mapping is the same as ODT: title slide produces a
//! title block; section H1 starts a new page; content slide → H2 + flow.

use crate::image::{self, ImageMeta};
use crate::ir::*;
use crate::theme::Theme;
use anyhow::Result;
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::FileOptions;
use zip::CompressionMethod;

const REL_HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_NUMBERING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const REL_SETTINGS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
const REL_OFFICE_DOC: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_CORE_PROPS: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const REL_APP_PROPS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";

pub fn write(
    slides: &[Slide],
    theme: &Theme,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
) -> Result<Vec<u8>> {
    let _ = logo;
    let rtl = direction
        .map(|s| s.eq_ignore_ascii_case("rtl"))
        .unwrap_or(false);

    // Collect images up front so we know what to embed and how to number them.
    let mut metas: Vec<ImageMeta> = Vec::new();
    let mut by_src: HashMap<String, usize> = HashMap::new();
    for slide in slides {
        collect_block_images(&slide.blocks, base_dir, &mut metas, &mut by_src)?;
    }

    let mut rels = DocRels::new();
    // Reserve rel ids for styles, numbering, settings (these are fixed positions
    // we reference from word/_rels/document.xml.rels).
    let _r_styles = rels.add(REL_STYLES, "styles.xml", false);
    let _r_numbering = rels.add(REL_NUMBERING, "numbering.xml", false);
    let _r_settings = rels.add(REL_SETTINGS, "settings.xml", false);
    let image_rels: Vec<String> = metas
        .iter()
        .enumerate()
        .map(|(i, m)| rels.add(REL_IMAGE, &format!("media/image{}.{}", i + 1, m.ext), false))
        .collect();

    let mut body = build_body(slides, theme, &by_src, &image_rels, &mut rels);
    if rtl {
        body = apply_rtl_docx(&body);
    }

    let buf: Vec<u8> = Vec::new();
    let cursor = Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);
    let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);

    write_file(
        &mut zip,
        "[Content_Types].xml",
        &content_types(&metas),
        stored,
    )?;
    write_file(&mut zip, "_rels/.rels", &root_rels(), stored)?;
    write_file(
        &mut zip,
        "docProps/app.xml",
        &app_xml(slides.len()),
        deflated,
    )?;
    write_file(
        &mut zip,
        "docProps/core.xml",
        &core_xml(deck_title, author),
        deflated,
    )?;
    write_file(
        &mut zip,
        "word/_rels/document.xml.rels",
        &rels.serialize(),
        stored,
    )?;
    write_file(&mut zip, "word/document.xml", &body, deflated)?;
    write_file(&mut zip, "word/styles.xml", &styles_xml(theme), deflated)?;
    write_file(&mut zip, "word/numbering.xml", &numbering_xml(), deflated)?;
    write_file(&mut zip, "word/settings.xml", &settings_xml(), deflated)?;

    for (i, m) in metas.iter().enumerate() {
        zip.start_file(&format!("word/media/image{}.{}", i + 1, m.ext), stored)?;
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
// Relationship table
// ---------------------------------------------------------------------------

struct DocRels {
    entries: Vec<(String, &'static str, String, bool)>,
    next: usize,
}

impl DocRels {
    fn new() -> Self {
        DocRels {
            entries: Vec::new(),
            next: 1,
        }
    }
    fn add(&mut self, rel_type: &'static str, target: &str, external: bool) -> String {
        let id = format!("rId{}", self.next);
        self.next += 1;
        self.entries
            .push((id.clone(), rel_type, target.to_string(), external));
        id
    }
    fn serialize(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
        );
        for (id, ty, target, external) in &self.entries {
            if *external {
                s.push_str(&format!(
                    "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\" TargetMode=\"External\"/>\n",
                    id,
                    ty,
                    escape_xml(target),
                ));
            } else {
                s.push_str(&format!(
                    "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"/>\n",
                    id,
                    ty,
                    escape_xml(target),
                ));
            }
        }
        s.push_str("</Relationships>");
        s
    }
}

// ---------------------------------------------------------------------------
// Static parts
// ---------------------------------------------------------------------------

fn content_types(metas: &[ImageMeta]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
"#,
    );
    let mut exts: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for m in metas {
        exts.insert(m.ext);
    }
    for ext in &exts {
        let mime = match *ext {
            "jpeg" => "image/jpeg",
            _ => "image/png",
        };
        s.push_str(&format!(
            r#"<Default Extension="{}" ContentType="{}"/>
"#,
            ext, mime,
        ));
    }
    s.push_str(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>"#,
    );
    s
}

fn root_rels() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="{}" Target="word/document.xml"/>
<Relationship Id="rId2" Type="{}" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="{}" Target="docProps/app.xml"/>
</Relationships>"#,
        REL_OFFICE_DOC, REL_CORE_PROPS, REL_APP_PROPS,
    )
}

fn app_xml(slide_count: usize) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">
<Application>md2any</Application>
<Pages>{}</Pages>
</Properties>"#,
        slide_count.max(1),
    )
}

fn core_xml(title: &str, author: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>{}</dc:title>
<dc:creator>{}</dc:creator>
</cp:coreProperties>"#,
        escape_xml(title),
        escape_xml(author),
    )
}

fn settings_xml() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:zoom w:percent="100"/>
<w:defaultTabStop w:val="720"/>
<w:characterSpacingControl w:val="doNotCompress"/>
</w:settings>"#,
    )
}

fn numbering_xml() -> String {
    // Two abstract num definitions: 0 = bullets (4 levels), 1 = decimal.
    let mut bullets = String::new();
    let bullet_chars = ["", "", "", ""];
    let bullet_glyphs = ['•', '◦', '▪', '▫'];
    for i in 0..4u32 {
        bullets.push_str(&format!(
            r#"<w:lvl w:ilvl="{i}">
<w:start w:val="1"/>
<w:numFmt w:val="bullet"/>
<w:lvlText w:val="{ch}"/>
<w:lvlJc w:val="left"/>
<w:pPr><w:ind w:left="{left}" w:hanging="360"/></w:pPr>
<w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol" w:hint="default"/></w:rPr>
</w:lvl>
"#,
            i = i,
            ch = bullet_glyphs[i as usize],
            left = 720 + i * 360,
        ));
        let _ = bullet_chars;
    }
    let mut numbers = String::new();
    for i in 0..4u32 {
        numbers.push_str(&format!(
            r#"<w:lvl w:ilvl="{i}">
<w:start w:val="1"/>
<w:numFmt w:val="decimal"/>
<w:lvlText w:val="%{n}."/>
<w:lvlJc w:val="left"/>
<w:pPr><w:ind w:left="{left}" w:hanging="360"/></w:pPr>
</w:lvl>
"#,
            i = i,
            n = i + 1,
            left = 720 + i * 360,
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:abstractNum w:abstractNumId="0">
{bullets}
</w:abstractNum>
<w:abstractNum w:abstractNumId="1">
{numbers}
</w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
</w:numbering>"#,
        bullets = bullets,
        numbers = numbers,
    )
}

fn styles_xml(theme: &Theme) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">

<w:docDefaults>
<w:rPrDefault>
<w:rPr>
<w:rFonts w:ascii="{body_font}" w:hAnsi="{body_font}"/>
<w:sz w:val="22"/>
<w:color w:val="{body_color}"/>
</w:rPr>
</w:rPrDefault>
<w:pPrDefault>
<w:pPr><w:spacing w:before="80" w:after="80" w:line="288" w:lineRule="auto"/></w:pPr>
</w:pPrDefault>
</w:docDefaults>

<w:style w:type="paragraph" w:default="1" w:styleId="Normal">
<w:name w:val="Normal"/>
</w:style>

<w:style w:type="paragraph" w:styleId="Title">
<w:name w:val="Title"/>
<w:basedOn w:val="Normal"/>
<w:pPr>
<w:spacing w:before="720" w:after="200"/>
<w:jc w:val="center"/>
</w:pPr>
<w:rPr>
<w:rFonts w:ascii="{title_font}" w:hAnsi="{title_font}"/>
<w:b/><w:sz w:val="64"/>
<w:color w:val="{title_color}"/>
</w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="Subtitle">
<w:name w:val="Subtitle"/>
<w:basedOn w:val="Normal"/>
<w:pPr>
<w:spacing w:before="120" w:after="240"/>
<w:jc w:val="center"/>
</w:pPr>
<w:rPr><w:i/><w:sz w:val="28"/><w:color w:val="{muted_color}"/></w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="Heading1">
<w:name w:val="heading 1"/>
<w:basedOn w:val="Normal"/>
<w:next w:val="Normal"/>
<w:pPr>
<w:pageBreakBefore/>
<w:spacing w:before="240" w:after="200"/>
<w:outlineLvl w:val="0"/>
</w:pPr>
<w:rPr>
<w:rFonts w:ascii="{title_font}" w:hAnsi="{title_font}"/>
<w:b/><w:sz w:val="44"/>
<w:color w:val="{title_color}"/>
</w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="Heading2">
<w:name w:val="heading 2"/>
<w:basedOn w:val="Normal"/>
<w:next w:val="Normal"/>
<w:pPr>
<w:spacing w:before="320" w:after="120"/>
<w:outlineLvl w:val="1"/>
</w:pPr>
<w:rPr>
<w:rFonts w:ascii="{title_font}" w:hAnsi="{title_font}"/>
<w:b/><w:sz w:val="32"/>
<w:color w:val="{title_color}"/>
</w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="Heading3">
<w:name w:val="heading 3"/>
<w:basedOn w:val="Normal"/>
<w:next w:val="Normal"/>
<w:pPr>
<w:spacing w:before="200" w:after="100"/>
<w:outlineLvl w:val="2"/>
</w:pPr>
<w:rPr><w:b/><w:sz w:val="26"/><w:color w:val="{title_color}"/></w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="CodeBlock">
<w:name w:val="Code"/>
<w:basedOn w:val="Normal"/>
<w:pPr>
<w:spacing w:before="40" w:after="40" w:line="240" w:lineRule="auto"/>
<w:shd w:val="clear" w:color="auto" w:fill="{code_bg}"/>
<w:ind w:left="200" w:right="200"/>
</w:pPr>
<w:rPr>
<w:rFonts w:ascii="{mono_font}" w:hAnsi="{mono_font}" w:cs="{mono_font}"/>
<w:sz w:val="20"/>
<w:color w:val="{code_text}"/>
</w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="Quote">
<w:name w:val="Quote"/>
<w:basedOn w:val="Normal"/>
<w:pPr>
<w:ind w:left="600"/>
<w:pBdr><w:left w:val="single" w:sz="18" w:space="8" w:color="{accent}"/></w:pBdr>
</w:pPr>
<w:rPr><w:i/><w:color w:val="{muted_color}"/></w:rPr>
</w:style>

<w:style w:type="paragraph" w:styleId="ListParagraph">
<w:name w:val="List Paragraph"/>
<w:basedOn w:val="Normal"/>
<w:pPr>
<w:spacing w:before="40" w:after="40"/>
<w:ind w:left="720"/>
<w:contextualSpacing/>
</w:pPr>
</w:style>

<w:style w:type="character" w:styleId="Hyperlink">
<w:name w:val="Hyperlink"/>
<w:rPr>
<w:color w:val="{link}"/>
<w:u w:val="single"/>
</w:rPr>
</w:style>

<w:style w:type="character" w:styleId="InlineCode">
<w:name w:val="InlineCode"/>
<w:rPr>
<w:rFonts w:ascii="{mono_font}" w:hAnsi="{mono_font}"/>
<w:color w:val="{code_accent}"/>
<w:shd w:val="clear" w:color="auto" w:fill="{code_bg}"/>
</w:rPr>
</w:style>

<w:style w:type="character" w:styleId="MetaLabel">
<w:name w:val="MetaLabel"/>
<w:rPr>
<w:rFonts w:ascii="{mono_font}" w:hAnsi="{mono_font}"/>
<w:sz w:val="18"/>
<w:color w:val="{muted_color}"/>
</w:rPr>
</w:style>

</w:styles>"#,
        body_font = escape_xml(&theme.body_font),
        title_font = escape_xml(&theme.title_font),
        mono_font = escape_xml(&theme.mono_font),
        title_color = theme.title_color,
        body_color = theme.body_color,
        muted_color = theme.muted_color,
        code_bg = theme.code_bg,
        code_text = theme.code_text,
        code_accent = theme.code_accent,
        accent = theme.accent,
        link = theme.link,
    )
}

// ---------------------------------------------------------------------------
// Document body
// ---------------------------------------------------------------------------

fn build_body(
    slides: &[Slide],
    theme: &Theme,
    by_src: &HashMap<String, usize>,
    image_rels: &[String],
    rels: &mut DocRels,
) -> String {
    let mut body = String::new();
    let mut last_section_title: Option<String> = None;
    let mut first_section_emitted = false;

    for slide in slides {
        match &slide.kind {
            SlideKind::Title {
                subtitle,
                author,
                date,
            } => {
                body.push_str(&para_styled("Title", &escape_xml(&slide.title)));
                if let Some(s) = subtitle {
                    body.push_str(&para_styled("Subtitle", &escape_xml(s)));
                }
                let footer = match (author.as_ref(), date.as_ref()) {
                    (Some(a), Some(d)) => Some(format!("{} · {}", a, d)),
                    (Some(a), None) => Some(a.clone()),
                    (None, Some(d)) => Some(d.clone()),
                    _ => None,
                };
                if let Some(f) = footer {
                    body.push_str(&para_styled("Subtitle", &escape_xml(&f)));
                }
            }
            SlideKind::Section => {
                // Heading1 default style has pageBreakBefore. Override on the
                // first section to avoid a leading blank page.
                let style = if first_section_emitted {
                    "Heading1"
                } else {
                    "Heading1NoBreak"
                };
                first_section_emitted = true;
                body.push_str(&heading(style, &slide.title));
                last_section_title = Some(slide.title.clone());
                render_blocks(&mut body, &slide.blocks, theme, by_src, image_rels, rels);
            }
            SlideKind::Content => {
                let title = trim_cont_suffix(&slide.title);
                let skip = last_section_title
                    .as_deref()
                    .map(|s| trim_cont_suffix(s) == title)
                    .unwrap_or(false);
                if !skip {
                    body.push_str(&heading("Heading2", title));
                }
                last_section_title = None;
                render_blocks(&mut body, &slide.blocks, theme, by_src, image_rels, rels);
            }
        }
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
<w:body>
{}
<w:sectPr>
<w:pgSz w:w="12240" w:h="15840"/>
<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>
</w:sectPr>
</w:body>
</w:document>"#,
        body,
    )
}

/// Add `<w:bidi/>` to every paragraph's `<w:pPr>` and switch left-aligned
/// runs to right-aligned. Word recognises `<w:bidi/>` inside `pPr` as the
/// paragraph-direction toggle for RTL scripts.
fn apply_rtl_docx(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() + xml.matches("<w:pPr>").count() * 12);
    let mut rest = xml;
    while let Some(idx) = rest.find("<w:pPr>") {
        out.push_str(&rest[..idx + "<w:pPr>".len()]);
        out.push_str("<w:bidi/>");
        rest = &rest[idx + "<w:pPr>".len()..];
    }
    out.push_str(rest);
    // Flip explicit left-align to right-align where present.
    out.replace(r#"<w:jc w:val="left"/>"#, r#"<w:jc w:val="right"/>"#)
}

fn render_blocks(
    out: &mut String,
    blocks: &[Block],
    theme: &Theme,
    by_src: &HashMap<String, usize>,
    image_rels: &[String],
    rels: &mut DocRels,
) {
    for b in blocks {
        render_block(out, b, theme, by_src, image_rels, rels);
    }
}

fn render_block(
    out: &mut String,
    b: &Block,
    theme: &Theme,
    by_src: &HashMap<String, usize>,
    image_rels: &[String],
    rels: &mut DocRels,
) {
    match b {
        Block::Paragraph(runs) => {
            out.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr>{}</w:p>"#,
                inline_runs(runs, rels),
            ));
        }
        Block::Heading { level, runs } => {
            let style = match *level {
                3 => "Heading3",
                _ => "Heading3",
            };
            out.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="{}"/></w:pPr>{}</w:p>"#,
                style,
                inline_runs(runs, rels),
            ));
        }
        Block::List(items) => render_list(out, items, rels),
        Block::CodeBlock {
            lang, title, lines, ..
        } => {
            if let Some(t) = title {
                let label = match lang.as_deref() {
                    Some(l) => format!("[{}] {}", l, t),
                    None => t.clone(),
                };
                out.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:rPr><w:rStyle w:val="MetaLabel"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    escape_xml(&label),
                ));
            } else if let Some(l) = lang {
                out.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:rPr><w:rStyle w:val="MetaLabel"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    escape_xml(l),
                ));
            }
            for line in lines {
                out.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="CodeBlock"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    escape_xml(line),
                ));
            }
        }
        Block::Quote(paras) => {
            for para in paras {
                out.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr>{}</w:p>"#,
                    inline_runs(para, rels),
                ));
            }
        }
        Block::Table { headers, rows } => render_table(out, headers, rows, rels),
        Block::Columns { left, right } => {
            render_blocks(out, left, theme, by_src, image_rels, rels);
            render_blocks(out, right, theme, by_src, image_rels, rels);
        }
        Block::ColumnBreak => {}
        Block::Image {
            src,
            alt,
            width_pct: _,
        } => {
            if let Some(&idx) = by_src.get(src) {
                if let Some(rid) = image_rels.get(idx) {
                    render_image(out, rid, &theme_image_dims(), alt);
                }
            }
        }
        Block::Footnotes(items) => {
            out.push_str(
                r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:rPr><w:rStyle w:val="MetaLabel"/></w:rPr><w:t>Notes</w:t></w:r></w:p>"#,
            );
            for item in items {
                out.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr>{}</w:p>"#,
                    inline_runs(&item.runs, rels),
                ));
            }
        }
    }
}

fn render_list(out: &mut String, items: &[ListItem], rels: &mut DocRels) {
    for item in items {
        // numId 2 is the numbered list, 1 is the bullet list — both are
        // pre-declared in numbering.xml.
        let num_id = if item.ordered { 2 } else { 1 };
        let level = item.level.min(3);
        out.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="{}"/><w:numId w:val="{}"/></w:numPr></w:pPr>{}</w:p>"#,
            level,
            num_id,
            inline_runs(&item.runs, rels),
        ));
    }
}

fn render_table(
    out: &mut String,
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    rels: &mut DocRels,
) {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if cols == 0 {
        return;
    }
    let col_w = 9000 / cols.max(1) as u32; // dxa twentieths-of-a-point
    out.push_str(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/>
<w:tblBorders>
<w:top w:val="single" w:sz="4" w:color="CCCCCC"/>
<w:left w:val="single" w:sz="4" w:color="CCCCCC"/>
<w:bottom w:val="single" w:sz="4" w:color="CCCCCC"/>
<w:right w:val="single" w:sz="4" w:color="CCCCCC"/>
<w:insideH w:val="single" w:sz="4" w:color="EEEEEE"/>
<w:insideV w:val="single" w:sz="4" w:color="EEEEEE"/>
</w:tblBorders>
</w:tblPr><w:tblGrid>"#,
    );
    for _ in 0..cols {
        out.push_str(&format!(r#"<w:gridCol w:w="{}"/>"#, col_w));
    }
    out.push_str("</w:tblGrid>");

    if !headers.is_empty() {
        out.push_str("<w:tr><w:trPr><w:tblHeader/></w:trPr>");
        for c in headers
            .iter()
            .chain(std::iter::repeat(&Vec::new()))
            .take(cols)
        {
            out.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/><w:shd w:val="clear" w:color="auto" w:fill="F1F5F9"/></w:tcPr><w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p></w:tc>"#,
                col_w,
                escape_xml(&runs_plain_text(c)),
            ));
        }
        out.push_str("</w:tr>");
    }
    for row in rows {
        out.push_str("<w:tr>");
        for c in row.iter().chain(std::iter::repeat(&Vec::new())).take(cols) {
            out.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/></w:tcPr><w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr>{}</w:p></w:tc>"#,
                col_w,
                inline_runs(c, rels),
            ));
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
}

fn render_image(out: &mut String, rid: &str, dims: &(u32, u32), alt: &str) {
    // dims are EMU (914400 per inch). Render at ~6 inches wide.
    let target_w_emu = 6_000_000;
    let (iw, ih) = *dims;
    let aspect = if ih > 0 { iw as f32 / ih as f32 } else { 1.5 };
    let cx = target_w_emu;
    let cy = (target_w_emu as f32 / aspect.max(0.1)) as u32;
    out.push_str(&format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Normal"/><w:jc w:val="center"/></w:pPr><w:r><w:drawing>
<wp:inline distT="0" distB="0" distL="0" distR="0">
<wp:extent cx="{cx}" cy="{cy}"/>
<wp:docPr id="1" name="image" descr="{alt}"/>
<wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" noChangeAspect="1"/></wp:cNvGraphicFramePr>
<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
<pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
<pic:nvPicPr><pic:cNvPr id="0" name=""/><pic:cNvPicPr/></pic:nvPicPr>
<pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>
<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>
</pic:pic>
</a:graphicData>
</a:graphic>
</wp:inline>
</w:drawing></w:r></w:p>"#,
        cx = cx,
        cy = cy,
        alt = escape_xml(alt),
        rid = rid,
    ));
}

fn theme_image_dims() -> (u32, u32) {
    // Reasonable default 4:3 aspect when actual image dims aren't passed.
    (8, 6)
}

// ---------------------------------------------------------------------------
// Inline runs
// ---------------------------------------------------------------------------

fn inline_runs(runs: &[Run], rels: &mut DocRels) -> String {
    let mut s = String::new();
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        let mut rpr = String::from("<w:rPr>");
        if r.bold {
            rpr.push_str("<w:b/>");
        }
        if r.italic {
            rpr.push_str("<w:i/>");
        }
        if r.strike {
            rpr.push_str("<w:strike/>");
        }
        if r.code {
            rpr.push_str(r#"<w:rStyle w:val="InlineCode"/>"#);
        }
        if r.link.is_some() {
            rpr.push_str(r#"<w:rStyle w:val="Hyperlink"/>"#);
        }
        rpr.push_str("</w:rPr>");
        if rpr == "<w:rPr></w:rPr>" {
            rpr = String::new();
        }
        let run = format!(
            r#"<w:r>{}<w:t xml:space="preserve">{}</w:t></w:r>"#,
            rpr,
            escape_xml(&r.text),
        );
        if let Some(url) = &r.link {
            let rid = rels.add(REL_HYPERLINK, url, true);
            s.push_str(&format!(
                r#"<w:hyperlink r:id="{}" w:history="1">{}</w:hyperlink>"#,
                rid, run,
            ));
        } else {
            s.push_str(&run);
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn para_styled(style: &str, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="{}"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        style, text,
    )
}

fn heading(style: &str, title: &str) -> String {
    // Use a dedicated style id `Heading1NoBreak` to suppress the page break
    // on the first H1. We don't redeclare it in styles.xml; Word treats an
    // unknown style id as the default-style chain, which gives an OK fallback
    // (it inherits from Normal). To make it look like a Heading1 we inline
    // the formatting.
    if style == "Heading1NoBreak" {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="240" w:after="200"/><w:outlineLvl w:val="0"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="44"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            escape_xml(title),
        )
    } else {
        format!(
            r#"<w:p><w:pPr><w:pStyle w:val="{}"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            style,
            escape_xml(title),
        )
    }
}

fn runs_plain_text(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text.clone()).collect::<String>()
}

fn trim_cont_suffix(s: &str) -> &str {
    s.strip_suffix(" (cont.)").unwrap_or(s)
}

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
