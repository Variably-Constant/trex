//! Physical quantities: a number with a unit symbol, read as one token and
//! compared within the unit's family after an exact normalization to the
//! family's base unit.
//!
//! The table holds the SI units with their prefixed forms, the customary
//! units the standards define exactly against them, and the units of
//! computing: bytes, bit rates, request rates. Every factor is a terminating
//! decimal against its family's base, and each base is chosen for that: the
//! degree Fahrenheit for temperature, the count per minute for frequency, the
//! meter per hour for speed, a pascal split into 64516 parts for pressure. A
//! quantity therefore compares exactly, as a number does, and `0.1 kg` is
//! `100 g` to the digit. Radians and the typographic px, pt and em have no
//! terminating factor and are not units here.
//!
//! The lexer reads a symbol attached to its number (`5kg`, `20°C`), and after
//! one space, no-break space, narrow no-break space or thin space when the
//! symbol has two or more characters or holds a non-letter (`5 kg`,
//! `3.2 GHz`, `40 %`, `5 m/s`) and is not an English word. A symbol left to
//! the tokens around it - the kelvin's `K`, `in`, `bar`, `cal`, `gal`, and
//! every single letter after a space - is read by a library kind from the
//! context ([`crate::library`]); the parser here accepts every symbol, so a
//! token so read and a predicate's value read in the same table. A leading
//! sign belongs to the quantity when nothing alphanumeric precedes it, so
//! `-40°C` is one token at minus forty and `10-20kg` keeps its range dash.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::token::TokenKind;
use crate::typed::Decimal;

/// The dimension a unit measures. A quantity compares only with one of its
/// own family, whatever units the two were written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    Mass,
    Length,
    Area,
    Volume,
    Temperature,
    Time,
    Frequency,
    /// A byte count, as `\Z` reads it.
    Data,
    /// Bits or bytes per second.
    DataRate,
    /// A dimensionless share: percent, per mille, parts per million.
    Ratio,
    Voltage,
    Current,
    Power,
    Resistance,
    Energy,
    Charge,
    Capacitance,
    Inductance,
    Pressure,
    Force,
    Torque,
    Speed,
    /// Events per second: requests, operations, frames, packets.
    Throughput,
    Angle,
    LuminousFlux,
    Illuminance,
    /// Amount of substance, in moles.
    Amount,
    /// A level in decibels.
    Level,
    /// A power level in decibel-milliwatts.
    PowerLevel,
    /// Dots or pixels per inch.
    Resolution,
}

impl Family {
    /// Every family, in the order the reference lists them.
    pub const ALL: [Family; 30] = [
        Family::Mass,
        Family::Length,
        Family::Area,
        Family::Volume,
        Family::Temperature,
        Family::Time,
        Family::Frequency,
        Family::Data,
        Family::DataRate,
        Family::Ratio,
        Family::Voltage,
        Family::Current,
        Family::Power,
        Family::Resistance,
        Family::Energy,
        Family::Charge,
        Family::Capacitance,
        Family::Inductance,
        Family::Pressure,
        Family::Force,
        Family::Torque,
        Family::Speed,
        Family::Throughput,
        Family::Angle,
        Family::LuminousFlux,
        Family::Illuminance,
        Family::Amount,
        Family::Level,
        Family::PowerLevel,
        Family::Resolution,
    ];

    /// The name a `family:` clause compares against.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Family::Mass => "mass",
            Family::Length => "length",
            Family::Area => "area",
            Family::Volume => "volume",
            Family::Temperature => "temperature",
            Family::Time => "time",
            Family::Frequency => "frequency",
            Family::Data => "data",
            Family::DataRate => "datarate",
            Family::Ratio => "ratio",
            Family::Voltage => "voltage",
            Family::Current => "current",
            Family::Power => "power",
            Family::Resistance => "resistance",
            Family::Energy => "energy",
            Family::Charge => "charge",
            Family::Capacitance => "capacitance",
            Family::Inductance => "inductance",
            Family::Pressure => "pressure",
            Family::Force => "force",
            Family::Torque => "torque",
            Family::Speed => "speed",
            Family::Throughput => "throughput",
            Family::Angle => "angle",
            Family::LuminousFlux => "luminous",
            Family::Illuminance => "illuminance",
            Family::Amount => "amount",
            Family::Level => "level",
            Family::PowerLevel => "powerlevel",
            Family::Resolution => "resolution",
        }
    }

    /// The family called `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Family> {
        Family::ALL.iter().copied().find(|f| f.name() == name)
    }

    /// The unit every value of the family is normalized to, chosen so that
    /// every unit's factor against it terminates.
    #[must_use]
    pub fn base(self) -> &'static str {
        match self {
            Family::Mass => "g",
            Family::Length => "m",
            Family::Area => "m\u{b2}",
            Family::Volume => "m\u{b3}",
            Family::Temperature => "\u{b0}F",
            Family::Time => "ns",
            Family::Frequency => "/min",
            Family::Data => "B",
            Family::DataRate => "bit/s",
            Family::Ratio => "ppb",
            Family::Voltage => "V",
            Family::Current => "A",
            Family::Power => "W",
            Family::Resistance => "\u{3a9}",
            Family::Energy => "J",
            Family::Charge => "C",
            Family::Capacitance => "F",
            Family::Inductance => "H",
            Family::Pressure => "Pa/64516",
            Family::Force => "N",
            Family::Torque => "N\u{b7}m",
            Family::Speed => "m/h",
            Family::Throughput => "/s",
            Family::Angle => "\u{b0}",
            Family::LuminousFlux => "lm",
            Family::Illuminance => "lx",
            Family::Amount => "mol",
            Family::Level => "dB",
            Family::PowerLevel => "dBm",
            Family::Resolution => "dpi",
        }
    }
}

