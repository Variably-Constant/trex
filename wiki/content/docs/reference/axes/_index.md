---
title: Property axes
linkTitle: Axes
weight: 30
sidebar:
  open: true
---

An axis is one reading taken at every position of the input: a token has a scale, a nesting
depth and a texture at once, and each axis reads one of them.

{{< cards >}}
  {{< card link="spectral/" title="Spectral" subtitle="Byte-class mix, entropy, byte period and novelty at every byte, and where they change." >}}
  {{< card link="seam/" title="Seam" subtitle="Cuts where the bytes before a point stop predicting the bytes after it." >}}
  {{< card link="shape/" title="Shape" subtitle="Each token's silhouette, the period it repeats at, and the regions that repeat like a table." >}}
  {{< card link="orbit/" title="Orbit" subtitle="Each token as its representative under case, notation or word-shape folding." >}}
  {{< card link="magnitude/" title="Magnitude" subtitle="Each token's order of magnitude, its change, the window's energy, and the outliers." >}}
  {{< card link="stress/" title="Stress" subtitle="Bracket depth at each token, how long it has been held open, and where it falls." >}}
  {{< card link="flow/" title="Flow" subtitle="The slope, direction and momentum of a per-token signal, and where it reverses." >}}
  {{< card link="observation/" title="Observation" subtitle="Byte-class entropy of the past, the future and both, and where past and future disagree." >}}
  {{< card link="echo/" title="Echo" subtitle="Whether each token's content recurs, how often, how far away and how regularly." >}}
  {{< card link="relation/" title="Relation" subtitle="The graph of enclosure, binding, adjacency and reuse between tokens, and its loops." >}}
  {{< card link="context/" title="Context" subtitle="Every axis folded over the window, column, scope or key history a token is read against." >}}
{{< /cards >}}

## The taxonomy

| Family | Question | Axes |
|---|---|---|
| boundary | where to cut? | seam |
| character | what is here? | spectral (bytes), shape (tokens) |
| identity | what is the same? | orbit |
| scale | how much? | magnitude |
| load | how deep? | stress |
| dynamics | which way, how fast? | flow |
| vantage | do the past and the future agree? | observation |
| recurrence | does it return? | echo |
| arrangement | how do two positions stand to one another? | relation |
| context | against what? | context |

## Where each axis is read

| Axis | Command | Cmdlet | Rust | In a pattern |
|---|---|---|---|---|
| spectral | `trex spectral` | `Measure-TrexSpectral` | `trex::spectral` | `\F{...}` |
| seam | `trex seam` | `Measure-TrexSeam` | `trex::seam` | `@seam` |
| shape | `trex shape` | `Measure-TrexShape` | `trex::shape` | |
| orbit | `trex orbit` | `Measure-TrexOrbit` | `trex::orbit` | `(?orbit:G ...)` |
| magnitude | `trex magnitude` | `Measure-TrexMagnitude` | `trex::magnitude` | `\M{>k}` |
| stress | `trex stress` | `Measure-TrexStress` | `trex::stress` | `@nested>k` |
| flow | `trex flow` | `Measure-TrexFlow` | `trex::flow` | |
| observation | `trex observe` | `Measure-TrexObservation` | `trex::observation` | `@ambiguous` |
| echo | `trex echo` | `Measure-TrexEcho` | `trex::echo` | `@novel`, `@echoed`, `@echo` |
| relation | `trex relation` | `Measure-TrexRelation` | `trex::relation` | |
| context | | | `trex::context` | `\N{>+1:...}`, `@phase` |

Python reads the axes through the pattern forms.
