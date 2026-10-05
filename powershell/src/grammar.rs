//! Token grammars: named rules over TREX's typed tokens that parse any text,
//! with no parser written for its language. A grammar gives one
//! ordered-choice parse of an input; over every derivation, how many parses
//! there are and the probability of the best one or of all of them under the
//! grammar's `@p` weights; and over a run-together string and a dictionary,
//! the same for each way the string tiles into its words.

use pwrs::prelude::*;

use crate::axes::each_input;
use crate::common::{arg_err, read_err};

/// One node of a parse tree: a rule with the nodes it matched, or a token.
#[psclass(name = "Trex.ParseNode", show = "{Expression}")]
#[derive(Clone, Default)]
pub struct TrexParseNode {
    /// The file the text came from, on the root of a tree parsed from one;
    /// empty for a string and for every node under the root.
    pub path: String,
    /// The rule the node matched, empty for a token.
    pub rule: String,
    /// The token's text, or for a rule the texts of the tokens under it
    /// joined by single spaces.
    pub text: String,
    /// The nodes the rule matched, in order, each a Trex.ParseNode; none for
    /// a token.
    pub children: Vec<PsObject>,
    /// The tree under this node as an S-expression: a token as its text, a
    /// rule as `(rule child ...)`, and a rule of one child as that child.
    pub expression: String,
}

/// One way a run-together string tiles into dictionary words that the
/// grammar accepts.
#[psclass(name = "Trex.Tiling", show = "{Words}")]
#[derive(Clone, Default)]
pub struct TrexTiling {
    /// The words, in order; joined, they are the string.
    pub words: Vec<String>,
    /// The probability of the words' most probable parse under the
    /// grammar's `@p` weights.
    pub probability: f64,
    /// How many parses of the words the grammar has.
    pub parses: u64,
}

impl TrexTiling {
    fn of(tiling: trex::grammar::Tiling) -> Self {
        TrexTiling { words: tiling.words, probability: tiling.probability, parses: tiling.parses }
    }
}

/// A parse tree's node as the module writes it.
fn node_of(node: &trex::grammar::Node) -> PsResult<TrexParseNode> {
    match node {
        trex::grammar::Node::Terminal(text) => Ok(TrexParseNode {
            text: text.clone(),
            expression: node.sexpr(),
            ..TrexParseNode::default()
        }),
        trex::grammar::Node::Branch(rule, kids) => {
            let mut children = Vec::with_capacity(kids.len());
            let mut texts = Vec::with_capacity(kids.len());
            for kid in kids {
                let child = node_of(kid)?;
                if !child.text.is_empty() {
                    texts.push(child.text.clone());
                }
                children.push(child.into_ps()?);
            }
            Ok(TrexParseNode {
                rule: rule.clone(),
                text: texts.join(" "),
                children,
                expression: node.sexpr(),
                ..TrexParseNode::default()
            })
        }
    }
}

/// A token grammar, compiled.
#[psclass(name = "Trex.Grammar", mode = proxy)]
pub struct TrexGrammar {
    /// The rule a parse starts from.
    pub start: String,
    /// Every rule's name, in the order the grammar declares them.
    pub rules: Vec<String>,
    #[psfield(skip)]
    pub(crate) grammar: trex::Grammar,
}

impl TrexGrammar {
    /// The grammar `source` declares, started from `start` where one is
    /// named and from its first rule where none is.
    fn compile(source: &str, start: Option<&str>) -> PsResult<Self> {
        let mut grammar = trex::Grammar::parse(source)
            .map_err(|e| arg_err("TrexGrammarError", format!("grammar error at line {}: {}", e.line, e.msg)))?;
        if let Some(rule) = start {
            if !grammar.rule_names().contains(&rule) {
                return Err(arg_err("TrexGrammarStart", format!("the grammar declares no rule {rule:?}")));
            }
            grammar = grammar.with_start(rule);
        }
        Ok(TrexGrammar {
            start: grammar.start().to_string(),
            rules: grammar.rule_names().into_iter().map(str::to_string).collect(),
            grammar,
        })
    }
}

#[psmethods]
impl TrexGrammar {
    /// Compiles grammar source, one `name := alternatives` rule a line,
    /// starting from its first rule; New-TrexGrammar -Start names another.
    pub fn new(source: String) -> PsResult<Self> {
        TrexGrammar::compile(&source, None)
    }

    /// The parse of `text` from the start rule, or `$null` where it does not
    /// parse in full.
    pub fn parse(&self, text: String) -> PsResult<Option<TrexParseNode>> {
        self.grammar.parse_input(text.as_bytes()).map(|node| node_of(&node)).transpose()
    }

    /// Whether `text` parses in full under any derivation.
    pub fn test(&self, text: String) -> PsResult<bool> {
        Ok(self.grammar.recognizes(text.as_bytes()))
    }

    /// How many derivations of `text` the grammar has: 0 where it does not
    /// parse, more than 1 where the grammar is ambiguous for it.
    pub fn count_parses(&self, text: String) -> PsResult<u64> {
        Ok(self.grammar.count_parses(text.as_bytes()))
    }

