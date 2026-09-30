//! The shipped library: named token kinds with a shape and, where the
//! standard defines one, a checksum guard, and named sub-patterns over the
//! stream. A `\{name}` reaches every entry without a declaration, and a
//! declaration of the same name shadows it.
//!
//! A library kind is lexed only for a pattern that names it: a thirteen-digit
//! number is a number to every other pattern and an EAN-13 to the one that
//! asks. A library sub-pattern is inlined where it is named.
//!
//! The unit kinds read from the context - the kelvin's `K`, the inch's `in`,
//! `bar`, `cal`, `gal`, and the single-letter symbols after a space - are
//! kinds declared from a pattern: a number then the symbol, with a word of
//! the unit's family or another quantity of it within [`CONTEXT_WINDOW`]
//! significant tokens on either side. After the lex, the tokens such a match
//! covers fuse into one token of the kind, as a declared `kind` does, and
//! `\{qty}` holds every one of them beside the quantity kind itself. So
//! `cooled to 4.2K` reads a temperature and `a 5K run` a number and a word,
//! and a pattern that never names `\{qty}` or a unit kind never pays the
//! pass.

use std::sync::OnceLock;

use crate::ast::{AltMode, Atom, Pattern};
use crate::custom::{LIBRARY_ID_BASE, PatternKind, Precedence, ShapeSet, TokenShape, UnitGuard};
use crate::token::TokenKind;

/// The significant tokens on either side of a number and its symbol within
/// which a cue of the unit's family makes the two a quantity: the rolling
/// context window ([`crate::context::ContextConfig::token_window`]).
pub const CONTEXT_WINDOW: usize = 32;

/// What one entry is.
#[derive(Clone, Copy, Debug)]
pub enum Form {
    /// A bounded byte-pattern the lexer fuses into one token, with the check
    /// its standard defines.
    Shape { pat: &'static str, guard: Option<fn(&[u8]) -> bool> },
    /// A pattern over the stream, inlined where named.
    Let(&'static str),
    /// A vocabulary: one of these words, as written or in any case.
    Words { words: &'static [&'static str], any_case: bool },
    /// A unit kind read from the context: a number then one of `symbols`,
    /// attached or one separator apart, with the sub-pattern `cue` matching
    /// within [`CONTEXT_WINDOW`] tokens before the number or after the
    /// symbol; the tokens fuse into one token of the kind after the lex.
    Kind { symbols: &'static [&'static str], cue: &'static str },
}

/// One library entry.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The name `\{name}` uses.
    pub name: &'static str,
    /// What it matches, in one line.
    pub what: &'static str,
    pub form: Form,
}

impl Entry {
    /// Whether the entry is a kind a lex produces, as against a sub-pattern
    /// the parser inlines.
    #[must_use]
    pub fn is_kind(&self) -> bool {
        matches!(self.form, Form::Shape { .. } | Form::Kind { .. })
    }

    /// Whether the entry checks its bytes beyond their shape.
    #[must_use]
    pub fn is_guarded(&self) -> bool {
        matches!(self.form, Form::Shape { guard: Some(_), .. } | Form::Kind { .. })
    }

    /// Whether the entry is a unit kind read from the tokens around it.
    #[must_use]
    pub fn reads_context(&self) -> bool {
        matches!(self.form, Form::Kind { .. })
    }
}

fn iban_guard(text: &[u8]) -> bool {
    crate::checksum::iban(text)
}

fn github_token_guard(text: &[u8]) -> bool {
    crate::checksum::github_token(text)
}

fn isbn_guard(text: &[u8]) -> bool {
    crate::checksum::isbn(text)
}

fn vin_guard(text: &[u8]) -> bool {
    crate::checksum::vin(text)
}

fn isin_guard(text: &[u8]) -> bool {
    crate::checksum::isin(text)
}

fn gs1_guard(text: &[u8]) -> bool {
    crate::checksum::gs1(text)
}

fn imei_guard(text: &[u8]) -> bool {
    let digits: Vec<u8> = text.iter().copied().filter(|b| *b != b'-').collect();
    digits.len() == 15 && crate::checksum::luhn(&digits)
}

fn ethereum_guard(text: &[u8]) -> bool {
    text.len() == 42 && crate::checksum::ethereum_address(&text[2..])
}

fn bitcoin_guard(text: &[u8]) -> bool {
    if text.len() >= 3 && text[..3].eq_ignore_ascii_case(b"bc1") {
        crate::checksum::bech32(text)
    } else {
        crate::checksum::base58check(text)
    }
}

fn dns_label_guard(text: &[u8]) -> bool {
    text.len() <= 63
}

const LOG_LEVELS: &[&str] = &[
    "TRACE", "DEBUG", "INFO", "NOTICE", "WARN", "WARNING", "ERROR", "ERR", "CRIT", "CRITICAL",
    "FATAL", "ALERT", "EMERG", "EMERGENCY", "PANIC",
];

/// Words that place a number and a bare symbol in a family: each is a cue
/// for the unit kinds the library reads from the context.
const TEMPERATURE_WORDS: &[&str] = &[
    "temperature", "temperatures", "temp", "temps", "kelvin", "kelvins", "thermal", "thermally",
    "cryogenic", "cryogenics", "cryostat", "cooled", "cooling", "cools", "cool", "cold", "colder",
    "coldest", "heated", "heating", "heats", "heat", "hot", "hotter", "warm", "warmed", "warming",
    "boiling", "boils", "boil", "melting", "melts", "melt", "freezing", "freezes", "freeze",
    "frozen", "superconducting", "superconductor", "superconductivity", "celsius", "centigrade",
    "fahrenheit", "degrees", "degree", "thermometer", "thermostat", "ambient", "helium", "nitrogen",
    "blackbody", "isothermal", "annealed", "annealing", "chilled", "chiller", "refrigerated",
    "refrigerator", "oven", "furnace", "dewar", "plasma",
];

const LENGTH_WORDS: &[&str] = &[
    "length", "lengths", "long", "longer", "longest", "wide", "wider", "width", "widths", "tall",
    "taller", "height", "heights", "high", "higher", "deep", "deeper", "depth", "depths",
    "distance", "distances", "diameter", "diameters", "radius", "radii", "thick", "thicker",
    "thickness", "span", "spans", "far", "farther", "further", "away", "meter", "meters", "metre",
    "metres", "kilometer", "kilometers", "kilometre", "kilometres", "centimeter", "centimeters",
    "centimetre", "centimetres", "millimeter", "millimeters", "millimetre", "millimetres", "inch",
    "inches", "foot", "feet", "yard", "yards", "mile", "miles", "altitude", "elevation",
    "wavelength", "wavelengths", "gap", "clearance", "spacing", "pitch", "circumference",
    "perimeter", "stroke", "bore",
];

