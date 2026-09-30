---
title: Tutorial
linkTitle: Tutorial
weight: 1
sidebar:
  open: true
---

New to trex? Start here. The tutorial takes you from a first scan to advanced patterns, one
short chapter at a time. Every example is a real command with its real output; run them as you
read.

Read the pages in order:

1. [Getting started](getting-started/) - build the binary, run your first scan, understand the
   output.
2. [Your first patterns](first-patterns/) - atoms, sequences, alternation, quantifiers, and
   lenses.
3. [Binding and balance](binding-and-balance/) - named registers with back-reference, and
   balanced bracket groups: the part a regex cannot do.
4. [Axes and tools](axes-and-tools/) - the property axes (scale, symmetry, segmentation) and
   the rewrite / grammar / prefilter tools.
5. [Beyond regex](beyond-regex/) - the rest of the regex surface (lookaround, atomic groups,
   the three kinds of choice, `\G`, `\K`, symmetry scopes) and what only trex can say: shapes
   you declare, construct anchors, and thresholds read from the stream itself.

By the end you can read and write any trex pattern and know which of the sixteen commands to
reach for.
