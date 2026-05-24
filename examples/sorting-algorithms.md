---
title: A tour of sorting algorithms
subtitle: From bubble to Timsort, one algorithm at a time
author: md2any examples
date: auto
theme: light
aspect: 16:9
layout: clean
toc: true
---

# Why sorting still matters

## A solved problem, sort of

Sorting is the most-studied problem in computer science. Knuth devoted volume 3 of *The Art of Computer Programming* to it, then revised the volume twice. Every standard library ships at least one general-purpose sort. The fastest comparison-based algorithms have been within a constant factor of optimal since the 1960s.

So why study it?

- It's the **canonical lens** for analysing algorithms — best, average, worst cases; in-place vs. extra memory; stable vs. unstable.
- Real production sorts are **hybrids** — Timsort, pdqsort, introsort — built from the textbook primitives, not a single algorithm.
- Sorting is a **building block** for selection, deduplication, set operations, joins, and most database query plans.
- It pops up in interviews more than any other topic in the canon.

This deck walks through six classics + one hybrid, in the order you'd typically learn them.

## The lineup

| Algorithm     | Best        | Average    | Worst       | Memory | Stable |
|---------------|-------------|------------|-------------|--------|--------|
| Bubble sort   | $O(n)$      | $O(n^2)$   | $O(n^2)$    | $O(1)$ | yes    |
| Insertion sort| $O(n)$      | $O(n^2)$   | $O(n^2)$    | $O(1)$ | yes    |
| Selection sort| $O(n^2)$    | $O(n^2)$   | $O(n^2)$    | $O(1)$ | no     |
| Merge sort    | $O(n \log n)$| $O(n \log n)$| $O(n \log n)$| $O(n)$ | yes    |
| Heap sort     | $O(n \log n)$| $O(n \log n)$| $O(n \log n)$| $O(1)$ | no     |
| Quicksort     | $O(n \log n)$| $O(n \log n)$| $O(n^2)$   | $O(\log n)$| no |
| Timsort       | $O(n)$      | $O(n \log n)$| $O(n \log n)$| $O(n)$ | yes |

The lower bound for comparison-based sorting is $\Omega(n \log n)$ — proven by the decision-tree argument. To go faster you need extra structure: counting sort and radix sort hit $O(n)$ when keys are bounded integers.

# Bubble sort

## The simplest one

```python
def bubble_sort(a):
    n = len(a)
    for i in range(n):
        swapped = False
        for j in range(0, n - i - 1):
            if a[j] > a[j + 1]:
                a[j], a[j + 1] = a[j + 1], a[j]
                swapped = True
        if not swapped:
            break
    return a
```

Each pass walks the array and swaps adjacent pairs that are out of order. The largest element "bubbles" to the end on each pass, so after $k$ passes the last $k$ positions are sorted. The early-exit on `swapped == False` gives the best case of $O(n)$ on already-sorted input.

## Why we still teach it

It's slow — $O(n^2)$ comparisons and swaps in the average case — and almost never the right choice in production. But it's the easiest sort to explain on a whiteboard, and the inner loop is so simple that it's a useful demonstration of:

- **Pass-based** algorithms (versus recursive)
- **Comparison + swap** as the primitive operation
- **Early termination** when no work was done on a pass

# Insertion sort

## Like sorting playing cards in your hand

```python
def insertion_sort(a):
    for i in range(1, len(a)):
        key = a[i]
        j = i - 1
        while j >= 0 and a[j] > key:
            a[j + 1] = a[j]
            j -= 1
        a[j + 1] = key
    return a
```

Walk left to right; for each element, slide it back through the already-sorted prefix until it lands in the right spot.

Same $O(n^2)$ worst case as bubble sort, but with two advantages:

- **Fewer writes** on average — each element moves once into position, instead of bubbling pairwise.
- **Adaptive:** $O(n)$ on already-sorted input, $O(n + d)$ where $d$ is the number of inversions. Real-world data is often *almost* sorted, which makes insertion sort surprisingly competitive on small inputs.