const TIME_WORDS: &[&str] = &[
    "time", "times", "timing", "second", "seconds", "minute", "minutes", "hour", "hours", "hourly",
    "day", "days", "duration", "durations", "elapsed", "latency", "latencies", "delay", "delays",
    "delayed", "timeout", "timeouts", "interval", "intervals", "period", "periods", "wait", "waits",
    "waited", "waiting", "took", "takes", "lasted", "lasts", "runtime", "uptime", "downtime",
    "sleep", "sleeps", "slept", "pause", "paused", "timer", "timers", "deadline", "expires",
    "expiry", "ttl", "rtt", "every", "within", "after", "before",
];

const MASS_WORDS: &[&str] = &[
    "mass", "masses", "weight", "weights", "weigh", "weighs", "weighed", "weighing", "heavy",
    "heavier", "heaviest", "gram", "grams", "kilogram", "kilograms", "kilo", "kilos", "tonne",
    "tonnes", "ton", "tons", "pound", "pounds", "ounce", "ounces", "load", "loads", "loaded",
    "payload", "payloads", "cargo", "freight", "dose", "dosage", "doses", "tare",
];

const CURRENT_WORDS: &[&str] = &[
    "current", "currents", "amp", "amps", "ampere", "amperes", "amperage", "draw", "draws",
    "drawing", "drew", "fuse", "fuses", "breaker", "breakers", "circuit", "circuits", "rated",
    "rating", "ratings", "charging", "discharge", "discharging", "motor", "motors", "relay",
    "relays", "coil", "coils", "winding", "windings", "supply", "load",
];

const VOLTAGE_WORDS: &[&str] = &[
    "voltage", "voltages", "volt", "volts", "potential", "potentials", "supply", "supplies", "rail",
    "rails", "battery", "batteries", "charger", "chargers", "bias", "biased", "biasing",
    "regulator", "regulators", "transformer", "transformers", "mains", "output", "input", "pin",
    "pins", "gate", "drain", "source", "threshold", "breakdown", "ripple", "dropout", "vcc", "vdd",
    "vref", "vin", "vout", "emf", "dc", "ac",
];

const POWER_WORDS: &[&str] = &[
    "power", "powers", "watt", "watts", "wattage", "consumption", "consumes", "consuming",
    "consumed", "output", "outputs", "dissipation", "dissipates", "dissipated", "heater", "heaters",
    "bulb", "bulbs", "lamp", "lamps", "psu", "load", "loads", "rated", "rating", "generator",
    "generators", "turbine", "turbines", "solar", "panel", "panels", "inverter", "inverters",
    "amplifier", "amplifiers", "transmitter", "transmitters", "tdp", "draw", "draws", "charger",
    "chargers", "motor", "motors", "engine", "engines", "horsepower", "kilowatt", "kilowatts",
    "megawatt", "megawatts", "gigawatt", "gigawatts",
];

const FORCE_WORDS: &[&str] = &[
    "force", "forces", "newton", "newtons", "thrust", "tension", "tensions", "compression",
    "compressive", "tensile", "load", "loads", "loading", "pull", "pulls", "push", "pushes",
    "weight", "drag", "lift", "friction", "clamp", "clamping", "preload", "spring", "springs",
    "impact", "shear", "yield", "breaking", "strength",
];

const ENERGY_WORDS: &[&str] = &[
    "energy", "energies", "joule", "joules", "calorie", "calories", "kilocalorie", "kilocalories",
    "heat", "work", "kinetic", "potential", "enthalpy", "dissipated", "consumed", "consumption",
    "burn", "burns", "burned", "burnt", "stored", "storage", "capacity", "battery", "batteries",
    "fuel", "diet", "dietary", "nutrition", "nutritional", "meal", "meals", "snack", "serving",
    "servings", "intake", "metabolism", "workout", "exercise", "photon", "photons", "bond",
    "ionization", "binding", "released", "absorbed", "absorption", "emission", "yield",
];

const VOLUME_WORDS: &[&str] = &[
    "volume", "volumes", "liter", "liters", "litre", "litres", "milliliter", "milliliters",
    "millilitre", "millilitres", "gallon", "gallons", "quart", "quarts", "pint", "pints", "capacity",
    "capacities", "tank", "tanks", "fuel", "water", "milk", "oil", "juice", "beer", "wine",
    "liquid", "liquids", "fluid", "fluids", "container", "containers", "bottle", "bottles",
    "bucket", "buckets", "barrel", "barrels", "reservoir", "displacement", "displaces", "pour",
    "pours", "poured", "fill", "fills", "filled", "drink", "drinks", "drank",
];

const PRESSURE_WORDS: &[&str] = &[
    "pressure", "pressures", "pressurized", "pressurised", "pascal", "pascals", "psi", "tyre",
    "tyres", "tire", "tires", "boost", "hydraulic", "hydraulics", "pneumatic", "pneumatics",
    "atmosphere", "atmospheres", "atmospheric", "compressor", "compressors", "compressed", "pump",
    "pumps", "pumped", "vacuum", "manifold", "gauge", "gauges", "inflate", "inflated", "inflation",
    "regulator", "valve", "valves", "cylinder", "cylinders", "brake", "brakes", "espresso", "steam",
    "boiler", "boilers", "diving", "dive", "dives", "diver",
];

const HTTP_METHODS: &[&str] =
    &["GET", "HEAD", "POST", "PUT", "DELETE", "CONNECT", "OPTIONS", "TRACE", "PATCH"];

const WEEKDAYS: &[&str] = &[
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "mon", "tue",
    "wed", "thu", "fri", "sat", "sun",
];

const MONTHS: &[&str] = &[
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep",
    "sept", "oct", "nov", "dec",
];

/// ISO 4217 currency codes in circulation, and the fund and metal codes.
const CURRENCIES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BOV", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD",
    "CAD", "CDF", "CHE", "CHF", "CHW", "CLF", "CLP", "CNY", "COP", "COU", "CRC", "CUC", "CUP",
    "CVE", "CZK", "DJF", "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP",
    "GEL", "GHS", "GIP", "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS",
    "INR", "IQD", "IRR", "ISK", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW",
    "KWD", "KYD", "KZT", "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD",
    "MMK", "MNT", "MOP", "MRU", "MUR", "MVR", "MWK", "MXN", "MXV", "MYR", "MZN", "NAD", "NGN",
    "NIO", "NOK", "NPR", "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR",
    "RON", "RSD", "RUB", "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SLL",
    "SOS", "SRD", "SSP", "STN", "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY",
    "TTD", "TWD", "TZS", "UAH", "UGX", "USD", "USN", "UYI", "UYU", "UYW", "UZS", "VED", "VES",
    "VND", "VUV", "WST", "XAF", "XAG", "XAU", "XBA", "XBB", "XBC", "XBD", "XCD", "XDR", "XOF",
    "XPD", "XPF", "XPT", "XSU", "XTS", "XUA", "XXX", "YER", "ZAR", "ZMW", "ZWG", "ZWL",
];

