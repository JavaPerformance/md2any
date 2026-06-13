---
title: Rich Math
subtitle: Native display math without TeX
author: md2any
theme: light
math: svg
math_scale: 1.0
math_block_align: center
math_max_height: 180
math_macros:
  '\RR': '\mathbb{R}'
---

# Rich Math

## Fractions, Roots, And Scripts

Inline math stays readable source in `--math svg` mode: $E = mc^2$.

$$
\left(\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}\right)^2
$$

## Matrices And Arrays

$$
\left[
\begin{array}{cc}
\frac{1}{\sqrt{x^2 + 1}} & y_i^2 \\
\alpha + \beta & \binom{n}{k}
\end{array}
\right]
$$

## Cases

$$
f(x)=
\begin{cases}
x^2 & x \ge 0 \\
-x & x < 0
\end{cases}
$$

## Macros

$$
\forall x \in \RR^n,\quad \exists y \in \RR : y > \lVert x \rVert
$$
