//! Physical quantities over real inputs: the lexer's attached and spaced
//! forms, the sign, the class `\{qty}` over byte sizes, durations and
//! percentages, the unit kinds read from the context, the predicates in a
//! family's base unit, the set form, the rewrite slices, and the agreement of
//! the byte route and the parallel lexer with the serial lex.

use trex::{parse, scan};

fn texts(pattern: &trex::ast::Pattern, bytes: &[u8]) -> Vec<String> {
    scan(pattern, bytes)
        .into_iter()
        .map(|s| String::from_utf8_lossy(&bytes[s.range()]).into_owned())
        .collect()
}

/// The matched texts of `pattern` over `input`, through the front door and
/// through the set engine, which must agree.
fn found(pattern: &str, input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    let direct = texts(&pat, bytes);
    let forced = parse(&format!("({pattern} |> [\\N && \\W])"))
        .unwrap_or_else(|e| panic!("forced {pattern}: {e:?}"));
    let through_set = texts(&forced, bytes);
    assert_eq!(direct, through_set, "{pattern}: the two engines disagree");
    direct
}

#[test]
fn a_predicate_compares_in_the_family_and_only_there() {
    let text = "5000 g, 6 kg, 4.9kg, 12 lb, 6 m, 0.1 kg, 100g";
    assert_eq!(found("\\{qty}{>5kg}", text), vec!["6 kg", "12 lb"]);
    assert_eq!(found("\\{qty}{=100g}", text), vec!["0.1 kg", "100g"]);
    assert_eq!(found("\\{qty}{5kg..6kg}", text), vec!["5000 g", "6 kg", "12 lb"]);
    assert_eq!(found("\\{qty}{>5kg,<6kg}", text), vec!["12 lb"]);
    // Six quantities are here; `6 m` is a number and a word, since a single
    // letter after a space is read from the context and this text holds no
    // length cue.
    assert_eq!(found("\\{qty}", text).len(), 6);
    assert_eq!(found("\\{qty}{!=6kg}", text).len(), 5);
    assert_eq!(found("\\{qty}{>3m}", "5 km, 5m, 400 cm, 2 m/s, 1 mi"), vec!["5 km", "400 cm", "1 mi"]);
    assert_eq!(
        found("\\{qty}{=435cm}", "a rod 4.35 m long, then 4.350 m and 4.36 m"),
        vec!["4.35 m", "4.350 m"]
    );
}

#[test]
fn the_class_reads_bytes_durations_and_percentages_in_their_own_units() {
    assert_eq!(found("\\{qty}{>1GiB}", "512MB 2GiB 1.5 GB 1000 MB"), vec!["2GiB", "1.5 GB"]);
    // `90m` is a duration and so a time cue, which makes the bare `s` and `h`
    // of `7200 s` and `3 h` quantities too; two hours exactly clears neither.
    assert_eq!(
        found("\\{qty}{>2h}", "90m 3h 150 min 1h59m 7200 s 3 h"),
        vec!["3h", "150 min", "3 h"]
    );
    assert_eq!(found("\\{qty}{>=40%}", "35% 40 % 50% 3 %"), vec!["40 %", "50%"]);
    assert_eq!(found("\\{qty}{family:data}", "512MB 2GiB 1.5 GB 5 Mbps"), vec!["512MB", "2GiB", "1.5 GB"]);
    // The letter atoms keep their meaning: attached only.
    assert_eq!(found("\\Z", "512MB 1.5 GB"), vec!["512MB"]);
    assert_eq!(found("\\R", "90m 150 min"), vec!["90m"]);
    assert_eq!(found("\\%", "35% 40 %"), vec!["35%"]);
    assert_eq!(found("\\{quantity}", "512MB 1.5 GB 90m 150 min 35% 40 %"), vec!["1.5 GB", "150 min", "40 %"]);
}

#[test]
fn temperatures_compare_across_scales_and_a_sign_belongs_to_the_quantity() {
    let text = "temperature 86\u{b0}F, 50\u{b0}F, 305 K, 20\u{b0}C, -40\u{b0}C, 10-20\u{b0}C";
    assert_eq!(found("\\{qty}{>=30\u{b0}C}", text), vec!["86\u{b0}F", "305 K"]);
    assert_eq!(found("\\{qty}{<0\u{b0}C}", text), vec!["-40\u{b0}C"]);
    assert_eq!(found("\\{qty}{<300K}", text), vec!["50\u{b0}F", "20\u{b0}C", "-40\u{b0}C", "20\u{b0}C"]);
    assert_eq!(found("\\N", "10-20kg and x-5kg"), vec!["10"]);
    assert_eq!(found("\\{qty}", "10-20kg and x-5kg and (-5kg)"), vec!["20kg", "5kg", "-5kg"]);
    assert_eq!(found("\\{qty}{<0dB}", "\u{2212}5 dB and 3 dB"), vec!["\u{2212}5 dB"]);
}