/// ISO 3166-1 alpha-2 country codes.
const COUNTRIES: &[&str] = &[
    "AD", "AE", "AF", "AG", "AI", "AL", "AM", "AO", "AQ", "AR", "AS", "AT", "AU", "AW", "AX",
    "AZ", "BA", "BB", "BD", "BE", "BF", "BG", "BH", "BI", "BJ", "BL", "BM", "BN", "BO", "BQ",
    "BR", "BS", "BT", "BV", "BW", "BY", "BZ", "CA", "CC", "CD", "CF", "CG", "CH", "CI", "CK",
    "CL", "CM", "CN", "CO", "CR", "CU", "CV", "CW", "CX", "CY", "CZ", "DE", "DJ", "DK", "DM",
    "DO", "DZ", "EC", "EE", "EG", "EH", "ER", "ES", "ET", "FI", "FJ", "FK", "FM", "FO", "FR",
    "GA", "GB", "GD", "GE", "GF", "GG", "GH", "GI", "GL", "GM", "GN", "GP", "GQ", "GR", "GS",
    "GT", "GU", "GW", "GY", "HK", "HM", "HN", "HR", "HT", "HU", "ID", "IE", "IL", "IM", "IN",
    "IO", "IQ", "IR", "IS", "IT", "JE", "JM", "JO", "JP", "KE", "KG", "KH", "KI", "KM", "KN",
    "KP", "KR", "KW", "KY", "KZ", "LA", "LB", "LC", "LI", "LK", "LR", "LS", "LT", "LU", "LV",
    "LY", "MA", "MC", "MD", "ME", "MF", "MG", "MH", "MK", "ML", "MM", "MN", "MO", "MP", "MQ",
    "MR", "MS", "MT", "MU", "MV", "MW", "MX", "MY", "MZ", "NA", "NC", "NE", "NF", "NG", "NI",
    "NL", "NO", "NP", "NR", "NU", "NZ", "OM", "PA", "PE", "PF", "PG", "PH", "PK", "PL", "PM",
    "PN", "PR", "PS", "PT", "PW", "PY", "QA", "RE", "RO", "RS", "RU", "RW", "SA", "SB", "SC",
    "SD", "SE", "SG", "SH", "SI", "SJ", "SK", "SL", "SM", "SN", "SO", "SR", "SS", "ST", "SV",
    "SX", "SY", "SZ", "TC", "TD", "TF", "TG", "TH", "TJ", "TK", "TL", "TM", "TN", "TO", "TR",
    "TT", "TV", "TW", "TZ", "UA", "UG", "UM", "US", "UY", "UZ", "VA", "VC", "VE", "VG", "VI",
    "VN", "VU", "WF", "WS", "YE", "YT", "ZA", "ZM", "ZW",
];

