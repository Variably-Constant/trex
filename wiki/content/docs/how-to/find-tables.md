---
weight: 30
---

# Find tables with no delimiters

A ragged table defeats a byte-period detector: the field widths vary, so the comma offsets
shift every row. The [shape axis](../../reference/axes/shape/) autocorrelates *silhouettes*
instead of bytes, where width variation has already collapsed (a field is one number token
whatever its width), so the row template still repeats.

```console
$ trex shape --text $'1,22,3\n444,5,66\n7,888,9'
trex shape: 23 bytes, 15 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 5 tokens  (strength 1.00)
```

The strong period (the `N , N , N` silhouette, one template region at full strength) is the
table signal. Add `--period` to print the byte spans of the template regions, or `--classes`
for the per-token shape / period / novelty.

## Fuse with texture

`trex spectral --code-classify` fuses the shape period with the spectral texture, so a tabular
block reads as one `table` region rather than a texture-shredded `data` run. Use it when you
want a whole-file region map rather than the shape field alone.
