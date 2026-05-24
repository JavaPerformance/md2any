---
title: My talk
subtitle: A subtitle for the title slide
author: Your name
date: auto
theme: light
aspect: 16:9
layout: clean
---

# Introduction

Write your opening paragraph here. This becomes the body of the section
intro after the title slide.

## What we'll cover

- First point
- Second point
- Third point

## The plan

We're going to walk through three things today: setup, the live demo,
and the takeaways. Each gets its own section.

# Setup

## Prerequisites

Anything the audience needs to follow along.

```bash
brew install something
git clone https://example.com/repo.git
```

## Configuration

Edit `config.toml` and set the following:

```toml
[server]
host = "localhost"
port = 8080
```

# Live demo

## The happy path

Walk through the normal flow first. Keep it focused — one observation
per slide.

## Edge cases

| Case        | Behavior            | Notes               |
|-------------|---------------------|---------------------|
| Empty input | Returns nothing     | Graceful no-op      |
| Bad input   | Returns error       | Logged at WARN      |
| Large input | Streams in chunks   | Backpressure works  |

# Takeaways

## Three things to remember

1. The first thing
2. The second thing
3. The third thing

<!-- notes: Pause for questions here. Common: "what about X?" — answer is in the appendix. -->

# Thank you

## Questions?

Reach me at [your.name@example.com](mailto:your.name@example.com).