/// Every entry, in the order `trex lib` lists them. A shape's id is
/// [`LIBRARY_ID_BASE`] plus its place among the shapes.
pub const ENTRIES: &[Entry] = &[
    Entry {
        name: "iban",
        what: "an IBAN, compact or in groups of four, with its country's length and the mod 97-10 check",
        form: Form::Shape {
            pat: "[A-Z]{2}[0-9]{2}( ?[A-Z0-9]{4}){2,7}( ?[A-Z0-9]{1,4})?",
            guard: Some(iban_guard),
        },
    },
    Entry {
        name: "isbn",
        what: "an ISBN-10 (mod 11, X as ten) or ISBN-13 (978 or 979, EAN check), hyphens or spaces allowed",
        form: Form::Shape {
            pat: "(97[89][- ]?)?[0-9]{1,5}[- ]?[0-9]{1,7}[- ]?[0-9]{1,7}[- ]?[0-9X]",
            guard: Some(isbn_guard),
        },
    },
    Entry {
        name: "vin",
        what: "a vehicle identification number: seventeen characters without I, O or Q and the check digit ninth",
        form: Form::Shape { pat: "[A-HJ-NPR-Z0-9]{17}", guard: Some(vin_guard) },
    },
    Entry {
        name: "isin",
        what: "an ISIN: a country code, nine alphanumerics and the Luhn check over the expanded letters",
        form: Form::Shape { pat: "[A-Z]{2}[A-Z0-9]{9}[0-9]", guard: Some(isin_guard) },
    },
    Entry {
        name: "ean13",
        what: "an EAN-13 with its GS1 check digit",
        form: Form::Shape { pat: "[0-9]{13}", guard: Some(gs1_guard) },
    },
    Entry {
        name: "upca",
        what: "a UPC-A with its GS1 check digit",
        form: Form::Shape { pat: "[0-9]{12}", guard: Some(gs1_guard) },
    },
    Entry {
        name: "ean8",
        what: "an EAN-8 with its GS1 check digit",
        form: Form::Shape { pat: "[0-9]{8}", guard: Some(gs1_guard) },
    },
    Entry {
        name: "imei",
        what: "an IMEI: fifteen digits, hyphenated or not, with the Luhn check",
        form: Form::Shape { pat: "[0-9]{2}-?[0-9]{6}-?[0-9]{6}-?[0-9]", guard: Some(imei_guard) },
    },
    Entry {
        name: "ethaddr",
        what: "an Ethereum address: 0x and forty hex digits, EIP-55 case where the case is mixed",
        form: Form::Shape { pat: "0x[0-9a-fA-F]{40}", guard: Some(ethereum_guard) },
    },
    Entry {
        name: "btcaddr",
        what: "a Bitcoin address: base58check for 1 and 3, bech32 or bech32m for bc1",
        form: Form::Shape {
            pat: "[13][a-km-zA-HJ-NP-Z1-9]{25,34}|(bc1|BC1)[a-zA-Z0-9]{11,87}",
            guard: Some(bitcoin_guard),
        },
    },
    Entry {
        name: "awskey",
        what: "an AWS access key id: AKIA or ASIA and sixteen uppercase alphanumerics",
        form: Form::Shape { pat: "(AKIA|ASIA)[A-Z0-9]{16}", guard: None },
    },
    Entry {
        name: "github_token",
        what: "a GitHub token: ghp_, gho_, ghu_, ghs_ or ghr_ and thirty-six alphanumerics whose last six carry the CRC-32 of the thirty before them, or a github_pat_ fine-grained token",
        form: Form::Shape {
            pat: "gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9]{22}_[A-Za-z0-9]{59}",
            guard: Some(github_token_guard),
        },
    },
    Entry {
        name: "slack_token",
        what: "a Slack token: xoxb-, xoxa-, xoxp-, xoxr- or xoxs- and its body",
        form: Form::Shape { pat: "xox[abprs]-[0-9A-Za-z-]{10,72}", guard: None },
    },
    Entry {
        name: "google_key",
        what: "a Google API key: AIza and thirty-five key characters",
        form: Form::Shape { pat: "AIza[0-9A-Za-z_-]{35}", guard: None },
    },
    Entry {
        name: "stripe_key",
        what: "a Stripe key: sk_, pk_ or rk_ then live_ or test_ and its body",
        form: Form::Shape { pat: "[prs]k_(live|test)_[0-9A-Za-z]{24,99}", guard: None },
    },
    Entry {
        name: "twilio_key",
        what: "a Twilio account or key sid: AC or SK and thirty-two hex digits",
        form: Form::Shape { pat: "(SK|AC)[0-9a-f]{32}", guard: None },
    },
    Entry {
        name: "cve",
        what: "a CVE id: CVE-, the year, and four or more digits",
        form: Form::Shape { pat: "CVE-[0-9]{4}-[0-9]{4,7}", guard: None },
    },
    Entry {
        name: "mime",
        what: "a media type: one of the registered top-level types, a slash and a subtype",
        form: Form::Shape {
            pat: "(application|audio|font|image|message|model|multipart|text|video)/[a-zA-Z0-9][a-zA-Z0-9!#$&^_.+-]{0,126}",
            guard: None,
        },
    },
    Entry {
        name: "docker_image",
        what: "a container image reference with at least one path component, an optional tag and an optional sha256 digest",
        form: Form::Shape {
            pat: "([a-z0-9][a-z0-9._-]{0,62}/){1,3}[a-z0-9][a-z0-9._-]{0,127}(:[A-Za-z0-9_][A-Za-z0-9_.-]{0,127})?(@sha256:[0-9a-f]{64})?",
            guard: None,
        },
    },
    Entry {
        name: "k8s_name",
        what: "a Kubernetes resource name: lowercase DNS label segments joined by dashes, at most sixty-three characters",
        form: Form::Shape { pat: "[a-z0-9]{1,63}(-[a-z0-9]{1,63}){1,9}", guard: Some(dns_label_guard) },
    },
    Entry {
        name: "private_key",
        what: "a PEM private key block, from its BEGIN line to its END line",
        form: Form::Let(
            "\\P+ \"BEGIN\" \\W{0,3} \"PRIVATE\" \"KEY\" \\P+ .*? \\P+ \"END\" \\W{0,3} \"PRIVATE\" \"KEY\" \\P+",
        ),
    },
    Entry {
        name: "git_sha",
        what: "a full git object id: forty or sixty-four lowercase hex digits",
        form: Form::Shape { pat: "[0-9a-f]{40}|[0-9a-f]{64}", guard: None },
    },
    Entry {
        name: "ip_private",
        what: "an address in a private range: 10/8, 172.16/12, 192.168/16 or fc00::/7",
        form: Form::Let(
            "\\I{in:10.0.0.0/8} | \\I{in:172.16.0.0/12} | \\I{in:192.168.0.0/16} | \\I{in:fc00::/7}",
        ),
    },
    Entry {
        name: "ip_loopback",
        what: "a loopback address: 127/8 or ::1",
        form: Form::Let("\\I{in:127.0.0.0/8} | \\I{in:0::1/128}"),
    },
    Entry {
        name: "ip_linklocal",
        what: "a link-local address: 169.254/16 or fe80::/10",
        form: Form::Let("\\I{in:169.254.0.0/16} | \\I{in:fe80::/10}"),
    },
    Entry {
        name: "log_level",
        what: "a log level word in any case: TRACE, DEBUG, INFO, NOTICE, WARN, WARNING, ERROR, ERR, CRIT, CRITICAL, FATAL, ALERT, EMERG, EMERGENCY, PANIC",
        form: Form::Words { words: LOG_LEVELS, any_case: true },
    },
    Entry {
        name: "http_method",
        what: "an HTTP method as the standard spells it: GET, HEAD, POST, PUT, DELETE, CONNECT, OPTIONS, TRACE, PATCH",
        form: Form::Words { words: HTTP_METHODS, any_case: false },
    },
    Entry {
        name: "http_status",
        what: "an HTTP status code, 100 to 599",
        form: Form::Let("\\N{100..599}"),
    },
    Entry { name: "http_1xx", what: "an informational status, 100 to 199", form: Form::Let("\\N{100..199}") },
    Entry { name: "http_2xx", what: "a success status, 200 to 299", form: Form::Let("\\N{200..299}") },
    Entry { name: "http_3xx", what: "a redirection status, 300 to 399", form: Form::Let("\\N{300..399}") },
    Entry { name: "http_4xx", what: "a client error status, 400 to 499", form: Form::Let("\\N{400..499}") },
    Entry { name: "http_5xx", what: "a server error status, 500 to 599", form: Form::Let("\\N{500..599}") },
    Entry {
        name: "currency",
        what: "an ISO 4217 currency code as written, USD or EUR",
        form: Form::Words { words: CURRENCIES, any_case: false },
    },
    Entry {
        name: "country",
        what: "an ISO 3166-1 alpha-2 country code as written, DE or US",
        form: Form::Words { words: COUNTRIES, any_case: false },
    },
    Entry {
        name: "weekday",
        what: "a weekday name or its three-letter form, in any case",
        form: Form::Words { words: WEEKDAYS, any_case: true },
    },
    Entry {
        name: "month",
        what: "a month name or its short form, in any case",
        form: Form::Words { words: MONTHS, any_case: true },
    },
    Entry {
        name: "temperature_word",
        what: "a word of temperature in any case: temperature, temp, kelvin, thermal, cryogenic, cooled, heated, boiling, freezing, celsius, fahrenheit, degrees and their kin",
        form: Form::Words { words: TEMPERATURE_WORDS, any_case: true },
    },
    Entry {
        name: "temperature_cue",
        what: "a temperature word, the variable T, or a temperature quantity: what makes a bare K a kelvin",
        form: Form::Let("\\{temperature_word} | \"T\" | \\{quantity}{family:temperature}"),
    },
    Entry {
        name: "kelvin",
        what: "a temperature in kelvin, a number then K, read where a temperature cue lies within the context window",
        form: Form::Kind { symbols: &["K"], cue: "temperature_cue" },
    },
    Entry {
        name: "length_word",
        what: "a word of length in any case: length, wide, height, depth, distance, diameter, meter, inch, foot, mile and their kin",
        form: Form::Words { words: LENGTH_WORDS, any_case: true },
    },
    Entry {
        name: "length_cue",
        what: "a length word or a length quantity",
        form: Form::Let("\\{length_word} | \\{quantity}{family:length}"),
    },
    Entry {
        name: "inch",
        what: "a length in inches, a number then in, read where a length cue lies within the context window",
        form: Form::Kind { symbols: &["in"], cue: "length_cue" },
    },
    Entry {
        name: "meter",
        what: "a length in meters, a number then m a space apart, read where a length cue lies within the context window",
        form: Form::Kind { symbols: &["m"], cue: "length_cue" },
    },
    Entry {
        name: "time_word",
        what: "a word of time in any case: time, seconds, minutes, hours, duration, elapsed, latency, delay, timeout, interval and their kin",
        form: Form::Words { words: TIME_WORDS, any_case: true },
    },
    Entry {
        name: "time_cue",
        what: "a time word, a duration, or a time quantity",
        form: Form::Let("\\{time_word} | \\R | \\{quantity}{family:time}"),
    },
    Entry {
        name: "second",
        what: "a time in seconds, a number then s a space apart, read where a time cue lies within the context window",
        form: Form::Kind { symbols: &["s"], cue: "time_cue" },
    },
    Entry {
        name: "hour",
        what: "a time in hours, a number then h a space apart, read where a time cue lies within the context window",
        form: Form::Kind { symbols: &["h"], cue: "time_cue" },
    },
    Entry {
        name: "mass_word",
        what: "a word of mass in any case: mass, weight, weighs, heavy, gram, kilogram, tonne, pound, ounce, load, dose and their kin",
        form: Form::Words { words: MASS_WORDS, any_case: true },
    },
    Entry {
        name: "mass_cue",
        what: "a mass word or a mass quantity",
        form: Form::Let("\\{mass_word} | \\{quantity}{family:mass}"),
    },
    Entry {
        name: "gram",
        what: "a mass in grams, a number then g a space apart, read where a mass cue lies within the context window",
        form: Form::Kind { symbols: &["g"], cue: "mass_cue" },
    },
    Entry {
        name: "tonne",
        what: "a mass in tonnes, a number then t a space apart, read where a mass cue lies within the context window",
        form: Form::Kind { symbols: &["t"], cue: "mass_cue" },
    },
    Entry {
        name: "current_word",
        what: "a word of electric current in any case: current, amps, ampere, draw, fuse, breaker, circuit, rated and their kin",
        form: Form::Words { words: CURRENT_WORDS, any_case: true },
    },
    Entry {
        name: "current_cue",
        what: "a current word or a current quantity",
        form: Form::Let("\\{current_word} | \\{quantity}{family:current}"),
    },
    Entry {
        name: "ampere",
        what: "a current in amperes, a number then A a space apart, read where a current cue lies within the context window",
        form: Form::Kind { symbols: &["A"], cue: "current_cue" },
    },
    Entry {
        name: "voltage_word",
        what: "a word of voltage in any case: voltage, volts, potential, supply, rail, battery, charger, bias, regulator and their kin",
        form: Form::Words { words: VOLTAGE_WORDS, any_case: true },
    },
    Entry {
        name: "voltage_cue",
        what: "a voltage word or a voltage quantity",
        form: Form::Let("\\{voltage_word} | \\{quantity}{family:voltage}"),
    },
    Entry {
        name: "volt",
        what: "a voltage in volts, a number then V a space apart, read where a voltage cue lies within the context window",
        form: Form::Kind { symbols: &["V"], cue: "voltage_cue" },
    },
    Entry {
        name: "power_word",
        what: "a word of power in any case: power, watts, consumption, output, dissipation, heater, generator, motor, horsepower and their kin",
        form: Form::Words { words: POWER_WORDS, any_case: true },
    },
    Entry {
        name: "power_cue",
        what: "a power word or a power quantity",
        form: Form::Let("\\{power_word} | \\{quantity}{family:power}"),
    },
    Entry {
        name: "watt",
        what: "a power in watts, a number then W a space apart, read where a power cue lies within the context window",
        form: Form::Kind { symbols: &["W"], cue: "power_cue" },
    },
    Entry {
        name: "force_word",
        what: "a word of force in any case: force, newtons, thrust, tension, compression, load, pull, drag, friction and their kin",
        form: Form::Words { words: FORCE_WORDS, any_case: true },
    },
    Entry {
        name: "force_cue",
        what: "a force word or a force quantity",
        form: Form::Let("\\{force_word} | \\{quantity}{family:force}"),
    },
    Entry {
        name: "newton",
        what: "a force in newtons, a number then N a space apart, read where a force cue lies within the context window",
        form: Form::Kind { symbols: &["N"], cue: "force_cue" },
    },
    Entry {
        name: "energy_word",
        what: "a word of energy in any case: energy, joules, calories, heat, work, kinetic, battery, fuel, diet, meal, photon and their kin",
        form: Form::Words { words: ENERGY_WORDS, any_case: true },
    },
    Entry {
        name: "energy_cue",
        what: "an energy word or an energy quantity",
        form: Form::Let("\\{energy_word} | \\{quantity}{family:energy}"),
    },
    Entry {
        name: "joule",
        what: "an energy in joules, a number then J a space apart, read where an energy cue lies within the context window",
        form: Form::Kind { symbols: &["J"], cue: "energy_cue" },
    },
    Entry {
        name: "calorie",
        what: "an energy in calories, a number then cal, read where an energy cue lies within the context window",
        form: Form::Kind { symbols: &["cal"], cue: "energy_cue" },
    },
    Entry {
        name: "volume_word",
        what: "a word of volume in any case: volume, liters, gallons, capacity, tank, fuel, water, liquid, container, bottle and their kin",
        form: Form::Words { words: VOLUME_WORDS, any_case: true },
    },
    Entry {
        name: "volume_cue",
        what: "a volume word or a volume quantity",
        form: Form::Let("\\{volume_word} | \\{quantity}{family:volume}"),
    },
    Entry {
        name: "liter",
        what: "a volume in liters, a number then L or l a space apart, read where a volume cue lies within the context window",
        form: Form::Kind { symbols: &["L", "l"], cue: "volume_cue" },
    },
    Entry {
        name: "gallon",
        what: "a volume in US gallons, a number then gal, read where a volume cue lies within the context window",
        form: Form::Kind { symbols: &["gal"], cue: "volume_cue" },
    },
    Entry {
        name: "pressure_word",
        what: "a word of pressure in any case: pressure, pascal, psi, tire, boost, hydraulic, pneumatic, compressor, pump, vacuum, valve and their kin",
        form: Form::Words { words: PRESSURE_WORDS, any_case: true },
    },
    Entry {
        name: "pressure_cue",
        what: "a pressure word or a pressure quantity",
        form: Form::Let("\\{pressure_word} | \\{quantity}{family:pressure}"),
    },
    Entry {
        name: "bar",
        what: "a pressure in bar, a number then bar, read where a pressure cue lies within the context window",
        form: Form::Kind { symbols: &["bar"], cue: "pressure_cue" },
    },
];