Production sorts (Timsort, pdqsort, introsort) all switch to insertion sort for sub-arrays below ~16-32 elements. The constant factor wins at small $n$.

## In Rust

```rust
fn insertion_sort<T: Ord>(slice: &mut [T]) {
    for i in 1..slice.len() {
        let mut j = i;
        while j > 0 && slice[j - 1] > slice[j] {
            slice.swap(j - 1, j);
            j -= 1;
        }
    }
}
```

# Selection sort

## Find the minimum, repeat

```python
def selection_sort(a):
    n = len(a)
    for i in range(n):
        min_idx = i
        for j in range(i + 1, n):
            if a[j] < a[min_idx]:
                min_idx = j
        a[i], a[min_idx] = a[min_idx], a[i]
    return a
```

The cleanest of the $O(n^2)$ family conceptually: scan to find the smallest remaining element, swap it into position, advance. Repeat $n$ times.

It performs **exactly $n - 1$ swaps**, regardless of input. That's the lowest swap count of any $O(n^2)$ sort, which matters when comparisons are cheap but swaps are expensive (e.g. moving large records, or writing to flash memory). Otherwise insertion sort beats it on almost every metric.

It is **not stable** — equal elements can swap relative order during the long-distance swap step.

# Merge sort

## Divide and conquer, the prototype

```rust
fn merge_sort<T: Ord + Clone>(a: &[T]) -> Vec<T> {
    if a.len() <= 1 {
        return a.to_vec();
    }
    let mid = a.len() / 2;
    let left = merge_sort(&a[..mid]);
    let right = merge_sort(&a[mid..]);
    merge(left, right)
}

fn merge<T: Ord>(mut left: Vec<T>, mut right: Vec<T>) -> Vec<T> {
    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut li, mut ri) = (0, 0);
    while li < left.len() && ri < right.len() {
        if left[li] <= right[ri] {
            out.push(std::mem::replace(&mut left[li], unsafe { std::mem::zeroed() }));
            li += 1;
        } else {
            out.push(std::mem::replace(&mut right[ri], unsafe { std::mem::zeroed() }));
            ri += 1;
        }
    }
    out.extend(left.drain(li..));
    out.extend(right.drain(ri..));
    out
}
```

## The recurrence

Splitting an array of size $n$ into two halves and merging takes $O(n)$ work for the merge plus two recursive calls of size $n/2$:

$$T(n) = 2T(n/2) + O(n)$$

The master theorem gives $T(n) = O(n \log n)$. This holds for **every** input, best/average/worst — merge sort has no bad cases. The catch is $O(n)$ auxiliary memory for the merge buffer, which is why it's less popular than quicksort despite the better worst-case guarantee.

Merge sort is **stable**: equal elements preserve their relative order because the merge step takes from the left half on ties.

# Quicksort

## The most-used sort in practice

```rust
fn quicksort<T: Ord>(slice: &mut [T]) {
    if slice.len() <= 1 {
        return;
    }
    let pivot = partition(slice);
    let (left, right) = slice.split_at_mut(pivot);
    quicksort(left);
    quicksort(&mut right[1..]);
}

fn partition<T: Ord>(slice: &mut [T]) -> usize {
    let pivot = slice.len() - 1;
    let mut store = 0;
    for i in 0..pivot {
        if slice[i] <= slice[pivot] {
            slice.swap(i, store);
            store += 1;
        }
    }
    slice.swap(store, pivot);
    store
}
```

Pick a pivot, partition around it, recurse on each side. The classic divide-and-conquer with in-place partitioning.

## Why $O(n^2)$ worst case?

If the pivot is always the smallest or largest element (e.g. picking the last element of an already-sorted array), partitioning produces one empty side and one side of size $n-1$. The recurrence becomes:

$$T(n) = T(n-1) + O(n) = O(n^2)$$

Production implementations dodge this with:

- **Median-of-three** pivoting: pick the median of `a[0]`, `a[mid]`, `a[last]`
- **Random** pivot selection
- **Introsort**: fall back to heap sort when recursion depth exceeds $2 \log_2 n$
- **pdqsort**: introsort + pattern-defeating heuristics, the algorithm Rust's stable `sort_unstable` uses

With these guards, the practical worst case is $O(n \log n)$ and the constant factor is excellent — fewer comparisons and far better cache behaviour than merge sort.

# Heap sort

## Sorting via a binary heap

```python
def heap_sort(a):
    n = len(a)
    # Build max-heap in place.
    for i in range(n // 2 - 1, -1, -1):
        _sift_down(a, i, n)
    # Repeatedly swap root to end and sift down the new root.
    for end in range(n - 1, 0, -1):
        a[0], a[end] = a[end], a[0]
        _sift_down(a, 0, end)
    return a

def _sift_down(a, root, end):
    while True:
        child = 2 * root + 1
        if child >= end:
            return
        if child + 1 < end and a[child + 1] > a[child]:
            child += 1
        if a[root] >= a[child]:
            return
        a[root], a[child] = a[child], a[root]
        root = child
```

Build a max-heap, then repeatedly swap the root (the maximum) to the end of the array and sift down. Each `_sift_down` is $O(\log n)$, called $n$ times, giving the guaranteed $O(n \log n)$.

Trade-off vs. merge sort: **in-place** ($O(1)$ extra memory) but **not stable**, and the constant factor is worse than quicksort because each sift-down jumps across cache lines.

Used as the fallback in **introsort** when quicksort's recursion gets too deep, precisely because its $O(n \log n)$ worst case is guaranteed.

# Timsort

## The one your language probably uses

Tim Peters wrote it for Python in 2002. The Java standard library adopted it in 2009 for object arrays. Rust's stable `sort` is a Timsort variant. Android, V8, and Swift use it. The C++ standard library mostly uses introsort, but `std::stable_sort` is typically a variant of merge sort with Timsort-like enhancements.

Two ideas, combined:

1. **Real-world data has runs** — sequences that are already sorted, ascending or descending. Find them, reverse the descending ones, then merge.
2. **Merging is cheap** when you can use $O(n)$ extra memory and the runs are large.

The result is an adaptive merge sort that hits $O(n)$ on already-sorted input, $O(n \log n)$ worst case, and is stable.

```python
# Conceptual skeleton — production Timsort is ~1000 lines with binary
# insertion sort for small runs, galloping mode for skewed merges, and a
# carefully maintained stack of pending merges (the "stack invariant").
def timsort(a):
    runs = find_runs(a)              # detect already-sorted subsequences
    runs = [r if ascending(r) else reverse(r) for r in runs]
    return merge_runs(runs)
```

# When to use which

## The decision table

| Situation                                | Pick                           |
|------------------------------------------|--------------------------------|
| Anything in the standard library         | Whatever it ships (Timsort / pdqsort / introsort) |
| Small ($n < 32$) sub-arrays              | Insertion sort                 |
| Need stability (preserve equal-key order)| Timsort or merge sort          |
| Tight memory budget, can't afford $O(n)$ | Heap sort or quicksort         |
| Already mostly sorted                    | Timsort or insertion sort      |
| Worst-case guarantee required (real-time)| Heap sort or merge sort        |
| Bounded integer keys                     | Counting sort or radix sort    |
| Massive data, doesn't fit in RAM         | External merge sort (multi-way) |

Default to your language's built-in. Reach for a custom implementation only when profiling proves it matters.

# Beyond comparison-based sorting

## When you know more about the keys

The $\Omega(n \log n)$ lower bound applies to algorithms that **compare** keys. If your keys have structure — they're bounded integers, fixed-width strings, floats with known range — you can do better.

| Algorithm     | Time         | Memory | Notes                                    |
|---------------|--------------|--------|------------------------------------------|
| Counting sort | $O(n + k)$   | $O(k)$ | $k$ = range of keys                       |
| Radix sort    | $O(d(n + b))$| $O(n+b)$| $d$ = digits, $b$ = base; great for ints |
| Bucket sort   | $O(n)$ avg   | $O(n)$ | uniform-distribution assumption          |
| Pigeonhole    | $O(n + k)$   | $O(k)$ | one bucket per distinct key              |

