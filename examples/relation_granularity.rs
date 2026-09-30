//! The relation graph read at two granularities, side by side.
//!
//! The relation tier finds its edges over tokens, where a node is a word, a
//! number or a bracket. A supertoken node is a call, a binding, a key/value or
//! a list. Contracting the token graph onto supertoken nodes reads the same
//! relations at the larger scale, so the two columns below are one structure
//! seen twice.
//!
//! Run with `cargo run --example relation_granularity`.

use trex::relation::NodeMap;

fn report(label: &str, src: &str) {
    let bytes = src.as_bytes();
    let toks = trex::lexer::lex(bytes);
    let units = trex::supertokens(bytes);

    let per_token = NodeMap::per_token(toks.len());
    let per_unit = NodeMap::per_supertoken(&units, &toks);

    let field = trex::relation::analyze(&toks, bytes);
    let tok_edges = field.contracted_edges(&per_token);
    let unit_edges = field.contracted_edges(&per_unit);

    let t = trex::topology::of_edges(&tok_edges);
    let u = trex::topology::of_edges(&unit_edges);

    let ct = trex::curvature::analyze(&toks, bytes);
    let cu = trex::curvature::analyze_over(&toks, bytes, &per_unit);
    let gt = trex::geodesic::analyze(&toks, bytes);
    let gu = trex::geodesic::analyze_over(&toks, bytes, &per_unit);

    println!("{label}");
    println!("  {} bytes, {} tokens, {} supertokens", bytes.len(), toks.len(), units.len());
    println!("                       tokens    supertokens");
    println!("  nodes             {:>9}    {:>11}", t.nodes, u.nodes);
    println!("  edges             {:>9}    {:>11}", t.edges, u.edges);
    println!("  components (b0)   {:>9}    {:>11}", t.components, u.components);
    println!("  cycle rank (b1)   {:>9}    {:>11}", t.cycle_rank, u.cycle_rank);
    println!("  min curvature     {:>9}    {:>11}", ct.min_ricci(), cu.min_ricci());
    println!("  bridge edges      {:>9}    {:>11}", ct.bridges(), cu.bridges());
    println!("  geodesic diameter {:>9}    {:>11}", gt.diameter(), gu.diameter());
    println!("  max shortcut      {:>9}    {:>11}", gt.max_shortcut(), gu.max_shortcut());
    println!();
}

fn main() {
    println!("The same relation graph, contracted onto two node sets.\n");

    report(
        "code: nested calls and a rebound name",
        "let total = sum(price, tax);\nlet net = round(total);\nprint(net, total);\n",
    );
    report(
        "config: repeated keys across blocks",
        "server {\n  host: example.com\n  port: 8080\n}\nclient {\n  host: example.com\n  port: 9090\n}\n",
    );
    report(
        "prose: no brackets, no reuse of structure",
        "the quick brown fox jumps over the lazy dog\nand then the dog looks up at the fox\n",
    );
    report(
        "flat log lines: adjacency and repeated fields",
        "INFO start id=1\nINFO stop id=1\nWARN retry id=2\n",
    );

    println!("Reading the columns:");
    println!("  A token graph's nodes are words, numbers and brackets, and its");
    println!("  adjacency backbone alone puts an edge between every consecutive");
    println!("  pair, so b1 counts cycles closed against that backbone.");
    println!("  A supertoken graph's nodes are calls, bindings and key/value");
    println!("  entries, so an edge means one construct bears on another and b1");
    println!("  counts independent dependencies among constructs.");
}