/// The pattern text a `Words` entry inlines.
fn words_source(words: &[&str], any_case: bool) -> String {
    let alternation = words.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(" | ");
    if any_case { format!("(?orbit:case {alternation})") } else { alternation }
}

/// The pattern text of a unit kind read from the context: a number, one of
/// the symbols, and the cue within [`CONTEXT_WINDOW`] tokens before the
/// number (a lookbehind ending after the symbol spans the cue, the tokens
/// between, the number and the symbol) or after the symbol.
fn context_kind_source(symbols: &[&str], cue: &str) -> String {
    let symbol = symbols.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(" | ");
    format!(
        "\\N ({symbol}) (~<(\\{{{cue}}} .{{0,{}}}) | ~>{CONTEXT_WINDOW}(\\{{{cue}}}))",
        CONTEXT_WINDOW + 1
    )
}

/// The library as one declaration set: every shape and kind under its fixed
/// id, every sub-pattern parsed against the entries before it.
pub fn standard() -> &'static ShapeSet {
    static STANDARD: OnceLock<ShapeSet> = OnceLock::new();
    STANDARD.get_or_init(|| {
        let mut set = ShapeSet::without_library();
        let mut kinds = 0u8;
        for e in ENTRIES {
            match e.form {
                Form::Shape { pat, guard } => {
                    let parsed = crate::bytepat::parse(pat.as_bytes())
                        .unwrap_or_else(|m| panic!("library shape {} parses: {m}", e.name));
                    let window = parsed
                        .max_len()
                        .unwrap_or_else(|| panic!("library shape {} is bounded", e.name));
                    set.push_library_shape(TokenShape {
                        name: e.name.to_string(),
                        pat: parsed,
                        window,
                        precedence: Precedence::Before,
                        id: LIBRARY_ID_BASE + kinds,
                        guard,
                    });
                    kinds += 1;
                }
                Form::Let(src) => {
                    set.declare_let(&format!("{} = {src}", e.name))
                        .unwrap_or_else(|err| panic!("library entry {} declares: {err}", e.name));
                }
                Form::Words { words, any_case } => {
                    set.declare_let(&format!("{} = {}", e.name, words_source(words, any_case)))
                        .unwrap_or_else(|err| panic!("library entry {} declares: {err}", e.name));
                }
                Form::Kind { symbols, cue } => {
                    let src = context_kind_source(symbols, cue);
                    let pattern = crate::parser::parse_with_shapes(&src, &set).unwrap_or_else(|err| {
                        panic!("library kind {} parses: {} at byte {}", e.name, err.msg, err.pos)
                    });
                    set.push_library_kind(PatternKind {
                        name: e.name.to_string(),
                        id: LIBRARY_ID_BASE + kinds,
                        pattern,
                        guard: Some(UnitGuard { symbols }),
                    });
                    kinds += 1;
                }
            }
        }
        set
    })
}