/// One unit: its symbol as written and its exact factor to the family's
/// base, `mantissa` times ten to the `exp`, with the offset a temperature
/// scale adds after scaling.
struct UnitDef {
    symbol: &'static str,
    family: Family,
    mantissa: u128,
    exp: i32,
    offset: &'static str,
}

const fn u(symbol: &'static str, family: Family, mantissa: u128, exp: i32) -> UnitDef {
    UnitDef { symbol, family, mantissa, exp, offset: "0" }
}

const fn temperature(symbol: &'static str, mantissa: u128, exp: i32, offset: &'static str) -> UnitDef {
    UnitDef { symbol, family: Family::Temperature, mantissa, exp, offset }
}

/// Every unit. The micro prefix is accepted as the micro sign, the Greek mu
/// and the ASCII `u`; the ohm as its two code points and the word; a squared
/// or cubed unit as the superscript and the digit.
const UNITS: &[UnitDef] = &[
    // Mass, in grams.
    u("pg", Family::Mass, 1, -12),
    u("ng", Family::Mass, 1, -9),
    u("\u{b5}g", Family::Mass, 1, -6),
    u("\u{3bc}g", Family::Mass, 1, -6),
    u("ug", Family::Mass, 1, -6),
    u("mg", Family::Mass, 1, -3),
    u("g", Family::Mass, 1, 0),
    u("kg", Family::Mass, 1, 3),
    u("t", Family::Mass, 1, 6),
    u("kt", Family::Mass, 1, 9),
    u("Mt", Family::Mass, 1, 12),
    u("lb", Family::Mass, 45_359_237, -5),
    u("lbs", Family::Mass, 45_359_237, -5),
    u("oz", Family::Mass, 28_349_523_125, -9),
    // Length, in meters.
    u("nm", Family::Length, 1, -9),
    u("\u{b5}m", Family::Length, 1, -6),
    u("\u{3bc}m", Family::Length, 1, -6),
    u("um", Family::Length, 1, -6),
    u("mm", Family::Length, 1, -3),
    u("cm", Family::Length, 1, -2),
    u("dm", Family::Length, 1, -1),
    u("m", Family::Length, 1, 0),
    u("km", Family::Length, 1, 3),
    u("in", Family::Length, 254, -4),
    u("ft", Family::Length, 3048, -4),
    u("yd", Family::Length, 9144, -4),
    u("mi", Family::Length, 1_609_344, -3),
    u("nmi", Family::Length, 1852, 0),
    // Area, in square meters.
    u("mm\u{b2}", Family::Area, 1, -6),
    u("mm2", Family::Area, 1, -6),
    u("cm\u{b2}", Family::Area, 1, -4),
    u("cm2", Family::Area, 1, -4),
    u("m\u{b2}", Family::Area, 1, 0),
    u("m2", Family::Area, 1, 0),
    u("km\u{b2}", Family::Area, 1, 6),
    u("km2", Family::Area, 1, 6),
    u("ha", Family::Area, 1, 4),
    u("in\u{b2}", Family::Area, 64516, -8),
    u("in2", Family::Area, 64516, -8),
    u("ft\u{b2}", Family::Area, 9_290_304, -8),
    u("ft2", Family::Area, 9_290_304, -8),
    u("yd\u{b2}", Family::Area, 83_612_736, -8),
    u("yd2", Family::Area, 83_612_736, -8),
    u("mi\u{b2}", Family::Area, 2_589_988_110_336, -6),
    u("mi2", Family::Area, 2_589_988_110_336, -6),
    u("acre", Family::Area, 40_468_564_224, -7),
    u("acres", Family::Area, 40_468_564_224, -7),
    // Volume, in cubic meters.
    u("mm\u{b3}", Family::Volume, 1, -9),
    u("mm3", Family::Volume, 1, -9),
    u("cm\u{b3}", Family::Volume, 1, -6),
    u("cm3", Family::Volume, 1, -6),
    u("cc", Family::Volume, 1, -6),
    u("mL", Family::Volume, 1, -6),
    u("ml", Family::Volume, 1, -6),
    u("cL", Family::Volume, 1, -5),
    u("cl", Family::Volume, 1, -5),
    u("dL", Family::Volume, 1, -4),
    u("dl", Family::Volume, 1, -4),
    u("L", Family::Volume, 1, -3),
    u("l", Family::Volume, 1, -3),
    u("m\u{b3}", Family::Volume, 1, 0),
    u("m3", Family::Volume, 1, 0),
    u("gal", Family::Volume, 3_785_411_784, -12),
    u("qt", Family::Volume, 946_352_946, -12),
    u("in\u{b3}", Family::Volume, 16_387_064, -12),
    u("in3", Family::Volume, 16_387_064, -12),
    u("ft\u{b3}", Family::Volume, 28_316_846_592, -12),
    u("ft3", Family::Volume, 28_316_846_592, -12),
    // Temperature, in degrees Fahrenheit: the one base on which the Celsius
    // and kelvin scales both terminate.
    temperature("\u{b0}C", 18, -1, "32"),
    temperature("\u{2103}", 18, -1, "32"),
    temperature("\u{b0}F", 1, 0, "0"),
    temperature("\u{2109}", 1, 0, "0"),
    temperature("K", 18, -1, "-459.67"),
    // Time, in nanoseconds, as a duration is read.
    u("ps", Family::Time, 1, -3),
    u("ns", Family::Time, 1, 0),
    u("\u{b5}s", Family::Time, 1, 3),
    u("\u{3bc}s", Family::Time, 1, 3),
    u("us", Family::Time, 1, 3),
    u("ms", Family::Time, 1, 6),
    u("s", Family::Time, 1, 9),
    u("sec", Family::Time, 1, 9),
    u("secs", Family::Time, 1, 9),
    u("min", Family::Time, 6, 10),
    u("mins", Family::Time, 6, 10),
    u("h", Family::Time, 36, 11),
    u("hr", Family::Time, 36, 11),
    u("hrs", Family::Time, 36, 11),
    u("d", Family::Time, 864, 11),
    u("w", Family::Time, 6048, 11),
    u("y", Family::Time, 31536, 12),
    // Frequency, per minute, so a revolution a minute is one.
    u("mHz", Family::Frequency, 6, -2),
    u("Hz", Family::Frequency, 60, 0),
    u("kHz", Family::Frequency, 6, 4),
    u("MHz", Family::Frequency, 6, 7),
    u("GHz", Family::Frequency, 6, 10),
    u("THz", Family::Frequency, 6, 13),
    u("rpm", Family::Frequency, 1, 0),
    // Data, in bytes, with the decimal and binary prefixes `\Z` reads.
    u("B", Family::Data, 1, 0),
    u("kB", Family::Data, 1, 3),
    u("KB", Family::Data, 1, 3),
    u("kb", Family::Data, 1, 3),
    u("Kb", Family::Data, 1, 3),
    u("MB", Family::Data, 1, 6),
    u("mb", Family::Data, 1, 6),
    u("Mb", Family::Data, 1, 6),
    u("GB", Family::Data, 1, 9),
    u("gb", Family::Data, 1, 9),
    u("Gb", Family::Data, 1, 9),
    u("TB", Family::Data, 1, 12),
    u("tb", Family::Data, 1, 12),
    u("Tb", Family::Data, 1, 12),
    u("PB", Family::Data, 1, 15),
    u("pb", Family::Data, 1, 15),
    u("EB", Family::Data, 1, 18),
    u("KiB", Family::Data, 1024, 0),
    u("MiB", Family::Data, 1_048_576, 0),
    u("GiB", Family::Data, 1_073_741_824, 0),
    u("TiB", Family::Data, 1_099_511_627_776, 0),
    u("PiB", Family::Data, 1_125_899_906_842_624, 0),
    u("EiB", Family::Data, 1_152_921_504_606_846_976, 0),
    // Data rate, in bits per second; a byte is eight bits.
    u("bit/s", Family::DataRate, 1, 0),
    u("bps", Family::DataRate, 1, 0),
    u("kbps", Family::DataRate, 1, 3),
    u("Kbps", Family::DataRate, 1, 3),
    u("Mbps", Family::DataRate, 1, 6),
    u("Gbps", Family::DataRate, 1, 9),
    u("Tbps", Family::DataRate, 1, 12),
    u("kbit/s", Family::DataRate, 1, 3),
    u("Mbit/s", Family::DataRate, 1, 6),
    u("Gbit/s", Family::DataRate, 1, 9),
    u("B/s", Family::DataRate, 8, 0),
    u("kB/s", Family::DataRate, 8, 3),
    u("KB/s", Family::DataRate, 8, 3),
    u("MB/s", Family::DataRate, 8, 6),
    u("GB/s", Family::DataRate, 8, 9),
    u("TB/s", Family::DataRate, 8, 12),
    u("KiB/s", Family::DataRate, 8192, 0),
    u("MiB/s", Family::DataRate, 8_388_608, 0),
    u("GiB/s", Family::DataRate, 8_589_934_592, 0),
    // Ratio, in parts per billion.
    u("%", Family::Ratio, 1, 7),
    u("\u{2030}", Family::Ratio, 1, 6),
    u("ppm", Family::Ratio, 1, 3),
    u("ppb", Family::Ratio, 1, 0),
    // Voltage, in volts.
    u("\u{b5}V", Family::Voltage, 1, -6),
    u("\u{3bc}V", Family::Voltage, 1, -6),
    u("uV", Family::Voltage, 1, -6),
    u("mV", Family::Voltage, 1, -3),
    u("V", Family::Voltage, 1, 0),
    u("kV", Family::Voltage, 1, 3),
    u("MV", Family::Voltage, 1, 6),
    // Current, in amperes.
    u("pA", Family::Current, 1, -12),
    u("nA", Family::Current, 1, -9),
    u("\u{b5}A", Family::Current, 1, -6),
    u("\u{3bc}A", Family::Current, 1, -6),
    u("uA", Family::Current, 1, -6),
    u("mA", Family::Current, 1, -3),
    u("A", Family::Current, 1, 0),
    u("kA", Family::Current, 1, 3),
    // Power, in watts; a horsepower is 550 foot-pounds a second.
    u("\u{b5}W", Family::Power, 1, -6),
    u("\u{3bc}W", Family::Power, 1, -6),
    u("uW", Family::Power, 1, -6),
    u("mW", Family::Power, 1, -3),
    u("W", Family::Power, 1, 0),
    u("kW", Family::Power, 1, 3),
    u("MW", Family::Power, 1, 6),
    u("GW", Family::Power, 1, 9),
    u("TW", Family::Power, 1, 12),
    u("hp", Family::Power, 74_569_987_158_227_022, -14),
    // Resistance, in ohms.
    u("m\u{3a9}", Family::Resistance, 1, -3),
    u("m\u{2126}", Family::Resistance, 1, -3),
    u("mohm", Family::Resistance, 1, -3),
    u("\u{3a9}", Family::Resistance, 1, 0),
    u("\u{2126}", Family::Resistance, 1, 0),
    u("ohm", Family::Resistance, 1, 0),
    u("ohms", Family::Resistance, 1, 0),
    u("k\u{3a9}", Family::Resistance, 1, 3),
    u("k\u{2126}", Family::Resistance, 1, 3),
    u("kohm", Family::Resistance, 1, 3),
    u("M\u{3a9}", Family::Resistance, 1, 6),
    u("M\u{2126}", Family::Resistance, 1, 6),
    u("Mohm", Family::Resistance, 1, 6),
    u("G\u{3a9}", Family::Resistance, 1, 9),
    u("G\u{2126}", Family::Resistance, 1, 9),
    // Energy, in joules; the thermochemical calorie, the ISO British thermal
    // unit and the 2019 SI electronvolt.
    u("mJ", Family::Energy, 1, -3),
    u("J", Family::Energy, 1, 0),
    u("kJ", Family::Energy, 1, 3),
    u("MJ", Family::Energy, 1, 6),
    u("GJ", Family::Energy, 1, 9),
    u("TJ", Family::Energy, 1, 12),
    u("cal", Family::Energy, 4184, -3),
    u("kcal", Family::Energy, 4184, 0),
    u("Wh", Family::Energy, 36, 2),
    u("kWh", Family::Energy, 36, 5),
    u("MWh", Family::Energy, 36, 8),
    u("GWh", Family::Energy, 36, 11),
    u("TWh", Family::Energy, 36, 14),
    u("eV", Family::Energy, 1_602_176_634, -28),
    u("keV", Family::Energy, 1_602_176_634, -25),
    u("MeV", Family::Energy, 1_602_176_634, -22),
    u("GeV", Family::Energy, 1_602_176_634, -19),
    u("TeV", Family::Energy, 1_602_176_634, -16),
    u("BTU", Family::Energy, 105_505_585_262, -8),
    u("Btu", Family::Energy, 105_505_585_262, -8),
    // Charge, in coulombs.
    u("mAh", Family::Charge, 36, -1),
    u("Ah", Family::Charge, 36, 2),
    u("kAh", Family::Charge, 36, 5),
    u("C", Family::Charge, 1, 0),
    // Capacitance, in farads.
    u("pF", Family::Capacitance, 1, -12),
    u("nF", Family::Capacitance, 1, -9),
    u("\u{b5}F", Family::Capacitance, 1, -6),
    u("\u{3bc}F", Family::Capacitance, 1, -6),
    u("uF", Family::Capacitance, 1, -6),
    u("mF", Family::Capacitance, 1, -3),
    u("F", Family::Capacitance, 1, 0),
    // Inductance, in henries.
    u("nH", Family::Inductance, 1, -9),
    u("\u{b5}H", Family::Inductance, 1, -6),
    u("\u{3bc}H", Family::Inductance, 1, -6),
    u("uH", Family::Inductance, 1, -6),
    u("mH", Family::Inductance, 1, -3),
    u("H", Family::Inductance, 1, 0),
    // Pressure, in 1/64516 of a pascal: the pound-force over the square inch
    // (4.4482216152605 N over 0.00064516 m²) then terminates.
    u("Pa", Family::Pressure, 64516, 0),
    u("hPa", Family::Pressure, 64516, 2),
    u("kPa", Family::Pressure, 64516, 3),
    u("MPa", Family::Pressure, 64516, 6),
    u("GPa", Family::Pressure, 64516, 9),
    u("bar", Family::Pressure, 64516, 5),
    u("mbar", Family::Pressure, 64516, 2),
    u("psi", Family::Pressure, 44_482_216_152_605, -5),
    u("atm", Family::Pressure, 6_537_083_700, 0),
    u("mmHg", Family::Pressure, 860_142_714_646_614, -8),
    // Force, in newtons.
    u("mN", Family::Force, 1, -3),
    u("N", Family::Force, 1, 0),
    u("kN", Family::Force, 1, 3),
    u("MN", Family::Force, 1, 6),
    u("lbf", Family::Force, 44_482_216_152_605, -13),
    u("kgf", Family::Force, 980_665, -5),
    // Torque, in newton-meters.
    u("Nm", Family::Torque, 1, 0),
    u("N\u{b7}m", Family::Torque, 1, 0),
    u("kNm", Family::Torque, 1, 3),
    u("kN\u{b7}m", Family::Torque, 1, 3),
    // Speed, in meters per hour, on which the second, the mile and the
    // nautical mile all terminate.
    u("mm/s", Family::Speed, 36, -1),
    u("cm/s", Family::Speed, 36, 0),
    u("m/s", Family::Speed, 36, 2),
    u("km/h", Family::Speed, 1, 3),
    u("kmh", Family::Speed, 1, 3),
    u("kph", Family::Speed, 1, 3),
    u("km/s", Family::Speed, 36, 5),
    u("mph", Family::Speed, 1_609_344, -3),
    u("kn", Family::Speed, 1852, 0),
    u("ft/s", Family::Speed, 109_728, -2),
    // Throughput, per second.
    u("req/s", Family::Throughput, 1, 0),
    u("rps", Family::Throughput, 1, 0),
    u("qps", Family::Throughput, 1, 0),
    u("tps", Family::Throughput, 1, 0),
    u("ops/s", Family::Throughput, 1, 0),
    u("fps", Family::Throughput, 1, 0),
    u("iops", Family::Throughput, 1, 0),
    u("pps", Family::Throughput, 1, 0),
    // Angle, in degrees.
    u("\u{b0}", Family::Angle, 1, 0),
    u("deg", Family::Angle, 1, 0),
    // Light.
    u("lm", Family::LuminousFlux, 1, 0),
    u("lx", Family::Illuminance, 1, 0),
    // Amount of substance, in moles.
    u("nmol", Family::Amount, 1, -9),
    u("\u{b5}mol", Family::Amount, 1, -6),
    u("\u{3bc}mol", Family::Amount, 1, -6),
    u("umol", Family::Amount, 1, -6),
    u("mmol", Family::Amount, 1, -3),
    u("mol", Family::Amount, 1, 0),
    u("kmol", Family::Amount, 1, 3),
    // Levels.
    u("dB", Family::Level, 1, 0),
    u("dBm", Family::PowerLevel, 1, 0),
    // Resolution.
    u("dpi", Family::Resolution, 1, 0),
    u("ppi", Family::Resolution, 1, 0),
];