    /// The probability of `text`'s most probable derivation under the
    /// grammar's `@p` weights; 0 where it does not parse.
    pub fn best_probability(&self, text: String) -> PsResult<f64> {
        Ok(self.grammar.best_parse_prob(text.as_bytes()))
    }

    /// The probability of all of `text`'s derivations together.
    pub fn total_probability(&self, text: String) -> PsResult<f64> {
        Ok(self.grammar.total_prob(text.as_bytes()))
    }

    /// How many ways the run-together `text` tiles into the words of
    /// `dictionary` with the grammar accepting the words.
    pub fn count_segmentations(&self, text: String, dictionary: Vec<String>) -> PsResult<u64> {
        Ok(self.grammar.count_segmentations(&text, &dictionary))
    }

    /// The probability of the most probable tiling of the run-together
    /// `text` into the words of `dictionary` that the grammar accepts.
    pub fn best_segmentation_probability(&self, text: String, dictionary: Vec<String>) -> PsResult<f64> {
        Ok(self.grammar.best_segmentation_prob(&text, &dictionary))
    }

    /// Every tiling of the run-together `text` into the words of
    /// `dictionary` that the grammar accepts, each with its most probable
    /// parse's probability and its parse count, most probable first.
    pub fn segment(&self, text: String, dictionary: Vec<String>) -> PsResult<Vec<TrexTiling>> {
        Ok(self.grammar.segment(&text, &dictionary).into_iter().map(TrexTiling::of).collect())
    }
}

/// Compiles a token grammar for Invoke-TrexGrammar and as an object with
/// Parse, Test, CountParses, BestProbability, TotalProbability, Segment,
/// CountSegmentations and BestSegmentationProbability methods.
///
/// A grammar is one rule a line, `name := alternatives`, alternatives
/// separated by `|`: `<rule>` another rule, `"lit"` a token by its text,
/// `number`, `ident`, `string`, `ip`, `url`, `email`, `time` or `punct` a
/// token by its kind, `*`, `+` and `?` after a symbol its repetition,
/// `( a | b )` a group, and `@p` after an alternative its weight, 1 when
/// absent. `#` starts a comment. A rule that begins with itself is left
/// recursive, which is how an operator grammar writes precedence. The first
/// rule is the start unless -Start names another.
///
/// # Examples
/// $arith = New-TrexGrammar -Path ./arith.grammar
/// New-TrexGrammar 'sum := <sum> "+" number | number'
/// Get-Content ./calls.grammar -Raw | New-TrexGrammar -Start call
#[cmdlet(verb = "New", noun = "TrexGrammar", alias = "New-TxGrammar", default_parameter_set = "Source", output = ["Trex.Grammar"])]
#[derive(Default)]
pub struct NewTrexGrammar {
    /// The grammar's source text.
    #[param(mandatory, position = 0, set = "Source", value_from_pipeline)]
    pub source: String,
    /// A file holding the grammar.
    #[param(mandatory, set = "Path")]
    pub path: String,
    /// The rule a parse starts from, in place of the first one declared.
    #[param]
    pub start: Option<String>,
}

impl Cmdlet for NewTrexGrammar {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let source = if self.path.is_empty() {
            self.source.clone()
        } else {
            let mut resolved = ps.resolve_path(&self.path, false)?;
            let Some(file) = resolved.pop() else {
                return Err(arg_err("TrexGrammarPath", format!("{} names no file", self.path)));
            };
            std::fs::read_to_string(&file).map_err(|e| read_err(&file, e))?
        };
        ps.write(TrexGrammar::compile(&source, self.start.as_deref())?)
    }
}

/// What Invoke-TrexGrammar writes for an input.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Tree,
    Count,
    Best,
    Total,
}