/// What every shipped entry must take, read and refuse, written as the `test`
/// lines a user's own pattern file writes.
///
/// One source. These are the library's own tests and the answer `trex lib
/// --test` gives with no file, so a reader checking a build they did not make
/// runs the same lines the crate's suite does, through the same parser and
/// the same checker.
///
/// As lines rather than as fields on [`Entry`] because that is what they are:
/// the grammar is the one a user writes, so an example here and an example in
/// someone's file are read by one piece of code and cannot drift into two
/// dialects. `accepts` is the whole significant extent, `reads "span" in
/// "text"` is that span out of that text - which is the expectation a kind
/// read from the tokens around it needs, since it takes part of a line and
/// leaves the rest - and `rejects` is nothing anywhere.
pub const TESTS: &[&str] = &[
    r#"iban accepts "GB82WEST12345698765432" "GB82 WEST 1234 5698 7654 32" "DE89370400440532013000" rejects "GB82WEST12345698765433" "ZZ82WEST12345698765432" "DE8937040044053201300""#,
    r#"isbn accepts "978-0-306-40615-7" "0306406152" "080442957X" "9780306406157" rejects "978-0-306-40615-8" "0306406153" "1234""#,
    r#"vin accepts "1HGCM82633A004352" rejects "1HGCM82634A004352" "1HGCM82633I004352""#,
    r#"isin accepts "US0378331005" "GB0002634946" rejects "US0378331006" "US037833100""#,
    r#"ean13 accepts "4006381333931" rejects "4006381333932" "400638133393""#,
    r#"upca accepts "036000291452" rejects "036000291453""#,
    r#"ean8 accepts "96385074" rejects "96385075""#,
    r#"imei accepts "490154203237518" "49-015420-323751-8" rejects "490154203237519""#,
    r#"ethaddr accepts "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed" "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed" rejects "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAeD" "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAe""#,
    r#"btcaddr accepts "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2" "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy" "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq" rejects "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3" "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdx""#,
    r#"awskey accepts "AKIAIOSFODNN7EXAMPLE" "ASIAIOSFODNN7EXAMPLE" rejects "AKIAIOSFODNN7EXAMPL" "AKIAiosfodnn7example""#,
    // The last six characters carry the CRC-32 of the thirty before them, so
    // a token differing in one of them is not one.
    r#"github_token accepts "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" rejects "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd34KlM7" "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij" "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi" "ghx_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij""#,
    r#"slack_token accepts "xoxb-123456789012-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx" rejects "xoxz-123456789012-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx""#,
    r#"google_key accepts "AIzaSyA1234567890abcdefghijklmnopqrstuv" rejects "AIzaSyA1234567890abcdefghijklmnopqrstu""#,
    r#"stripe_key accepts "sk_live_abcdefghijklmnopqrstuvwx" "pk_test_abcdefghijklmnopqrstuvwx" rejects "sk_prod_abcdefghijklmnopqrstuvwx""#,
    r#"twilio_key accepts "SK0123456789abcdef0123456789abcdef" "AC0123456789abcdef0123456789abcdef" rejects "SK0123456789abcdef0123456789abcde""#,
    r#"cve accepts "CVE-2021-44228" "CVE-2014-0160" rejects "CVE-21-44228" "CVE-2021-441""#,
    r#"mime accepts "text/html" "application/vnd.api+json" "image/svg+xml" rejects "text/" "html""#,
    r#"docker_image accepts "library/nginx" "ghcr.io/org/app:1.2.3" "docker.io/library/redis:7" rejects "nginx" "Nginx/app""#,
    r#"k8s_name accepts "my-app-7d9f" "web-0" rejects "my_app" "MyApp" "app""#,
    "private_key accepts \"-----BEGIN RSA PRIVATE KEY-----\\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\\n-----END RSA PRIVATE KEY-----\" rejects \"-----BEGIN CERTIFICATE-----\\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\\n-----END CERTIFICATE-----\"",
    r#"git_sha accepts "da39a3ee5e6b4b0d3255bfef95601890afd80709" rejects "da39a3ee5e6b4b0d3255bfef95601890afd8070" "DA39A3EE5E6B4B0D3255BFEF95601890AFD80709""#,
    r#"ip_private accepts "10.1.2.3" "192.168.0.1" "172.20.5.5" rejects "8.8.8.8" "172.32.0.1""#,
    r#"ip_loopback accepts "127.0.0.1" "::1" rejects "128.0.0.1" "10.0.0.1""#,
    r#"ip_linklocal accepts "169.254.10.10" "fe80::1" rejects "169.253.0.1""#,
    r#"log_level accepts "ERROR" "warn" "Info" rejects "ERRORS" "information""#,
    r#"http_method accepts "GET" "PATCH" rejects "get" "GETS""#,
    r#"http_status accepts "404" "100" "599" rejects "99" "600""#,
    r#"http_1xx accepts "101" rejects "200""#,
    r#"http_2xx accepts "204" rejects "301" "199""#,
    r#"http_3xx accepts "301" rejects "404""#,
    r#"http_4xx accepts "404" rejects "500""#,
    r#"http_5xx accepts "503" rejects "404""#,
    r#"currency accepts "USD" "EUR" "ZWG" rejects "usd" "XYZ""#,
    r#"country accepts "DE" "US" rejects "de" "ZZ""#,
    r#"weekday accepts "Monday" "fri" rejects "Mondays" "frid""#,
    r#"month accepts "January" "sep" "SEPT" rejects "Januar" "jn""#,
    r#"temperature_word accepts "temperature" "Kelvin" "CRYOGENIC" rejects "temperatured" "warmish""#,
    r#"temperature_cue accepts "temp" "T" "20°C" "300°F" rejects "5kg" "t""#,
    r#"length_word accepts "length" "Wide" "MILES" rejects "lengthy" "widen""#,
    r#"length_cue accepts "depth" "5km" "3in" rejects "5kg""#,
    r#"time_word accepts "latency" "Elapsed" "HOURS" rejects "latent" "hourglass""#,
    r#"time_cue accepts "timeout" "1500ms" "5 min" "3h20m" rejects "5kg""#,
    r#"mass_word accepts "weight" "Weighs" "TONNES" rejects "weightless" "tonal""#,
    r#"mass_cue accepts "payload" "5kg" "2 lb" rejects "5km""#,
    r#"current_word accepts "current" "Amps" "BREAKER" rejects "currently" "amplifier""#,
    r#"current_cue accepts "fuse" "5mA" "2 kA" rejects "5kg""#,
    r#"voltage_word accepts "voltage" "Battery" "RAIL" rejects "voltages2" "railing""#,
    r#"voltage_cue accepts "supply" "5kV" "3.3 mV" rejects "5kg""#,
    r#"power_word accepts "power" "Watts" "HEATER" rejects "powerful" "wattle""#,
    r#"power_cue accepts "consumption" "5kW" "2 hp" rejects "5kg""#,
    r#"force_word accepts "force" "Thrust" "TENSION" rejects "forced" "thrusting""#,
    r#"force_cue accepts "drag" "5kN" "2 lbf" rejects "5kg""#,
    r#"energy_word accepts "energy" "Calories" "KINETIC" rejects "energetic" "calorific""#,
    r#"energy_cue accepts "battery" "5kJ" "2 kWh" rejects "5kg""#,
    r#"volume_word accepts "volume" "Gallons" "TANK" rejects "volumes3" "tanker""#,
    r#"volume_cue accepts "capacity" "5mL" "2 mL" rejects "5kg""#,
    r#"pressure_word accepts "pressure" "Hydraulic" "PSI" rejects "pressured" "pumping""#,
    r#"pressure_cue accepts "boost" "5kPa" "2 psi" rejects "5kg""#,
    // The unit kinds read from the tokens around them. Each takes part of a
    // line and leaves the rest, so what it must do is stated with `reads`;
    // what it must not do is a plain `rejects`, since a quantity with no cue
    // in the window is read by nothing.
    // The last reject holds a newline between the number and the symbol, so
    // it says the window does not reach across one.
    "kelvin reads \"4.2K\" in \"cooled to 4.2K overnight\" reads \"300 K\" in \"T = 300 K\" reads \"77 K\" in \"the sample sat at 77 K in liquid nitrogen\" reads \"77 K\" in \"77 K, then the cryostat was drained\" rejects \"a 5K run\" \"10K followers this week\" \"see 5\\nK\"",
    r#"inch reads "3 in" in "a board 3 in wide" rejects "3 in a row" "we went 3 in""#,
    r#"meter reads "5 m" in "a pole 5 m tall" reads "5 m" in "5 m away, then stop" rejects "see item 5 m for details""#,
    r#"second reads "30 s" in "the timeout is 30 s" rejects "chapter 5 s""#,
    r#"hour reads "2 h" in "elapsed 2 h" rejects "figure 2 h""#,
    r#"gram reads "5 g" in "a dose of 5 g" rejects "item 5 g""#,
    r#"tonne reads "20 t" in "a load of 20 t" rejects "table 20 t""#,
    r#"ampere reads "5 A" in "the fuse is rated 5 A" rejects "figure 5 A""#,
    r#"volt reads "12 V" in "a supply of 12 V" rejects "chapter 12 V""#,
    r#"watt reads "500 W" in "a heater of 500 W" rejects "500 W Main St""#,
    r#"newton reads "10 N" in "a force of 10 N" rejects "10 N Broadway""#,
    r#"joule reads "5 J" in "energy of 5 J" rejects "5 J Smith""#,
    r#"calorie reads "200 cal" in "a snack of 200 cal" rejects "we saw 200 cal""#,
    r#"liter reads "50 L" in "a tank of 50 L" reads "2 l" in "drank 2 l of water" rejects "level 50 L""#,
    r#"gallon reads "5 gal" in "a tank of 5 gal" rejects "the 5 gal""#,
    r#"bar reads "2.5 bar" in "tire pressure 2.5 bar" rejects "went to 5 bar""#,
];

