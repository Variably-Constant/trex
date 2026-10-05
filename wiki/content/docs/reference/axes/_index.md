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
  {{< card link="stress/" title="Stress" subtitle="Bracket depth at each token, how long it has been held open, and where the depth drops." >}}
  {{< card link="flow/" title="Flow" subtitle="The slope, direction and momentum of a per-token signal, and where it reverses." >}}
  {{< card link="observation/" title="Observation" subtitle="Byte-class entropy of the past, the future and both, and where past and future disagree." >}}
  {{< card link="echo/" title="Echo" subtitle="Whether each token's content recurs, how often, how far away and how regularly." >}}
  {{< card link="relation/" title="Relation" subtitle="The graph of enclosure, binding, adjacency and reuse between tokens, and its loops." >}}
  {{< card link="gravity/" title="Gravity" subtitle="Which types pull each other closer than chance or push apart, where a unit's past pushes it away, and where the input holds together least." >}}
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
| arrangement | how are two positions related? | relation |
| affinity | what pulls together, and what pushes apart? | gravity |
| context | against what? | context |

## Where each axis is read

| Axis | Command | Cmdlet | Python | Rust | In a pattern |
|---|---|---|---|---|---|
| spectral | `trex spectral` | `Measure-TrexSpectral` | `trex.axes.spectral` | `trex::spectral` | `\F{...}` |
| seam | `trex seam` | `Measure-TrexSeam` | `trex.axes.seam` | `trex::seam` | `@seam` |
| shape | `trex shape` | `Measure-TrexShape` | `trex.axes.shape` | `trex::shape` | |
| orbit | `trex orbit` | `Measure-TrexOrbit` | `trex.axes.orbit` | `trex::orbit` | `(?orbit:G ...)` |
| magnitude | `trex magnitude` | `Measure-TrexMagnitude` | `trex.axes.magnitude` | `trex::magnitude` | `\M{>k}` |
| stress | `trex stress` | `Measure-TrexStress` | `trex.axes.stress` | `trex::stress` | `@nested>k` |
| flow | `trex flow` | `Measure-TrexFlow` | `trex.axes.flow` | `trex::flow` | |
| observation | `trex observe` | `Measure-TrexObservation` | `trex.axes.observation` | `trex::observation` | `@ambiguous` |
| echo | `trex echo` | `Measure-TrexEcho` | `trex.axes.echo` | `trex::echo` | `@novel`, `@echoed`, `@echo` |
| relation | `trex relation` | `Measure-TrexRelation` | `trex.axes.relation` | `trex::relation` | |
| gravity | `trex gravity` | `Measure-TrexGravity` | `trex.axes.gravity` | `trex::gravity` | `@strain`, `@bound`, `@kin`, `=kin` |
| context | `trex context` | `Measure-TrexContext` | `trex.axes.context` | `trex::context` | `\N{>+1:...}`, `@phase` |

A cmdlet and a Python function give the same report: its summary readings, the points it marks
as frames, and every frame on request. Offsets are UTF-16 code units in PowerShell and the
input's units in Python, characters of a str and bytes of bytes.