/// Parses text against a token grammar and writes the parse tree, or with
/// -Count, -Best or -Probability a value over every derivation.
///
/// The parse is one ordered-choice parse from the start rule, written as a
/// Trex.ParseNode; text that does not parse in full is an error record.
/// -Count writes how many derivations the grammar has of the text, which
/// says how ambiguous it is; -Best the probability of the most probable one
/// under the grammar's `@p` weights, which resolves the ambiguity; and
/// -Probability all of them together. -Segment reads a run-together string
/// over the words of -Dictionary, every way it tiles into them, and writes
/// each tiling the grammar accepts as a Trex.Tiling, most probable first;
/// with -Count how many there are, or with -Best the probability of the most
/// probable one.
///
/// Each input gets one answer, in the order read. The count and the
/// probabilities come from a chart cubic in the token count, so a long text
/// is parsed a sentence at a time.
///
/// # Examples
/// Invoke-TrexGrammar $arith '2 + 3 * 4'
/// '1 + 2', '1 + 2 + 3' | Invoke-TrexGrammar $ambiguous -Count
/// Invoke-TrexGrammar $words -Segment 'thecatsat' -Dictionary the, cat, cats, at, sat -Best
/// Invoke-TrexGrammar $words -Segment 'thecatsat' -Dictionary the, cat, cats, at, sat
#[cmdlet(verb = "Invoke", noun = "TrexGrammar", alias = "Invoke-TxGrammar", default_parameter_set = "Text", output = ["Trex.ParseNode", "Trex.Tiling", "System.UInt64", "System.Double"])]
#[derive(Default)]
pub struct InvokeTrexGrammar {
    /// The grammar: a Trex.Grammar from New-TrexGrammar, or grammar source
    /// text.
    #[param(mandatory, position = 0)]
    pub grammar: PsObject,
    /// The text to parse; each string piped in is parsed on its own.
    #[param(mandatory, position = 1, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files to parse, each whole; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files to parse, read as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// A run-together string to tile into the words of -Dictionary.
    #[param(mandatory, set = "Segment")]
    pub segment: String,
    /// The words -Segment tiles into.
    #[param(mandatory, set = "Segment")]
    pub dictionary: Vec<String>,
    /// The rule a parse starts from, in place of the grammar's own.
    #[param]
    pub start: Option<String>,
    /// Writes how many derivations the grammar has of each input, or with
    /// -Segment how many tilings it accepts.
    #[param]
    pub count: bool,
    /// Writes the probability of each input's most probable derivation, or
    /// with -Segment of its most probable tiling.
    #[param]
    pub best: bool,
    /// Writes the probability of all of each input's derivations together.
    #[param(set = ["Text", "Path", "LiteralPath"])]
    pub probability: bool,
    compiled: Option<TrexGrammar>,
    answer: Option<Answer>,
}

impl InvokeTrexGrammar {
    /// Writes the answer for one input; `path` is empty for a string.
    fn answer(&self, ps: &Pipeline<'_>, text: &str, path: &str) -> PsResult<()> {
        let (Some(g), Some(answer)) = (&self.compiled, self.answer) else {
            return Ok(());
        };
        let grammar = &g.grammar;
        match answer {
            Answer::Count => ps.write(grammar.count_parses(text.as_bytes())),
            Answer::Best => ps.write(grammar.best_parse_prob(text.as_bytes())),
            Answer::Total => ps.write(grammar.total_prob(text.as_bytes())),
            Answer::Tree => match grammar.parse_input(text.as_bytes()) {
                Some(node) => {
                    let mut root = node_of(&node)?;
                    root.path = path.to_string();
                    ps.write(root)
                }
                None => {
                    let shown = if path.is_empty() { format!("{text:?}") } else { path.to_string() };
                    ps.write_error(&PsError::new(
                        ErrorCategory::InvalidData,
                        "TrexNoParse",
                        format!("{shown} does not parse in full from rule {}", g.start),
                    ))
                }
            },
        }
    }
}

impl Cmdlet for InvokeTrexGrammar {
    fn begin(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        let mut asked = Vec::new();
        if self.count {
            asked.push(("-Count", Answer::Count));
        }
        if self.best {
            asked.push(("-Best", Answer::Best));
        }
        if self.probability {
            asked.push(("-Probability", Answer::Total));
        }
        let answer = match asked.as_slice() {
            [] => Answer::Tree,
            [(_, one)] => *one,
            [(a, _), (b, _), ..] => {
                return Err(arg_err("TrexGrammarAnswer", format!("{a} and {b} each say what is written; give one")));
            }
        };
        let given = &self.grammar;
        let compiled = if given.is_null() {
            return Err(arg_err("TrexNoGrammar", "a grammar is required"));
        } else if given.type_name()? == "Trex.Grammar" {
            let proxy = PsProxy::<TrexGrammar>::from_ps(given)?;
            let grammar = proxy.with(|g| g.grammar.clone())?;
            let source_start = grammar.start().to_string();
            let start = self.start.clone().unwrap_or(source_start);
            if !grammar.rule_names().contains(&start.as_str()) {
                return Err(arg_err("TrexGrammarStart", format!("the grammar declares no rule {start:?}")));
            }
            let grammar = grammar.with_start(&start);
            TrexGrammar {
                start: grammar.start().to_string(),
                rules: grammar.rule_names().into_iter().map(str::to_string).collect(),
                grammar,
            }
        } else {
            TrexGrammar::compile(&String::from_ps(given)?, self.start.as_deref())?
        };
        self.compiled = Some(compiled);
        self.answer = Some(answer);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if !self.segment.is_empty() {
            let Some(g) = &self.compiled else {
                return Ok(());
            };
            return match self.answer {
                Some(Answer::Best) => ps.write(g.grammar.best_segmentation_prob(&self.segment, &self.dictionary)),
                Some(Answer::Count) => ps.write(g.grammar.count_segmentations(&self.segment, &self.dictionary)),
                _ => {
                    for tiling in g.grammar.segment(&self.segment, &self.dictionary) {
                        ps.write(TrexTiling::of(tiling))?;
                    }
                    Ok(())
                }
            };
        }
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            self.answer(ps, &input.text, &input.path)
        })
    }
}