/// Every entry.
#[must_use]
pub fn entries() -> &'static [Entry] {
    ENTRIES
}

/// The id of the library kind called `name`.
#[must_use]
pub fn id_of(name: &str) -> Option<u8> {
    standard().id_of(name)
}

/// The name of the library kind `id`.
#[must_use]
pub fn name_of(id: u8) -> Option<&'static str> {
    let set = standard();
    set.shapes()
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.name.as_str())
        .or_else(|| set.pattern_kinds().iter().find(|k| k.id == id).map(|k| k.name.as_str()))
}

/// The library shape with `id`.
#[must_use]
pub fn shape_of(id: u8) -> Option<&'static TokenShape> {
    standard().shapes().iter().find(|s| s.id == id)
}

/// The library kind declared from a pattern with `id`.
#[must_use]
pub fn kind_of(id: u8) -> Option<&'static PatternKind> {
    standard().pattern_kinds().iter().find(|k| k.id == id)
}

/// The names of the unit kinds read from the context, in entry order: what
/// `\{qty}` holds beside the built-in kinds.
#[must_use]
pub fn context_kind_names() -> Vec<&'static str> {
    ENTRIES.iter().filter(|e| e.reads_context()).map(|e| e.name).collect()
}

/// Whether `id` is a unit kind the library reads from the context.
#[must_use]
pub fn is_context_kind(id: u8) -> bool {
    kind_of(id).is_some()
}