/// Symbols the lexer never reads: a library kind reads each from the tokens
/// around it, since a bare `K` is oftener a thousand than a kelvin and a
/// bare `C`, `F` or `H` a letter.
const CONTEXT_ONLY: &[&str] = &["K", "C", "F", "H"];

/// Symbols the lexer reads attached to the number only, being English words
/// after a space: `3 in a row`, `went to 5 bar`, `gave 5 us`.
const ATTACHED_ONLY: &[&str] = &["in", "bar", "cal", "gal", "us"];

/// How far the lexer reads a symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// Attached to the number, and after one separator.
    Spaced,
    /// Attached only: a single letter, or an English word.
    Attached,
    /// Not by the lexer; by a library kind, from the context.
    Context,
}

fn reach(symbol: &str) -> Reach {
    if CONTEXT_ONLY.contains(&symbol) {
        return Reach::Context;
    }
    let single_letter = symbol.len() == 1 && symbol.as_bytes()[0].is_ascii_alphabetic();
    if single_letter || ATTACHED_ONLY.contains(&symbol) {
        return Reach::Attached;
    }
    Reach::Spaced
}

/// The table as the lexer and the parser read it: every unit by its symbol,
/// and by its first byte the units opening with it, longest first, so the
/// first symbol that fits at a position is the longest.
struct Table {
    by_symbol: HashMap<&'static str, &'static UnitDef>,
    by_first: Vec<Vec<(&'static UnitDef, Reach)>>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut by_symbol = HashMap::with_capacity(UNITS.len());
        let mut by_first: Vec<Vec<(&'static UnitDef, Reach)>> = vec![Vec::new(); 256];
        for unit in UNITS {
            assert!(by_symbol.insert(unit.symbol, unit).is_none(), "one unit per symbol: {}", unit.symbol);
            by_first[usize::from(unit.symbol.as_bytes()[0])].push((unit, reach(unit.symbol)));
        }
        for list in &mut by_first {
            list.sort_by_key(|(u, _)| std::cmp::Reverse(u.symbol.len()));
        }
        Table { by_symbol, by_first }
    })
}

