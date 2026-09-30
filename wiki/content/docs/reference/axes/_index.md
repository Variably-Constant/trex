---
title: Property axes
linkTitle: Axes
weight: 30
sidebar:
  open: true
---

A token stream carries numeric fields beyond each token's identity. Each **axis** is one such
field, available as a standalone command and as a pattern predicate. They are orthogonal: a
token has a scale *and* a nesting depth *and* a texture at once, and each axis reads one of
them.

{{< cards >}}
  {{< card link="spectral/" title="Spectral" subtitle="Temporal texture: local entropy, byte-period, change-points." >}}
  {{< card link="seam/" title="Seam" subtitle="Predictive segmentation: cut where the past stops predicting the future." >}}
  {{< card link="shape/" title="Shape" subtitle="Silhouette structure: width-free template period." >}}
  {{< card link="orbit/" title="Orbit" subtitle="Symmetry: fold case / notation / word-shape to one representative." >}}
  {{< card link="magnitude/" title="Magnitude" subtitle="Scale: order of magnitude, energy, gradient, outliers." >}}
  {{< card link="stress/" title="Stress" subtitle="Structural load: bracket-nesting depth, strain, fracture." >}}
  {{< card link="flow/" title="Flow" subtitle="Dynamics: slope, direction, momentum of any signal." >}}
  {{< card link="observation/" title="Observation" subtitle="Vantage: how observer-dependent each reading is." >}}
  {{< card link="echo/" title="Echo" subtitle="Recurrence: content-addressed repetition at unbounded range." >}}
  {{< card link="context/" title="Context" subtitle="The rolling context: every axis folded over a window, a column, a scope, or a key's history." >}}
{{< /cards >}}

## The taxonomy

| Family | Question | Axes |
|---|---|---|
| boundary | where to cut? | BPE (frequency), seam (predictability) |
| character | what is here? | spectral (temporal), shape (structural) |
| identity | what is the same? | orbit (symmetry) |
| scale | how much? | magnitude / energy / gradient |
| load | how strained? | stress |
| dynamics | which way, how fast? | flow |
| vantage | how observer-dependent? | observation |
| recurrence | does it return? | echo (the two-point function over every other axis's one-point readings) |
| context | against what? | context (every axis folded over the window, the column, the scope, or the history a token is read against) |
