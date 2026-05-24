//! Tiny LaTeX-flavoured math to Unicode translator. Not a real math
//! renderer — just enough to make `$E = mc^2$` and `\sum_{i=1}^n f(x_i)` look
//! reasonable in a slide deck without pulling in a TeX engine.
//!
//! The translator runs as a *preprocessor* pass before pulldown-cmark sees
//! the markdown. It replaces `$...$` and `$$...$$` spans (outside fenced
//! code blocks) with italicised Unicode. Anything we can't translate is
//! passed through verbatim so the user can spot it and adjust.

/// Walk the source string and replace math spans. Skips fenced code blocks
/// (` ``` ` / `~~~`) so dollar signs inside code are left alone.
pub fn translate(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_code = false;
    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            out.push_str(line);
            continue;
        }
        if in_code {
            out.push_str(line);
            continue;
        }
        translate_line(line, &mut out);
    }
    out
}

fn translate_line(line: &str, out: &mut String) {
    // Scan over byte indices but always slice on UTF-8 boundaries so we
    // never split a multi-byte char. The `$`, `\`, and `` ` `` markers we
    // care about are pure ASCII, so byte indices align with character
    // boundaries.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Pass-through for inline code spans. CommonMark: a run of N
        // backticks opens a code span that closes at the next run of
        // exactly N backticks. Math syntax inside is literal — copy it
        // verbatim so `$x$` and `x^2` in the docs survive intact.
        if bytes[i] == b'`' {
            let mut n = 0;
            while i + n < bytes.len() && bytes[i + n] == b'`' {
                n += 1;
            }
            if let Some(close) = find_code_span_close(bytes, i + n, n) {
                let end = close + n;
                out.push_str(&line[i..end]);
                i = end;
                continue;
            }
            // No matching close — treat the run as literal text.
            out.push_str(&line[i..i + n]);
            i += n;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
                if let Some(end) = find_close(bytes, i + 2, b"$$") {
                    let inner = &line[i + 2..end];
                    out.push_str(&render_math(inner, true));
                    i = end + 2;
                    continue;
                }
            } else if let Some(end) = find_close(bytes, i + 1, b"$") {
                let inner = &line[i + 1..end];
                if !inner.contains('\n') {
                    out.push_str(&render_math(inner, false));
                    i = end + 1;
                    continue;
                }
            }
        }
        // Copy one full UTF-8 character at a time.
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&line[i..ch_end]);
        i = ch_end;
    }
}

/// Find the start of a closing backtick run of exactly `n` backticks. Returns
/// the byte index of the first backtick of the closing run, or `None` if
/// there is no matching close on this line.
fn find_code_span_close(bytes: &[u8], from: usize, n: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] != b'`' {
            j += 1;
            continue;
        }
        let mut run = 0;
        while j + run < bytes.len() && bytes[j + run] == b'`' {
            run += 1;
        }
        if run == n {
            return Some(j);
        }
        j += run;
    }
    None
}

fn utf8_char_end(bytes: &[u8], start: usize) -> usize {
    let b = bytes[start];
    let len = if b < 0x80 {
        1
    } else if b < 0xC0 {
        1 // continuation byte — shouldn't be a start, but be defensive
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    };
    (start + len).min(bytes.len())
}