/// A quantity read from text: the family its unit belongs to and its value in
/// the family's base unit, exact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quantity {
    pub family: Family,
    /// The value in the family's base unit.
    pub base: Decimal,
    /// The value as written, before the unit.
    pub value: Decimal,
    /// The unit symbol as written.
    pub unit: String,
}

fn unit_value(unit: &UnitDef, written: &Decimal) -> Decimal {
    let scaled = written.times(unit.mantissa).shift(unit.exp);
    match Decimal::parse(unit.offset) {
        Some(offset) if !offset.is_zero() => scaled.add(&offset),
        _ => scaled,
    }
}

/// The length of a sign at `i`: `-`, `+`, or the minus sign U+2212.
fn sign_len(input: &[u8], i: usize) -> Option<usize> {
    match input.get(i)? {
        b'-' | b'+' => Some(1),
        0xE2 if input.get(i..i + 3) == Some(&[0xE2, 0x88, 0x92]) => Some(3),
        _ => None,
    }
}

/// The length of one separator at `i`: a space, a no-break space, a narrow
/// no-break space or a thin space.
pub(crate) fn separator_len(input: &[u8], i: usize) -> Option<usize> {
    match input.get(i)? {
        b' ' => Some(1),
        0xC2 if input.get(i + 1) == Some(&0xA0) => Some(2),
        0xE2 if matches!(input.get(i + 1..i + 3), Some(&[0x80, 0xAF]) | Some(&[0x80, 0x89])) => Some(3),
        _ => None,
    }
}