Radix sort on 32-bit integers in base 256 does 4 passes of counting sort — that's the workhorse for sorting numeric IDs at scale.

# Code in unexpected places

## COBOL: sort verb

The COBOL `SORT` verb has been in the language since 1959. It compiles down to the platform's high-performance sort runtime (often a tuned merge sort + radix combination on z/OS).

```cobol
       IDENTIFICATION DIVISION.
       PROGRAM-ID. SORTER.
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT INPUT-FILE  ASSIGN TO "IN.DAT".
           SELECT OUTPUT-FILE ASSIGN TO "OUT.DAT".
           SELECT WORK-FILE   ASSIGN TO "SORTWK".
       DATA DIVISION.
       FILE SECTION.
       SD WORK-FILE.
       01 WORK-REC.
          05 KEY-FIELD PIC X(10).
          05 REST-OF   PIC X(70).
       PROCEDURE DIVISION.
       MAIN-PARA.
           SORT WORK-FILE
               ON ASCENDING KEY KEY-FIELD
               USING INPUT-FILE
               GIVING OUTPUT-FILE.
           STOP RUN.
```

## JCL: sorting a dataset

If you don't need custom logic, the operating system can sort a file before your program ever runs. Decades-old, still in production.

```jcl
//SORTJOB  JOB  (ACCT),'SORT EMP FILE',CLASS=A,MSGCLASS=H
//STEP1    EXEC PGM=SORT
//SORTIN   DD   DSN=PROD.PAYROLL.EMPLOYEES,DISP=SHR
//SORTOUT  DD   DSN=PROD.PAYROLL.EMPLOYEES.SORTED,
//              DISP=(NEW,CATLG,DELETE),
//              SPACE=(CYL,(50,10),RLSE)
//SORTWK01 DD   UNIT=SYSDA,SPACE=(CYL,(50,10))
//SYSIN    DD   *
  SORT FIELDS=(1,10,CH,A)
/*
```

## PL/I: in-memory recursive quicksort

```pli
QSORT: PROCEDURE (A, LO, HI) RECURSIVE;
   DECLARE A(*) FIXED BIN(31),
           LO   FIXED BIN(31),
           HI   FIXED BIN(31);
   DECLARE (I, J, PIVOT, TEMP) FIXED BIN(31);
   IF LO < HI THEN DO;
      PIVOT = A(HI);
      I = LO - 1;
      DO J = LO TO HI - 1;
         IF A(J) <= PIVOT THEN DO;
            I = I + 1;
            TEMP = A(I); A(I) = A(J); A(J) = TEMP;
         END;
      END;
      TEMP = A(I+1); A(I+1) = A(HI); A(HI) = TEMP;
      CALL QSORT(A, LO, I);
      CALL QSORT(A, I + 2, HI);
   END;
END QSORT;
```

# What md2any showed off in this deck

## Feature index

- **Syntax highlighting** across 5 languages: Python, Rust, COBOL, JCL, PL/I
- **Block math** with $T(n) = 2T(n/2) + O(n)$ recurrence notation
- **Inline math** for complexity classes — $O(n \log n)$, $\Omega(n \log n)$, $O(1)$
- **Decision tables** with three and four columns
- **Comparison tables** of all seven algorithms across six dimensions
- **Code blocks with filename captions** (Rust insertion sort)
- **Nested lists** and lead-in paragraphs
- **External links** to references and standard-library implementations
- All in pure markdown — no images, no external dependencies, fully offline-renderable

Sources: Knuth, *The Art of Computer Programming Vol 3* (2nd ed., 1998); Tim Peters' [Timsort listsort.txt](https://github.com/python/cpython/blob/main/Objects/listsort.txt); Orson Peters' [pdqsort paper](https://arxiv.org/abs/2106.05123).