#[test]
fn the_unit_and_family_fields_read_the_symbol_as_written() {
    let text = "5kg, 5000 g, 3 kW, 5 m/s, 20\u{b0}C";
    assert_eq!(found("\\{qty}{unit:kg}", text), vec!["5kg"]);
    assert_eq!(found("\\{qty}{unit:k*}", text), vec!["5kg", "3 kW"]);
    assert_eq!(found("\\{qty}{family:mass}", text), vec!["5kg", "5000 g"]);
    assert_eq!(found("\\{qty}{family:speed}", text), vec!["5 m/s"]);
    assert_eq!(found("\\{qty}{unit:\u{b0}C}", text), vec!["20\u{b0}C"]);
    assert_eq!(found("\\{qty}{family:temperature,>0\u{b0}C}", text), vec!["20\u{b0}C"]);
}

#[test]
fn the_context_kinds_read_a_bare_symbol_beside_a_cue() {
    // A cue reaches every bare symbol of its family inside its window, so
    // each reading is in a text of its own.
    assert_eq!(found("\\{qty}", "cooled to 4.2K overnight"), vec!["4.2K"]);
    assert!(found("\\{qty}", "a 5K run this week").is_empty());
    assert_eq!(found("\\{qty}{family:temperature}", "cooled to 4.2K overnight"), vec!["4.2K"]);
    assert_eq!(found("\\{qty}{<5K}", "cooled to 4.2K overnight"), vec!["4.2K"]);
    assert_eq!(found("\\{qty}", "a board 3 in wide"), vec!["3 in"]);
    assert!(found("\\{qty}", "three 3 in a row").is_empty());
    assert_eq!(found("\\{qty}{>1h}", "2 h elapsed"), vec!["2 h"]);
    assert!(found("\\{qty}", "see figure 2 h").is_empty());
    // A pattern naming no quantity kind still sees the number and the word.
    assert_eq!(found("\\N \"K\"", "cooled to 4.2K and a 5K run"), vec!["4.2K", "5K"]);
    // A declaration shadows the library's kind, and the class takes the declaration.
    let mut set = trex::ShapeSet::new();
    set.declare_text("kind kelvin = \\N \"K\" ~>2(\"kelvin\")").expect("declares");
    let shadowed = trex::parser::parse_with_shapes("\\{qty}{family:temperature}", &set).expect("parses");
    let shaped = trex::engine::scan_with_shapes(&shadowed, b"cooled to 4.2K and 5K kelvin", &set);
    assert_eq!(shaped.len(), 1);
    assert_eq!(shaped[0].range(), 19..21, "the declared reading, not the library's");
}

#[test]
fn a_rewrite_slices_the_value_and_the_unit() {
    let pattern = parse("\\{qty}:q").expect("parses");
    let template = trex::Template::parse("${q:value} [${q:unit}]", &pattern.capture_names()).expect("a template");
    let out = trex::rewrite(&pattern, &template, "mass 5.50 kg at -40\u{b0}C over 3h20m".as_bytes());
    assert_eq!(String::from_utf8_lossy(&out), "mass 5.50 [kg] at -40 [\u{b0}C] over 3 [h20m]");
}

#[test]
fn a_set_of_quantities_holds_by_value_in_the_base_unit() {
    let dir = std::env::temp_dir().join(format!("trex/quantities-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the set directory");
    let path = dir.join("limits.txt");
    std::fs::write(&path, "5kg\n100 g\n2h\n").expect("write the set");
    let spec = path.to_string_lossy().replace('\\', "/");
    let text = "5000 g, 0.1 kg, 120 min, 5 km, 3h";
    assert_eq!(found(&format!("\\{{qty}}{{in:@{spec}}}"), text), vec!["5000 g", "0.1 kg", "120 min"]);
    let bad = dir.join("bad.txt");
    std::fs::write(&bad, "5kg\nfive\n").expect("write the set");
    let err = parse(&format!("\\{{qty}}{{in:@{}}}", bad.to_string_lossy().replace('\\', "/")))
        .expect_err("a line that is not a quantity is refused");
    assert!(err.msg.contains("line 2"), "{}", err.msg);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) => panic!("cannot remove {}: {e}", dir.display()),
    }
}

#[test]
fn the_byte_route_and_the_parallel_lexer_agree_with_the_serial_lex() {
    // A newline-free input past the parallel threshold, so every cut comes
    // from the whitespace rule, with a spaced quantity behind many of them.
    let line = "mass 5 kg at 3.2 GHz and 40 % or -40\u{b0}C over 5 m/s with 5 items, 3 in a row, 10-20kg; ";
    let mut input = String::new();
    while input.len() < 80 * 1024 {
        input.push_str(line);
    }
    let serial = trex::lexer::lex(input.as_bytes());
    assert_eq!(trex::parallel_lex::lex_parallel(input.as_bytes()), serial);
    let number = parse("\\N").expect("parses");
    let engine_first = scan(&number, input.as_bytes()).into_iter().next();
    assert_eq!(trex::find(&number, input.as_bytes()), engine_first);
    let quantity = parse("\\{quantity}").expect("parses");
    assert_eq!(scan(&quantity, line.as_bytes()).len(), 6);
    assert_eq!(trex::find(&quantity, line.as_bytes()).map(|s| s.range()), Some(5..9));
}