/// Whether a byte can open a unit symbol after a space: a letter, a byte
/// of a non-ASCII symbol, or the percent sign.
pub(crate) fn opens_a_symbol(c: Option<u8>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphabetic() || c >= 0x80 || c == b'%')
}

/// Whether a symbol ends at `k`: the next byte continues no word.
fn boundary_at(input: &[u8], k: usize) -> bool {
    match input.get(k) {
        None => true,
        Some(&c) if c.is_ascii_alphanumeric() || c == b'_' => false,
        Some(&c) if c >= 0x80 => crate::lexer::utf8_letter_at(input, k).is_none(),
        Some(_) => true,
    }
}

/// The longest symbol at `j` the lexer reads in this placement, ending at a
/// boundary, as its byte length. After a separator only a spaced-reach
/// symbol is read; attached, an attached-reach one as well.
fn symbol_len(input: &[u8], j: usize, spaced: bool) -> Option<usize> {
    let first = *input.get(j)?;
    table().by_first[usize::from(first)]
        .iter()
        .filter(|(_, r)| *r == Reach::Spaced || (!spaced && *r == Reach::Attached))
        .map(|(u, _)| u.symbol.as_bytes())
        .find(|sym| input.get(j..j + sym.len()) == Some(*sym) && boundary_at(input, j + sym.len()))
        .map(<[u8]>::len)
}

