---
title: How-To
linkTitle: How-To
weight: 2
sidebar:
  open: true
---

Recipes for one task each, with the command line, Rust, Python and PowerShell side by side where
each can do it. New to trex: start with the [tutorial](../tutorial/).

- [Build a pattern once and reuse it](build-and-reuse-a-pattern/) - mark the fields in one line, save the pattern, read new lines with it.
- [Query the records of a log](query-records/) - keep entries of several lines that hold, or lack, the patterns you name.
- [Count and rank matches](count-and-rank-matches/) - group by a rendered key and total a typed value per key.
- [Redact a log and follow it](redact-and-follow-logs/) - mask, keep what identifies a row, mask what the log gains.
- [Search a tree of files](search-a-tree-of-files/) - walk a directory, keep files by glob, list the matching files.
- [Index a tree](index-a-tree/) - skip the files a pattern cannot match without reading them.
- [Extract fields](extract-fields/) - typed values, key and value pairs, the columns of a table.
- [Lint with rules and fix](lint-with-rules/) - findings, CI annotations and fixes.
- [Test a pattern file](test-a-pattern-file/) - the texts each declaration must accept and reject.
- [Filter by typed values](filter-by-typed-values/) - durations, sizes, addresses and numbers by value.
- [Scan many patterns at once](scan-many-patterns-at-once/) - one read, each match under its pattern.
- [Explain why a pattern matched](explain-why-a-pattern-matched/) - kinds, checks, axis readings and the route.
- [Match inside encoded content](match-inside-encoded-content/) - JWT headers and claims, base64 contents.
- [Summarize a log by templates](summarize-a-log-by-templates/) - the line shapes a log repeats, and their lines.
- [Scan input in pieces](scan-input-in-pieces/) - matches settled chunk by chunk.
- [Rename or reshape matches](rename-matches/) - templates, typed slices, computed replacements, files.
- [Declare your own atoms](declare-your-own-atoms/) - your identifiers as tokens: test, find, count, mask.
- [Find tables](find-tables/) - tables by their repeating token shape, in text and in a tree.
- [Segment without delimiters](segment-without-delimiters/) - cut where the text stops predicting itself.
- [Find outliers](find-outliers/) - values out of scale with their window, column or key.
- [Match up to a symmetry](match-up-to-a-symmetry/) - case, word shape, subnet.
- [Parse with a token grammar](parse-with-a-token-grammar/) - rules over tokens, precedence, ambiguity.
- [Split run-together words](split-run-together-words/) - every tiling over a dictionary.
- [Learn a subword tokenizer](learn-a-subword-tokenizer/) - byte-pair merges from a corpus.
- [Find deep nesting](find-deep-nesting/) - tokens past a bracket depth.
- [Find repeated or new content](find-repeated-or-new-content/) - first sightings, and what one log has that another lacks.
- [Find rare lines and out-of-order times](find-rare-lines-and-out-of-order-times/) - rare line shapes, timestamps running backward.
- [Find where text changes kind](find-where-text-changes-kind/) - code inside prose, data inside code.
- [Pre-check a corpus for a literal](pre-check-a-corpus-for-a-literal/) - a presence filter answering without a scan.
- [Measure how far text compresses](compress-text/) - the code length of a context-mixing model.
- [Choose a backend](choose-a-backend/) - CPU, device, chunks, and the compressor's backends.
