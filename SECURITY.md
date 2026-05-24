# Security policy

## Reporting a vulnerability

If you've found a security issue in md2any, please **don't** open a public
issue. Instead:

1. Open a [private security advisory][adv] on GitHub.
2. Include a minimal reproduction (input markdown that causes the issue,
   the exact command line, and the unexpected behaviour).
3. We will acknowledge within 7 days and aim to ship a fix within 30 days
   for confirmed vulnerabilities.

[adv]: https://github.com/javaperformance/md2any/security/advisories/new

## Threat model

md2any reads markdown files and writes Office/OpenDocument/PDF files. The
most plausible security boundaries are:

- **Untrusted input markdown** — md2any parses markdown and embeds local
  image files. If md2any is run on an untrusted markdown file, the input
  can reference arbitrary local paths via `![alt](path)` or
  `<!-- bg: path -->`. The file's contents will be embedded in the output
  document. **Don't run md2any on untrusted input against a file system
  with secrets.**

- **Shell-out to diagram tools** — When a code fence is tagged `dot`,
  `mermaid`, or `plantuml`, md2any shells out to the matching CLI tool
  with the fence contents as input. If you don't trust the source markdown,
  consider these inputs the same as running the diagram tool directly on
  attacker-controlled input.

- **Generated output files** — md2any writes only to the path passed via
  `-o` (or derived from the input filename). It does not create files
  elsewhere on the file system.

## Supported versions

Only the latest released version receives security fixes. md2any uses
semantic versioning; patch releases (`0.1.x`) are backwards-compatible bug
fixes including security fixes.