fn find_close(bytes: &[u8], from: usize, marker: &[u8]) -> Option<usize> {
    let n = marker.len();
    let mut j = from;
    while j + n <= bytes.len() {
        if &bytes[j..j + n] == marker {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn render_math(src: &str, display: bool) -> String {
    let body = translate_math(src);
    if display {
        // Display math goes on its own line, italicised by the renderer via
        // surrounding underscores (markdown italics).
        format!("\n\n*{}*\n\n", body)
    } else {
        format!("*{}*", body)
    }
}

fn translate_math(src: &str) -> String {
    // Order matters here. We need `^{...}` and `_{...}` to keep their
    // braces long enough for `apply_super_sub` to read the full group;
    // otherwise stripping `{` and `}` early collapses `x^{n+1}` to `x^n+1`
    // and the super-sub pass would only catch the first character.
    let with_macros = expand_macros(src.trim());
    let with_frac = expand_frac(&with_macros);
    let with_binom = expand_binom(&with_frac);
    let with_sqrt = expand_sqrt(&with_binom);
    let with_super = apply_super_sub(&with_sqrt);
    // Anything still wrapped in braces was either malformed or a group we
    // don't recognise; flatten it now so the user doesn't see literal `{}`.
    with_super.replace('{', "").replace('}', "")
}

/// Replace `\alpha`-style commands with their Unicode equivalents.
fn expand_macros(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Single-character spacing macros: `\,` (thin), `\:` (medium),
        // `\;` (thick), `\!` (negative thin). LaTeX uses these for
        // fine-grained spacing; we just collapse them to a plain space
        // (or nothing for the negative one) since our renderer has no
        // typographic spacing primitives.
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            let replacement: Option<&str> = match next {
                b',' | b':' | b';' => Some(" "),
                b'!' => Some(""),
                _ => None,
            };
            if let Some(rep) = replacement {
                out.push_str(rep);
                i += 2;
                continue;
            }
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic()
        {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_ascii_alphabetic() {
                j += 1;
            }
            let name = &s[i + 1..j];
            if let Some(rep) = greek_or_symbol(name) {
                out.push_str(rep);
                i = j;
                // LaTeX swallows a single trailing space after a macro so
                // `\alpha k` reads as `αk`. That's the right behaviour for
                // single-glyph replacements (Greek letters, symbols), but
                // operator names like `\log n` need to keep the space:
                // "log n" is the desired reading, not "logn". Detect the
                // word-shaped case by the last char of the replacement.
                let rep_ends_in_alpha = rep
                    .chars()
                    .last()
                    .map_or(false, |c| c.is_ascii_alphabetic());
                if !rep_ends_in_alpha && i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }
                continue;
            }
            out.push_str(&s[i..j]);
            i = j;
            continue;
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn expand_frac(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 5 <= bytes.len() && &bytes[i..i + 5] == b"\\frac" && bytes[i] == b'\\' {
            let mut j = i + 5;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                if let (Some(end1), num_text) = read_group(bytes, j) {
                    let mut k = end1 + 1;
                    while k < bytes.len() && bytes[k] == b' ' {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'{' {
                        if let (Some(end2), den_text) = read_group(bytes, k) {
                            out.push('(');
                            out.push_str(&num_text);
                            out.push_str(")/(");
                            out.push_str(&den_text);
                            out.push(')');
                            i = end2 + 1;
                            continue;
                        }
                    }
                }
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

/// `\binom{n}{k}` → `C(n, k)`. Real LaTeX renders it as a column of two
/// values inside parentheses; the closest unambiguous ASCII analogue is the
/// `C(n, k)` (combinations) shorthand used in combinatorics textbooks.
fn expand_binom(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 6 <= bytes.len() && &bytes[i..i + 6] == b"\\binom" && bytes[i] == b'\\' {
            let mut j = i + 6;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                if let (Some(end1), top) = read_group(bytes, j) {
                    let mut k = end1 + 1;
                    while k < bytes.len() && bytes[k] == b' ' {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'{' {
                        if let (Some(end2), bot) = read_group(bytes, k) {
                            out.push_str("C(");
                            out.push_str(&top);
                            out.push_str(", ");
                            out.push_str(&bot);
                            out.push(')');
                            i = end2 + 1;
                            continue;
                        }
                    }
                }
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn expand_sqrt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 5 <= bytes.len() && &bytes[i..i + 5] == b"\\sqrt" {
            let mut j = i + 5;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                if let (Some(end), inner) = read_group(bytes, j) {
                    out.push('√');
                    out.push('(');
                    out.push_str(&inner);
                    out.push(')');
                    i = end + 1;
                    continue;
                }
            }
            out.push('√');
            i = j;
            continue;
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn read_group(bytes: &[u8], open: usize) -> (Option<usize>, String) {
    debug_assert_eq!(bytes[open], b'{');
    let mut depth = 0;
    let mut j = open;
    while j < bytes.len() {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let inner = std::str::from_utf8(&bytes[open + 1..j])
                        .unwrap_or("")
                        .to_string();
                    return (Some(j), inner);
                }
            }
            _ => {}
        }
        j += 1;
    }
    (None, String::new())
}

/// Convert `x^2`, `x^{2n}`, `a_i`, `a_{ij}` to Unicode super/subscript when
/// every character in the exponent maps cleanly. Falls back to `x^(2n)`
/// notation otherwise.
fn apply_super_sub(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c < (0x80 as char) && (c == '^' || c == '_') && !out.is_empty() {
            let map = if c == '^' {
                superscript_char
            } else {
                subscript_char
            };
            let (end, group) = if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                read_group(bytes, i + 1)
            } else if i + 1 < bytes.len() {
                let end = i + 2;
                (
                    Some(i + 1),
                    std::str::from_utf8(&bytes[i + 1..end])
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                (None, String::new())
            };
            if let Some(end_idx) = end {
                let mapped: Option<String> = group.chars().map(map).collect::<Option<String>>();
                if let Some(text) = mapped {
                    out.push_str(&text);
                } else {
                    out.push(c);
                    if group.chars().count() > 1 {
                        out.push('(');
                        out.push_str(&group);
                        out.push(')');
                    } else {
                        out.push_str(&group);
                    }
                }
                i = end_idx + 1;
                continue;
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn superscript_char(c: char) -> Option<String> {
    let m = match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        ' ' => ' ',
        // Lowercase Latin (Unicode "Modifier Letter Small …" range).
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        // Uppercase Latin where Unicode has a modifier-letter form. Missing
        // glyphs: C F Q S X Y Z (no equivalent codepoint exists, so callers
        // fall back to literal `^X` notation for those).
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        _ => return None,
    };
    Some(m.to_string())
}

fn subscript_char(c: char) -> Option<String> {
    let m = match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    };
    Some(m.to_string())
}

fn greek_or_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Greek lowercase
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "varphi" => "ϕ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        // Greek uppercase
        "Alpha" => "Α",
        "Beta" => "Β",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Epsilon" => "Ε",
        "Zeta" => "Ζ",
        "Eta" => "Η",
        "Theta" => "Θ",
        "Iota" => "Ι",
        "Kappa" => "Κ",
        "Lambda" => "Λ",
        "Mu" => "Μ",
        "Nu" => "Ν",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Rho" => "Ρ",
        "Sigma" => "Σ",
        "Tau" => "Τ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Chi" => "Χ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        // Large operators
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "oint" => "∮",
        "bigcup" => "⋃",
        "bigcap" => "⋂",
        // Relations
        "leq" => "≤",
        "le" => "≤",
        "geq" => "≥",
        "ge" => "≥",
        "neq" => "≠",
        "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "propto" => "∝",
        // Arrows
        "to" => "→",
        "rightarrow" => "→",
        "leftarrow" => "←",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "leftrightarrow" => "↔",
        "Leftrightarrow" => "⇔",
        "mapsto" => "↦",
        // Operators
        "pm" => "±",
        "mp" => "∓",
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "ast" => "∗",
        "star" => "⋆",
        // Logic / set
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "land" => "∧",
        "lor" => "∨",
        "lnot" => "¬",
        // Misc
        "infty" => "∞",
        "emptyset" => "∅",
        "partial" => "∂",
        "nabla" => "∇",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "ldots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        // Delimiter sizing macros — real LaTeX auto-scales the following
        // delimiter to match the height of its content. We have no stacking
        // engine, so we just collapse them to empty and let the explicit
        // delimiter character that follows render normally.
        "left" => "",
        "right" => "",
        "big" => "",
        "Big" => "",
        "bigg" => "",
        "Bigg" => "",
        // Spacing macros. Real LaTeX has fine-grained spacing primitives;
        // map them to plain ASCII spaces of roughly the same visual width.
        "quad" => "  ",
        "qquad" => "    ",
        // Operator names. In real LaTeX these are typeset in roman; we just
        // emit the literal text so `\log n` reads as `log n`. Without these
        // entries the backslash + name would leak into the output verbatim.
        "log" => "log",
        "ln" => "ln",
        "lg" => "lg",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "csc" => "csc",
        "sec" => "sec",
        "cot" => "cot",
        "arcsin" => "arcsin",
        "arccos" => "arccos",
        "arctan" => "arctan",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "tanh" => "tanh",
        "exp" => "exp",
        "min" => "min",
        "max" => "max",
        "sup" => "sup",
        "inf" => "inf",
        "lim" => "lim",
        "liminf" => "lim inf",
        "limsup" => "lim sup",
        "deg" => "deg",
        "det" => "det",
        "dim" => "dim",
        "gcd" => "gcd",
        "hom" => "hom",
        "ker" => "ker",
        "mod" => "mod",
        "Pr" => "Pr",
        _ => return None,
    })
}
