//! Golden-file tests over the parser → paginator pipeline.
//!
//! Each fixture in `tests/snapshots/<name>.md` is parsed, paginated, and
//! serialized via `md2any::slides_snapshot`. The result is compared against
//! `tests/snapshots/<name>.snap`. Run `UPDATE_SNAPSHOTS=1 cargo test` to
//! regenerate goldens after an intentional change.

use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn run_case(stem: &str) {
    let dir = fixtures_dir();
    let md_path = dir.join(format!("{stem}.md"));
    let snap_path = dir.join(format!("{stem}.snap"));
    let input = fs::read_to_string(&md_path)
        .unwrap_or_else(|e| panic!("read {}: {}", md_path.display(), e));

    let (front, body) = md2any::front_matter::extract(&input);
    let theme = md2any::theme::Theme::resolve(
        front.theme.as_deref().unwrap_or("light"),
        front.aspect.as_deref().unwrap_or("16:9"),
        front.font.as_deref(),
    )
    .expect("resolve theme");
    let slides = md2any::parser::parse(&body, &front, stem);
    let slides = md2any::paginate::paginate(slides, &theme);
    let slides = if front.toc {
        md2any::toc::inject(slides)
    } else {
        slides
    };

    let actual = md2any::slides_snapshot(&slides);

    if std::env::var("UPDATE_SNAPSHOTS").is_ok() || !snap_path.exists() {
        fs::write(&snap_path, &actual).expect("write snapshot");
        eprintln!("snapshot written: {}", snap_path.display());
        return;
    }

    let expected = fs::read_to_string(&snap_path).expect("read snapshot");
    if actual != expected {
        let diff_path = dir.join(format!("{stem}.actual.snap"));
        fs::write(&diff_path, &actual).ok();
        panic!(
            "snapshot mismatch for {stem}.\n\
             expected:\n{expected}\n\
             actual:\n{actual}\n\
             actual written to {}\n\
             rerun with UPDATE_SNAPSHOTS=1 cargo test to accept.",
            diff_path.display(),
        );
    }
}

#[test]
fn basic_deck() {
    run_case("basic");
}

#[test]
fn lists_and_code() {
    run_case("lists_and_code");
}

#[test]
fn footnotes() {
    run_case("footnotes");
}

#[test]
fn tables_and_columns() {
    run_case("tables_and_columns");
}
