---
title: Explanation
linkTitle: Explanation
weight: 4
sidebar:
  open: true
---

How trex reads input and runs a match.

{{< cards >}}
  {{< card link="why-tokens/" title="Why tokens" subtitle="What a typed-token alphabet makes expressible that a byte alphabet does not, and what lexing first costs." icon="light-bulb" >}}
  {{< card link="axes/" title="The axes" subtitle="What an axis reads, what each of the twelve reads that the others do not, and how they combine with patterns and with each other." icon="chart-bar" >}}
  {{< card link="the-engine/" title="The engine" subtitle="The routes and the two engines behind one entry point, which patterns each takes, and what each costs." icon="cog" >}}
  {{< card link="dual-grain/" title="Dual-grain scanning" subtitle="Lexing and matching as two grains over the same bytes, run on two threads as a producer and a consumer." icon="view-grid" >}}
  {{< card link="architecture/" title="Architecture" subtitle="Every public module, the execution surfaces a pattern is reachable through, and parity with the regex crate." icon="template" >}}
{{< /cards >}}
