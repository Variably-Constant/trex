#!/usr/bin/perl
# Perl's own regular expressions over the corpus and the patterns
# `vs_regex_full` uses.
#
# The counterpart of `other_engines.py`, and its output is the same tab-separated
# rows, so one reader merges both beside the Rust comparison. The corpus formula
# is `vs_regex_full.rs`'s, statement for statement: a comparison over different
# bytes is not one.
#
# Perl is a backtracking engine, so a pattern the others answer in linear time
# can cost it more. None of these patterns has a construct that backtracks
# catastrophically; the point of the row is what the common tool costs, not to
# find a pathological case.
#
# Run: perl benches/other_engines.pl [statements], default 200000.

use strict;
use warnings;
use Time::HiRes qw(time);

my $statements = $ARGV[0] // 200_000;

# Statements of four shapes, the corpus `benches/vs_regex_full.rs` builds.
sub corpus {
    my ($n) = @_;
    my $s = '';
    for my $i (0 .. $n - 1) {
        my $k = $i % 4;
        if    ($k == 0) { $s .= sprintf("let value_%d = %d ;\n", $i, $i * 37) }
        elsif ($k == 1) { $s .= sprintf("call_%d(alpha, beta, %d) ;\n", $i, $i) }
        elsif ($k == 2) { $s .= sprintf("key_%d: item_%d, item_%d, item_%d ;\n", $i, $i, $i + 1, $i + 2) }
        else            { $s .= sprintf("if (cond_%d) { do_%d(x) ; }\n", $i, $i) }
    }
    return $s;
}

my $text = corpus($statements);

# The patterns `vs_regex_full.rs` pairs with each trex pattern, in Perl's
# spelling. `\b[A-Za-z_][A-Za-z_0-9]*\b` and the rest are the same expression;
# the line anchor is `/m` rather than an inline `(?m)`.
# The third field says whether the pattern captures, which decides whether it
# gets a split row. It is written out rather than detected: a detector that got
# it wrong would print a row silently answering a different question.
my @PAIRS = (
    ['a literal word',                    qr/\balpha\b/,                                            0],
    ['a literal word that is absent',     qr/\bzzzqqq\b/,                                           0],
    ['any word token',                    qr/\b[A-Za-z_][A-Za-z_0-9]*\b/,                           0],
    ['any number token',                  qr/\b[0-9]+\b/,                                           0],
    ['either of two literals',            qr/\b(?:alpha|beta)\b/,                                   0],
    ['a word then punctuation',           qr/\b[A-Za-z_][A-Za-z_0-9]*\b =/,                         0],
    ['a byte-pattern inside a token',     qr/\bcond_[0-9]+\b/,                                      0],
    ['a literal, a word and punctuation', qr/\blet\b [A-Za-z_][A-Za-z_0-9]* =/,                     0],
    ['the same, with the word bound',     qr/\blet\b ([A-Za-z_][A-Za-z_0-9]*) =/,                   1],
    ['a bounded repeat of a token',       qr/\b[A-Za-z_][A-Za-z_0-9]*\b [A-Za-z_][A-Za-z_0-9]*\b/,  0],
    ['a line-anchored literal',           qr/^let\b/m,                                              0],
    ['a named capture then punctuation',  qr/([A-Za-z_][A-Za-z_0-9]*) =/,                           1],
);

my $ROUNDS    = 5;
my $BUDGET_MS = 25.0;

# Call the closure until the budget has passed and at least five calls have run,
# which is the Rust harness's shape, so the milliseconds are comparable.
sub time_budget {
    my ($f) = @_;
    my $answer = $f->();
    my $t0    = time();
    my $iters = 0;
    while (1) {
        $f->();
        $iters++;
        last if $iters >= 5 && (time() - $t0) * 1e3 >= $BUDGET_MS;
    }
    return ((time() - $t0) * 1e3 / $iters, $answer);
}

sub median {
    my @v = sort { $a <=> $b } @_;
    return $v[int(@v / 2)];
}

printf("# perl %vd, corpus %d bytes, %d statements\n", $^V, length($text), $statements);

for my $pair (@PAIRS) {
    my ($name, $rx, $captures) = @$pair;
    # `split` yields the captured groups between the pieces, so a pattern with
    # one answers a different question from the Rust harness's split; those
    # patterns are left without a split row rather than given a wrong one.
    my %ops = (
        'does it match at all'   => sub { $text =~ $rx ? 1 : 0 },
        'where it first matches' => sub { $text =~ $rx ? $-[0] + 1 : 0 },
        'every match'            => sub { my $n = 0; $n++ while $text =~ /$rx/g; $n },
        'replace every match'    => sub { my $c = $text; $c =~ s/$rx/X/g; length $c },
    );
    unless ($captures) {
        $ops{'split on every match'} = sub { scalar(my @p = split /$rx/, $text, -1) };
        $ops{'split into at most four pieces'} = sub { scalar(my @p = split /$rx/, $text, 4) };
    }
    # Every operation timed once a round in a rotating order, for the reason the
    # Rust harness rotates: position inside a run is worth a few percent.
    my @keys = sort keys %ops;
    my (%times, %answers);
    for my $r (0 .. $ROUNDS - 1) {
        for my $step (0 .. $#keys) {
            my $k = $keys[($r + $step) % @keys];
            my ($ms, $answer) = time_budget($ops{$k});
            push @{ $times{$k} }, $ms;
            $answers{$k} = $answer;
        }
    }
    for my $k (@keys) {
        printf("perl\t%s\t%s\t%.5f\t%s\n", $name, $k, median(@{ $times{$k} }), $answers{$k});
    }
}