/// One pattern whose matches are the matches of every context kind in
/// `kinds` together: each kind's pattern is a number then its own branch,
/// so the union is the number then the branches as alternatives, and a
/// lex fuses every such kind in one pass over the stream. The kind a span
/// belongs to is the one whose guard accepts it. A kind whose pattern does
/// not open with a number joins as a whole alternative.
#[must_use]
pub fn context_union(kinds: &[&PatternKind]) -> Pattern {
    if let [only] = kinds {
        return only.pattern.clone();
    }
    let number = Pattern::Atom(Atom::Kind(TokenKind::Number));
    let mut branches = Vec::with_capacity(kinds.len());
    for k in kinds {
        match &k.pattern {
            Pattern::Concat(v) if v.len() >= 2 && v[0] == number => {
                branches.push(if v.len() == 2 { v[1].clone() } else { Pattern::Concat(v[1..].to_vec()) });
            }
            _ => {
                return Pattern::Alt(kinds.iter().map(|k| k.pattern.clone()).collect(), AltMode::First);
            }
        }
    }
    Pattern::Concat(vec![number, Pattern::Alt(branches, AltMode::First)])
}

/// The library sub-pattern called `name`.
#[must_use]
pub fn let_of(name: &str) -> Option<&'static Pattern> {
    standard().let_of(name)
}

/// The shapes a lex of `pattern`'s input has to run, or `None` where the
/// pattern names no library kind and the ordinary lex serves.
#[must_use]
pub fn shapes_for(pattern: &Pattern) -> Option<ShapeSet> {
    let ids = pattern.library_kinds();
    if ids.is_empty() {
        return None;
    }
    Some(ShapeSet::new().with_library_shapes(&ids))
}

#[cfg(test)]
mod tests {
    use super::*;


    /// Every shipped entry is named by a line of `TESTS` and meets it.
    #[test]
    fn every_entry_is_named_by_a_test_line_and_meets_it() {
        // Run through the same parser and the same checker a user's own
        // `test` lines take, so the library is held to its examples by the
        // code that holds a reader to theirs rather than by a second
        // implementation that could agree with the library and not with them.
        let mut shapes = crate::custom::ShapeSet::new();
        for (n, line) in TESTS.iter().enumerate() {
            if let Err(e) = shapes.declare_test(line, n + 1) {
                panic!("test line {}: {}", n + 1, e.msg);
            }
        }
        for e in ENTRIES {
            assert!(
                TESTS.iter().any(|l| l.split_whitespace().next() == Some(e.name)),
                "entry {} is named by no test line",
                e.name
            );
        }
        let failures = shapes.run_tests();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn the_quantity_class_reads_every_unit_kind_and_the_window_bounds_the_cue() {
        let qty = crate::parse("\\{qty}").expect("parses");
        let text = "cooled to 4.2K, 3 in wide, 2 h elapsed, 5kg, 10MB, 40%, 3.2 GHz";
        let spans = crate::scan(&qty, text.as_bytes());
        let got: Vec<&str> = spans.iter().map(|s| &text[s.start()..s.end()]).collect();
        assert_eq!(got, vec!["4.2K", "3 in", "2 h", "5kg", "10MB", "40%", "3.2 GHz"]);
        // A cue reaches every bare symbol in its window, so a text whose only
        // temperature word is far from the symbol reads a number and a word.
        assert!(crate::scan(&qty, b"a 5K run this week").is_empty());
        let cold = crate::parse("\\{qty}{<0\u{b0}C}").expect("parses");
        assert_eq!(crate::scan(&cold, b"cooled to 4.2K, then 20\xc2\xb0C, then -5\xc2\xb0C").len(), 2);
        // A cue at the window's edge counts; one token past it does not.
        let filler = "x ".repeat(CONTEXT_WINDOW - 1);
        let inside = format!("temperature {filler}300 K");
        let outside = format!("temperature x {filler}300 K");
        let kelvin = crate::parse("\\{kelvin}").expect("parses");
        assert_eq!(crate::scan(&kelvin, inside.as_bytes()).len(), 1, "the cue is the window's last token");
        assert!(crate::scan(&kelvin, outside.as_bytes()).is_empty(), "the cue is one token past the window");
        let after_inside = format!("300 K {filler}temperature");
        let after_outside = format!("300 K {filler}x temperature");
        assert_eq!(crate::scan(&kelvin, after_inside.as_bytes()).len(), 1);
        assert!(crate::scan(&kelvin, after_outside.as_bytes()).is_empty());
    }

    #[test]
    fn a_library_kind_is_lexed_only_for_a_pattern_that_names_it() {
        // Thirteen digits with a valid check are a number to `\N` and an
        // EAN-13 to `\{ean13}`; the pattern decides what the lexer makes.
        let text = b"code 4006381333931 sold";
        let number = crate::parse("\\N").expect("parses");
        let ean = crate::parse("\\{ean13}").expect("parses");
        assert_eq!(crate::scan(&number, text).len(), 1);
        assert_eq!(crate::scan(&ean, text).len(), 1);
        let both = crate::parse("\\{ean13} \\W").expect("parses");
        assert_eq!(crate::scan(&both, text).len(), 1, "the kind reads as one token before the word");
        assert!(crate::find(&ean, b"code 4006381333932 sold").is_none(), "a failed check is no EAN");
    }

    #[test]
    fn a_declaration_shadows_a_library_entry() {
        let mut set = ShapeSet::new();
        set.declare_text("let cve = \"CVE\" \"-\" \\N").expect("declares");
        let shadowed = crate::parser::parse_with_shapes("\\{cve}", &set).expect("parses");
        assert!(crate::scan(&shadowed, b"see CVE-2021 now").len() == 1, "the declared form matches");
        let library = crate::parse("\\{cve}").expect("parses");
        assert!(crate::scan(&library, b"see CVE-2021 now").is_empty(), "the library form needs the full id");
    }

    #[test]
    fn the_ids_and_names_round_trip() {
        for e in ENTRIES.iter().filter(|e| e.is_kind()) {
            let id = id_of(e.name).expect("every kind has an id");
            assert!(id >= LIBRARY_ID_BASE);
            assert_eq!(name_of(id), Some(e.name));
            let held = shape_of(id)
                .map(|s| s.name.as_str())
                .or_else(|| kind_of(id).map(|k| k.name.as_str()));
            assert_eq!(held, Some(e.name));
            assert_eq!(is_context_kind(id), e.reads_context());
        }
        assert!(let_of("http_2xx").is_some());
        assert!(id_of("http_2xx").is_none(), "a sub-pattern is not a kind");
        assert_eq!(context_kind_names().len(), ENTRIES.iter().filter(|e| e.reads_context()).count());
    }
}