/// The length of the spaced-form symbol at `j` with a boundary after it: the
/// test the lexer applies after one separator, and what the byte route and
/// the parallel lexer's cut both ask, so that neither reads as a number what
/// the lexer makes a quantity.
pub(crate) fn spaced_symbol_len(input: &[u8], j: usize) -> Option<usize> {
    symbol_len(input, j, true)
}

/// A byte that can carry a token across a number's start, leaving the number
/// something other than a quantity's own.
fn joins_a_number(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, b'.' | b':' | b'-' | b'_' | b'%' | b'$' | b',' | b'/' | b'+' | b'#')
        || c >= 0x80
}

/// Whether the run at `i` is the unit symbol of a quantity whose number lies
/// one separator back: `Some(true)` when it is, so no token of the run's own
/// begins or ends inside it; `Some(false)` when no quantity reaches it; and
/// `None` where a byte before the number could carry another token across its
/// start, so whether the lexer reaches a quantity here at all is the lexer's
/// to say.
///
/// The number is found by walking its digits back from the separator and the
/// reading is [`recognize`]'s own, so this and the lexer cannot drift apart.
/// The `None` is what keeps it one-directional: it costs the lex that was
/// being paid anyway, where a wrong `Some` would be an answer the whole
/// input's lex contradicts.
pub(crate) fn joins_the_number_before(input: &[u8], i: usize) -> Option<bool> {
    // Nothing joins a run that is no unit symbol, whatever stands before it,
    // which is the answer at almost every run and the only one reached
    // without reading back over a number.
    if spaced_symbol_len(input, i).is_none() {
        return Some(false);
    }
    for len in [1usize, 2, 3] {
        let Some(j) = i.checked_sub(len) else {
            continue;
        };
        if separator_len(input, j) != Some(len) || j == 0 || !input[j - 1].is_ascii_digit() {
            continue;
        }
        let mut s = j - 1;
        while s > 0 && input[s - 1].is_ascii_digit() {
            s -= 1;
        }
        if s > 1 && input[s - 1] == b'.' && input[s - 2].is_ascii_digit() {
            s -= 2;
            while s > 0 && input[s - 1].is_ascii_digit() {
                s -= 1;
            }
        }
        if s > 0 && joins_a_number(input[s - 1]) {
            return None;
        }
        if recognize(input, s).is_some_and(|end| end > i) {
            return Some(true);
        }
    }
    Some(false)
}

fn digit_run(input: &[u8], mut j: usize) -> usize {
    let start = j;
    while j < input.len() && input[j].is_ascii_digit() {
        j += 1;
    }
    j - start
}

