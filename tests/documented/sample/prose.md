# Reading a directory

The walk reports what it found rather than what it was asked for. A filter that
silently drops a file reads exactly the same as a directory that never held one, and
the reader cannot tell the two apart afterwards.

Every file the walk opens is named, and the kind beside it is the one covering the most
of its bytes. A file is rarely one thing throughout.

Consider a source file holding one long encoded literal. It is code by the count of its
regions and a blob by the weight of its bytes. The bytes are what a reader means when
they call a file one thing, so that is what the kind is taken from.

Where a file carries no strong shape at all, the spectral texture answers instead.
