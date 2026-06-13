---
title: Code Includes and Table Fit
author: md2any examples
aspect: 16:9
table_fit: auto
---

# Source snippets

## Include a line range

Fenced code blocks can pull from files next to the markdown source and keep
the original line numbers in code gutters and continuation slides.

```rust file=assets/snippet.rs#L5-L13 title="state update"
```

# Wide tables

## Split by column groups

`--table-fit auto` keeps the first column as a key column when a wide table is
split into readable column groups.

| Metric | Jan | Feb | Mar | Apr | May | Jun | Jul | Aug | Sep |
|--------|-----|-----|-----|-----|-----|-----|-----|-----|-----|
| Build  | 42  | 40  | 39  | 41  | 38  | 36  | 35  | 34  | 33  |
| Test   | 18  | 18  | 17  | 17  | 16  | 16  | 15  | 15  | 14  |
| Deploy | 9   | 9   | 8   | 8   | 8   | 7   | 7   | 7   | 6   |