/// Whether the byte before a sign makes it a dash inside something rather
/// than the quantity's own sign: a letter, a digit, an underscore, or a byte
/// of a non-ASCII character.
fn joins_a_sign(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// The end of the quantity opening at `i`, where the lexer met a digit or a
/// sign, or `None` where the bytes there are not one: a sign the byte before
/// joins, no digits, no symbol, or a symbol this placement does not read.
pub(crate) fn recognize(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    if let Some(len) = sign_len(input, j) {
        if j > 0 && joins_a_sign(input[j - 1]) {
            return None;
        }
        j += len;
    }
    let digits = digit_run(input, j);
    if digits == 0 {
        return None;
    }
    j += digits;
    if j + 1 < n && input[j] == b'.' && input[j + 1].is_ascii_digit() {
        j += 1;
        j += digit_run(input, j);
    }
    if let Some(len) = symbol_len(input, j, false) {
        return Some(j + len);
    }
    let sep = separator_len(input, j)?;
    let len = symbol_len(input, j + sep, true)?;
    Some(j + sep + len)
}

/// The written number and the written unit of a quantity's text: the sign
/// and digits, and the symbol after the separators, each as it stands.
#[must_use]
pub fn split(text: &str) -> Option<(&str, &str)> {
    let b = text.as_bytes();
    let mut j = 0;
    if let Some(len) = sign_len(b, 0) {
        j += len;
    }
    let digits = digit_run(b, j);
    if digits == 0 {
        return None;
    }
    j += digits;
    if j + 1 < b.len() && b[j] == b'.' && b[j + 1].is_ascii_digit() {
        j += 1;
        j += digit_run(b, j);
    }
    let value = &text[..j];
    let unit = text[j..].trim_start_matches([' ', '\u{a0}', '\u{202f}', '\u{2009}']);
    if unit.is_empty() {
        return None;
    }
    Some((value, unit))
}

/// The quantity `text` writes, in any unit of the table, attached or after
/// separators; a duration of several segments (`3h20m`) and a byte size in
/// any case (`10mb`) read through the readers `\R` and `\Z` use. `None`
/// where the text is not a number followed by a unit.
#[must_use]
pub fn parse(text: &str) -> Option<Quantity> {
    let text = text.trim();
    let (value, unit) = split(text)?;
    let written = Decimal::parse(&value.replace('\u{2212}', "-"))?;
    if let Some(def) = table().by_symbol.get(unit) {
        return Some(Quantity {
            family: def.family,
            base: unit_value(def, &written),
            value: written,
            unit: unit.to_string(),
        });
    }
    if let Some(ns) = crate::typed::parse_duration(text) {
        return Some(Quantity { family: Family::Time, base: ns, value: written, unit: unit.to_string() });
    }
    let bytes = crate::typed::parse_bytes(text)?;
    Some(Quantity { family: Family::Data, base: bytes, value: written, unit: unit.to_string() })
}

/// [`parse`] for the text of a token of `kind`: a duration token reads as a
/// duration, whose `m` is the minute, a byte size as bytes, a percentage as a
/// ratio; every other kind reads in the table.
#[must_use]
pub fn parse_as(kind: TokenKind, text: &str) -> Option<Quantity> {
    let text = text.trim();
    match kind {
        TokenKind::Duration => {
            let (value, unit) = split(text)?;
            let written = Decimal::parse(value)?;
            let base = crate::typed::parse_duration(text)?;
            Some(Quantity { family: Family::Time, base, value: written, unit: unit.to_string() })
        }
        TokenKind::ByteSize => {
            let (value, unit) = split(text)?;
            let written = Decimal::parse(value)?;
            let base = crate::typed::parse_bytes(text)?;
            Some(Quantity { family: Family::Data, base, value: written, unit: unit.to_string() })
        }
        TokenKind::Percent => {
            let (value, unit) = split(text)?;
            let written = Decimal::parse(value)?;
            let def = table().by_symbol.get("%")?;
            Some(Quantity {
                family: Family::Ratio,
                base: unit_value(def, &written),
                value: written,
                unit: unit.to_string(),
            })
        }
        _ => parse(text),
    }
}

/// Whether `text` is a number then one of `symbols`, attached or one
/// separator apart: the check a library kind applies to the span its pattern
/// matched, so a number and a symbol a line apart are not fused.
pub(crate) fn context_span(text: &[u8], symbols: &[&str]) -> bool {
    let mut j = digit_run(text, 0);
    if j == 0 {
        return false;
    }
    if j + 1 < text.len() && text[j] == b'.' && text[j + 1].is_ascii_digit() {
        j += 1;
        j += digit_run(text, j);
    }
    if let Some(len) = separator_len(text, j) {
        j += len;
    }
    symbols.iter().any(|s| &text[j..] == s.as_bytes())
}

/// The symbols of `family`, in table order.
#[must_use]
pub fn symbols(family: Family) -> Vec<&'static str> {
    UNITS.iter().filter(|u| u.family == family).map(|u| u.symbol).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_text(text: &str) -> String {
        parse(text).map_or_else(|| "none".to_string(), |q| format!("{} {}", q.base.to_text(), q.family.name()))
    }

    #[test]
    fn every_symbol_is_one_unit_and_every_family_has_a_base() {
        let t = table();
        assert_eq!(t.by_symbol.len(), UNITS.len());
        for f in Family::ALL {
            assert!(!f.base().is_empty());
            assert_eq!(Family::parse(f.name()), Some(f));
            assert!(!symbols(f).is_empty(), "{} has units", f.name());
        }
    }

    #[test]
    fn values_normalize_exactly() {
        assert_eq!(base_text("5kg"), "5000 mass");
        assert_eq!(base_text("0.1 kg"), "100 mass");
        assert_eq!(base_text("4.35 m"), "4.35 length");
        assert_eq!(base_text("435cm"), "4.35 length");
        assert_eq!(base_text("1 lb"), "453.59237 mass");
        assert_eq!(base_text("100\u{b0}C"), "212 temperature");
        assert_eq!(base_text("-40\u{b0}C"), "-40 temperature");
        assert_eq!(base_text("0K"), "-459.67 temperature");
        assert_eq!(base_text("273.15 K"), "32 temperature");
        assert_eq!(base_text("1 psi"), "444822161.52605 pressure");
        assert_eq!(base_text("1 bar"), "6451600000 pressure");
        assert_eq!(base_text("1 atm"), "6537083700 pressure");
        assert_eq!(base_text("60 rpm"), "60 frequency");
        assert_eq!(base_text("1 Hz"), "60 frequency");
        assert_eq!(base_text("1 m/s"), "3600 speed");
        assert_eq!(base_text("1 km/h"), "1000 speed");
        assert_eq!(base_text("1 mph"), "1609.344 speed");
        assert_eq!(base_text("1.5GiB"), "1610612736 data");
        assert_eq!(base_text("10mb"), "10000000 data");
        assert_eq!(base_text("3h20m"), "12000000000000 time");
        assert_eq!(base_text("5 min"), "300000000000 time");
        assert_eq!(base_text("40%"), "400000000 ratio");
        assert_eq!(base_text("40 %"), "400000000 ratio");
        assert_eq!(base_text("1 kWh"), "3600000 energy");
        assert_eq!(base_text("\u{2212}5 dB"), "-5 level");
        assert_eq!(base_text("5"), "none");
        assert_eq!(base_text("kg"), "none");
        assert_eq!(base_text("5 kgs"), "none");
    }

    #[test]
    fn a_token_reads_in_its_own_kind() {
        let m = parse_as(TokenKind::Duration, "5m").expect("a duration");
        assert_eq!((m.family, m.base.to_text().as_str()), (Family::Time, "300000000000"));
        let m = parse_as(TokenKind::Quantity, "5m").expect("a quantity");
        assert_eq!((m.family, m.base.to_text().as_str()), (Family::Length, "5"));
        let p = parse_as(TokenKind::Percent, "12.5%").expect("a percentage");
        assert_eq!((p.family, p.base.to_text().as_str()), (Family::Ratio, "125000000"));
        let z = parse_as(TokenKind::ByteSize, "2KiB").expect("a byte size");
        assert_eq!((z.family, z.base.to_text().as_str()), (Family::Data, "2048"));
    }

    fn end(text: &str) -> Option<usize> {
        recognize(text.as_bytes(), 0)
    }

    #[test]
    fn the_lexer_reads_attached_and_spaced_forms() {
        assert_eq!(end("5kg,"), Some(3));
        assert_eq!(end("5 kg,"), Some(4));
        assert_eq!(end("5\u{a0}kg"), Some(5));
        assert_eq!(end("3.2 GHz"), Some(7));
        assert_eq!(end("40 %"), Some(4));
        assert_eq!(end("20\u{b0}C"), Some(5));
        assert_eq!(end("45\u{b0}"), Some(4));
        assert_eq!(end("5 m/s"), Some(5));
        assert_eq!(end("5 m\u{b2}"), Some(5));
        assert_eq!(end("-40\u{b0}C"), Some(6));
        assert_eq!(end("\u{2212}5 dB"), Some(7));
        assert_eq!(end("5in"), Some(3));
        assert_eq!(end("5A"), Some(2));
        assert_eq!(end("5kWh"), Some(4));
        assert_eq!(end("5kgs"), None);
        assert_eq!(end("5 m"), None, "a single letter is not read after a space");
        assert_eq!(end("5 in"), None, "an English word is not read after a space");
        assert_eq!(end("5K"), None, "the kelvin is left to the context");
        assert_eq!(end("5 items"), None);
        assert_eq!(end("5  kg"), None, "one separator only");
        assert_eq!(end("5"), None);
        assert_eq!(recognize(b"10-20kg", 2), None, "a dash after a digit is a range dash");
        assert_eq!(recognize(b"x-5kg", 1), None);
        assert_eq!(recognize(b"(-5kg)", 1), Some(5));
    }

    #[test]
    fn the_context_check_holds_the_number_and_the_symbol_together() {
        assert!(context_span(b"5K", &["K"]));
        assert!(context_span(b"5 K", &["K"]));
        assert!(context_span("4.2\u{a0}K".as_bytes(), &["K"]));
        assert!(!context_span(b"5\nK", &["K"]));
        assert!(!context_span(b"5 Kelvin", &["K"]));
        assert!(!context_span(b"K", &["K"]));
        assert!(context_span(b"5 l", &["L", "l"]));
    }

    #[test]
    fn a_symbol_after_a_number_is_read_back_to_it() {
        // The position a byte route asks about is the symbol's own start.
        let joins = |text: &str, at: usize| joins_the_number_before(text.as_bytes(), at);
        assert_eq!(joins("2 kb", 2), Some(true));
        assert_eq!(joins("2.5 kb", 4), Some(true));
        assert_eq!(joins("10 GHz", 3), Some(true));
        assert_eq!(joins("5\u{a0}kg", 3), Some(true));
        assert_eq!(joins("2kb of 2 kb", 4), Some(false), "a word after a word joins nothing");
        assert_eq!(joins("5 items", 2), Some(false), "a word that is no unit symbol joins nothing");
        assert_eq!(joins("kb", 0), Some(false));
        assert_eq!(joins("5  kg", 3), Some(false), "one separator only");
        assert_eq!(joins("5 m", 2), Some(false), "a single letter is not read after a space");
        // A byte before the number can carry another token across its start,
        // and then only the lexer says whether a quantity is made here.
        assert_eq!(joins("1.2.3 kb", 6), None);
        assert_eq!(joins("x-5 kg", 4), None);
        // A run that is no unit symbol is settled without reading back, so a
        // number before one of those costs nothing and refuses nothing.
        assert_eq!(joins("alpha2 _2alpha", 7), Some(false));
        assert_eq!(joins("v1.2.3 v1.x", 7), Some(false));
        assert_eq!(joins("cond_22 cond_x", 8), Some(false));
    }

    #[test]
    fn the_written_parts_split_as_written() {
        assert_eq!(split("5.50 kg"), Some(("5.50", "kg")));
        assert_eq!(split("-40\u{b0}C"), Some(("-40", "\u{b0}C")));
        assert_eq!(split("5"), None);
        assert_eq!(split("kg"), None);
    }
}
