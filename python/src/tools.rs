//! The tools beside the scan: token grammars over a text's tokens and over
//! the ways a run-together string tiles into words, byte-pair encoders, a
//! tree's index, and presence prefilters over a corpus.

use std::path::{Path, PathBuf};

use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};

use crate::library::repr_of;
use crate::{Input, PathArg, Pattern};

/// One input as the tools read it: `text`, a str or bytes, or the file at
/// `path`, decoded as a scan reads it, with its name. A file holding a NUL
/// byte is refused as binary.
pub(crate) fn one_input(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
) -> PyResult<(Vec<u8>, Option<String>)> {
    match (text, path) {
        (Some(obj), None) => Ok((Input::of(obj)?.bytes().to_vec(), None)),
        (None, Some(path)) => {
            let name = path.display().to_string();
            let raw = py.detach(|| std::fs::read(&path)).map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
            if trex::files::is_binary(&raw) {
                return Err(PyValueError::new_err(format!("{name} holds a NUL byte and is binary")));
            }
            Ok((trex::encoding::decode(raw), Some(name)))
        }
        (Some(_), Some(_)) | (None, None) => {
            Err(PyTypeError::new_err("give the text, a str or bytes, or path=, a file's path; one of them"))
        }
    }
}

/// The one corpus `text`, a str or bytes, or the files `path` names hold:
/// a directory walked as `trex.files` walks one, each file decoded as a
/// scan reads it, a newline between two files. A file holding a NUL byte is
/// passed over, and refused where it was named alone.
fn corpus_of(py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathArg>) -> PyResult<Vec<u8>> {
    match (text, path) {
        (Some(obj), None) => Ok(Input::of(obj)?.bytes().to_vec()),
        (None, Some(paths)) => {
            let walked = crate::rules::walked(py, paths, &trex::files::WalkOptions::default())?;
            let mut errors = walked.errors;
            let mut corpus = Vec::new();
            for (name, file) in &walked.files {
                match py.detach(|| std::fs::read(file)) {
                    Err(e) => errors.push(format!("{name}: {e}")),
                    Ok(raw) if trex::files::is_binary(&raw) => {
                        if walked.lone {
                            return Err(PyValueError::new_err(format!("{name} holds a NUL byte and is binary")));
                        }
                    }
                    Ok(raw) => {
                        if !corpus.is_empty() {
                            corpus.push(b'\n');
                        }
                        corpus.extend(trex::encoding::decode(raw));
                    }
                }
            }
            if !errors.is_empty() {
                return Err(PyOSError::new_err(errors.join("; ")));
            }
            Ok(corpus)
        }
        (Some(_), Some(_)) | (None, None) => Err(PyTypeError::new_err(
            "give the text, a str or bytes, or path=, a file or directory's path or a list of them; one of them",
        )),
    }
}

/// One node of a parse tree: a rule with the nodes it matched, or a token.
#[pyclass(frozen, module = "trex")]
pub(crate) struct ParseNode {
    /// The file the text came from, on the root of a tree parsed from one;
    /// `None` for a text and for every node under the root.
    #[pyo3(get)]
    path: Option<String>,
    /// The rule the node matched; `None` for a token.
    #[pyo3(get)]
    rule: Option<String>,
    /// The token's text, or for a rule the texts of the tokens under it
    /// joined by single spaces.
    #[pyo3(get)]
    text: String,
    /// The nodes the rule matched, in order; none for a token.
    #[pyo3(get)]
    children: Vec<Py<ParseNode>>,
    /// The tree under this node as an S-expression: a token as its text, a
    /// rule as `(rule child ...)`, and a rule of one child as that child.
    #[pyo3(get)]
    expression: String,
}

impl ParseNode {
    fn of(py: Python<'_>, node: &trex::grammar::Node, path: Option<String>) -> PyResult<ParseNode> {
        match node {
            trex::grammar::Node::Terminal(text) => {
                Ok(ParseNode { path, rule: None, text: text.clone(), children: Vec::new(), expression: node.sexpr() })
            }
            trex::grammar::Node::Branch(rule, kids) => {
                let mut children = Vec::with_capacity(kids.len());
                let mut texts = Vec::with_capacity(kids.len());
                for kid in kids {
                    let child = ParseNode::of(py, kid, None)?;
                    if !child.text.is_empty() {
                        texts.push(child.text.clone());
                    }
                    children.push(Py::new(py, child)?);
                }
                Ok(ParseNode {
                    path,
                    rule: Some(rule.clone()),
                    text: texts.join(" "),
                    children,
                    expression: node.sexpr(),
                })
            }
        }
    }
}

#[pymethods]
impl ParseNode {
    /// The node as a plain dict of its fields, the children as dicts of
    /// their own.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("path", &self.path)?;
        d.set_item("rule", &self.rule)?;
        d.set_item("text", &self.text)?;
        let children = self
            .children
            .iter()
            .map(|c| c.bind(py).call_method0("to_dict"))
            .collect::<PyResult<Vec<_>>>()?;
        d.set_item("children", children)?;
        d.set_item("expression", &self.expression)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("ParseNode({})", repr_of(py, &self.expression)?))
    }
}

/// One way a run-together string tiles into dictionary words that a grammar
/// accepts.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Tiling {
    /// The words, in order; joined, they are the string.
    #[pyo3(get)]
    words: Vec<String>,
    /// The probability of the words' most probable parse under the
    /// grammar's `@p` weights.
    #[pyo3(get)]
    probability: f64,
    /// How many parses of the words the grammar has.
    #[pyo3(get)]
    parses: u64,
}

#[pymethods]
impl Tiling {
    /// The tiling as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("words", &self.words)?;
        d.set_item("probability", self.probability)?;
        d.set_item("parses", self.parses)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let words = self.words.iter().map(|w| repr_of(py, w)).collect::<PyResult<Vec<_>>>()?;
        Ok(format!("Tiling(words=[{}], probability={}, parses={})", words.join(", "), self.probability, self.parses))
    }
}

/// A token grammar: named rules over TREX's typed tokens that parse any
/// text with no parser written for its language, one `name :=
/// alternatives` rule a line. Alternatives are separated by `|`: `<rule>`
/// another rule, `"lit"` a token by its text, `number`, `ident`, `string`,
/// `ip`, `url`, `email`, `time` or `punct` a token by its kind, `*`, `+` and
/// `?` a repetition, `( a | b )` a group, and `@p` after an alternative its
/// weight, 1 when absent. The first rule is the start unless `start=` names
/// another. A grammar that does not parse raises `ValueError` naming the
/// line.
///
/// Each method reads the text positionally, a str or bytes, or `path=`, a
/// file decoded as a scan reads one.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Grammar {
    /// The rule a parse starts from.
    #[pyo3(get)]
    start: String,
    /// Every rule's name, in the order the grammar declares them.
    #[pyo3(get)]
    rules: Vec<String>,
    grammar: trex::Grammar,
}

#[pymethods]
impl Grammar {
    #[new]
    #[pyo3(signature = (source, *, start = None))]
    fn new(source: &str, start: Option<&str>) -> PyResult<Self> {
        let mut grammar = trex::Grammar::parse(source)
            .map_err(|e| PyValueError::new_err(format!("grammar error at line {}: {}", e.line, e.msg)))?;
        if let Some(rule) = start {
            if !grammar.rule_names().contains(&rule) {
                return Err(PyValueError::new_err(format!("the grammar declares no rule {rule:?}")));
            }
            grammar = grammar.with_start(rule);
        }
        Ok(Grammar {
            start: grammar.start().to_string(),
            rules: grammar.rule_names().into_iter().map(str::to_string).collect(),
            grammar,
        })
    }

    /// The parse of the text from the start rule, one ordered-choice parse,
    /// or `None` where it does not parse in full.
    #[pyo3(signature = (text = None, *, path = None))]
    fn parse(&self, py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<Option<ParseNode>> {
        let (bytes, name) = one_input(py, text, path)?;
        let node = py.detach(|| self.grammar.parse_input(&bytes));
        node.map(|n| ParseNode::of(py, &n, name)).transpose()
    }

    /// Whether the text parses in full under any derivation.
    #[pyo3(signature = (text = None, *, path = None))]
    fn accepts(&self, py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<bool> {
        let (bytes, _) = one_input(py, text, path)?;
        Ok(py.detach(|| self.grammar.recognizes(&bytes)))
    }

    /// How many derivations of the text the grammar has: 0 where it does not
    /// parse, more than 1 where the grammar is ambiguous for it.
    #[pyo3(signature = (text = None, *, path = None))]
    fn count(&self, py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<u64> {
        let (bytes, _) = one_input(py, text, path)?;
        Ok(py.detach(|| self.grammar.count_parses(&bytes)))
    }

    /// The probability of the text's most probable derivation under the
    /// grammar's `@p` weights; 0 where it does not parse.
    #[pyo3(signature = (text = None, *, path = None))]
    fn best(&self, py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<f64> {
        let (bytes, _) = one_input(py, text, path)?;
        Ok(py.detach(|| self.grammar.best_parse_prob(&bytes)))
    }

    /// The probability of all of the text's derivations together.
    #[pyo3(signature = (text = None, *, path = None))]
    fn probability(&self, py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<f64> {
        let (bytes, _) = one_input(py, text, path)?;
        Ok(py.detach(|| self.grammar.total_prob(&bytes)))
    }

    /// Every tiling of the run-together `text` into the words of
    /// `dictionary` that the grammar accepts, each with its most probable
    /// parse's probability and its parse count, most probable first, ties
    /// in the order the tilings read from the left. How many there are can
    /// grow exponentially with the text's length.
    fn segment(&self, py: Python<'_>, text: &str, dictionary: Vec<String>) -> Vec<Tiling> {
        py.detach(|| self.grammar.segment(text, &dictionary))
            .into_iter()
            .map(|t| Tiling { words: t.words, probability: t.probability, parses: t.parses })
            .collect()
    }

    /// How many ways the run-together `text` tiles into the words of
    /// `dictionary` with the grammar accepting the words, its parses of each
    /// tiling counted.
    fn count_segmentations(&self, py: Python<'_>, text: &str, dictionary: Vec<String>) -> u64 {
        py.detach(|| self.grammar.count_segmentations(text, &dictionary))
    }

    /// The probability of the most probable tiling of the run-together
    /// `text` into the words of `dictionary` that the grammar accepts.
    fn best_segmentation_probability(&self, py: Python<'_>, text: &str, dictionary: Vec<String>) -> f64 {
        py.detach(|| self.grammar.best_segmentation_prob(text, &dictionary))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Grammar(start={}, rules={})", repr_of(py, &self.start)?, self.rules.len()))
    }
}

/// A byte-pair encoder: the merges training learned from a corpus, in
/// order. Encoding splits each word into its characters and applies the
/// merges in order, so any word, one unseen in training too, splits into
/// subwords the model knows, the last of each word ending in `</w>`.
///
/// `Bpe(model)` reads a model's text, one merge a line with its two symbols
/// separated by a tab, as `save` writes it and `trex bpe train` does.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Bpe {
    /// How many merges the model holds.
    #[pyo3(get)]
    merges: usize,
    /// The model as text, one merge a line with its two symbols separated
    /// by a tab.
    #[pyo3(get)]
    model: String,
    bpe: trex::bpe::Bpe,
}

impl Bpe {
    fn of(bpe: trex::bpe::Bpe) -> Bpe {
        Bpe { merges: bpe.len(), model: bpe.to_lines(), bpe }
    }
}

#[pymethods]
impl Bpe {
    #[new]
    fn new(model: &str) -> PyResult<Self> {
        trex::bpe::Bpe::parse(model).map(Bpe::of).map_err(PyValueError::new_err)
    }

    /// Learns an encoder from `text`, a str or bytes, or the files `path=`
    /// names, read as one corpus: each word its characters and an
    /// end-of-word mark, every round merging the most frequent adjacent pair,
    /// ties broken by the pair's text, so the same corpus always learns the
    /// same model. `max_bytes` trains on the start of a large corpus, cut
    /// back to a whole word.
    #[staticmethod]
    #[pyo3(signature = (text = None, *, path = None, merges = 1000, max_bytes = None))]
    fn train(
        py: Python<'_>,
        text: Option<&Bound<'_, PyAny>>,
        path: Option<PathArg>,
        merges: usize,
        max_bytes: Option<usize>,
    ) -> PyResult<Bpe> {
        let corpus = corpus_of(py, text, path)?;
        let sample = match max_bytes {
            Some(cap) => trex::bpe::sample(&corpus, cap),
            None => &corpus[..],
        };
        Ok(Bpe::of(py.detach(|| trex::bpe::Bpe::train(sample, merges))))
    }

    /// Reads an encoder from a model file, one `save` or `trex bpe train`
    /// wrote; a line that is not a merge raises `ValueError` naming it.
    #[staticmethod]
    fn load(py: Python<'_>, path: PathBuf) -> PyResult<Bpe> {
        let name = path.display().to_string();
        let model =
            py.detach(|| std::fs::read_to_string(&path)).map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
        trex::bpe::Bpe::parse(&model).map(Bpe::of).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
    }

    /// Writes the model to the file at `path`, which `Bpe.load` and `trex bpe
    /// encode --model` read.
    fn save(&self, py: Python<'_>, path: PathBuf) -> PyResult<()> {
        let name = path.display().to_string();
        py.detach(|| std::fs::write(&path, &self.model)).map_err(|e| PyOSError::new_err(format!("{name}: {e}")))
    }

    /// The text, a str or bytes, or the file at `path=`, split into the
    /// model's subwords, word by word.
    #[pyo3(signature = (text = None, *, path = None))]
    fn encode(&self, py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<Vec<String>> {
        let (bytes, _) = one_input(py, text, path)?;
        Ok(py.detach(|| self.bpe.encode(&bytes)))
    }

    fn __len__(&self) -> usize {
        self.merges
    }

    fn __repr__(&self) -> String {
        format!("Bpe(merges={})", self.merges)
    }
}

/// A tree's index: one small summary a file, written at the tree's root as
/// `.trex-index` so a later scan opens only the files that can match. It is
/// an optimization only: a file the index has not seen, or one that changed
/// since, is always a candidate.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Index {
    /// The tree the index covers.
    #[pyo3(get)]
    root: String,
    /// The index file, at the tree's root.
    #[pyo3(get)]
    path: String,
    /// How many files it covers.
    #[pyo3(get)]
    files: usize,
    index: trex::index::Index,
}

impl Index {
    fn of(root: &Path, index: trex::index::Index) -> Index {
        Index {
            root: root.display().to_string(),
            path: root.join(trex::index::INDEX_FILE).display().to_string(),
            files: index.len(),
            index,
        }
    }
}

/// The walk `hidden`, `no_ignore` and `globs` ask for, refused where a glob
/// does not parse.
fn walk_of(hidden: bool, no_ignore: bool, globs: Option<Vec<String>>) -> PyResult<trex::files::WalkOptions> {
    let walk = trex::files::WalkOptions {
        hidden,
        no_ignore,
        globs: globs.unwrap_or_default(),
        ..trex::files::WalkOptions::default()
    };
    walk.check().map_err(PyValueError::new_err)?;
    Ok(walk)
}

/// Refuses a path that is not a directory, since an index covers a tree.
fn tree(path: &Path) -> PyResult<()> {
    if path.is_dir() {
        return Ok(());
    }
    Err(PyValueError::new_err(format!("{} is not a directory; an index covers a tree", path.display())))
}

#[pymethods]
impl Index {
    /// Writes the index of the tree at `path` at its root, as `trex index`
    /// and New-TrexIndex write one, and gives it. The tree is walked as a
    /// scan walks it, hidden files and those an ignore rule excludes left
    /// out unless asked for, and `globs` keeping or dropping walked files.
    #[staticmethod]
    #[pyo3(signature = (path, *, hidden = false, no_ignore = false, globs = None))]
    fn build(
        py: Python<'_>,
        path: PathBuf,
        hidden: bool,
        no_ignore: bool,
        globs: Option<Vec<String>>,
    ) -> PyResult<Index> {
        tree(&path)?;
        let walk = walk_of(hidden, no_ignore, globs)?;
        let root = path.to_string_lossy().into_owned();
        let (sources, errors) = py.detach(|| trex::files::collect(std::slice::from_ref(&root), &walk));
        if !errors.is_empty() {
            return Err(PyOSError::new_err(errors.join("; ")));
        }
        let files: Vec<PathBuf> = sources
            .into_iter()
            .filter_map(|s| match s {
                trex::files::Source::File(p) => Some(p),
                trex::files::Source::Stdin => None,
            })
            .collect();
        let index = py.detach(|| trex::index::Index::build(&path, &files));
        let written = path.join(trex::index::INDEX_FILE);
        py.detach(|| index.save(&path))
            .map_err(|e| PyOSError::new_err(format!("{}: {e}", written.display())))?;
        Ok(Index::of(&path, index))
    }

    /// The index at the root of the tree at `path`: `FileNotFoundError`
    /// where there is none, `ValueError` where the file there is not one
    /// this TREX wrote.
    #[staticmethod]
    fn load(py: Python<'_>, path: PathBuf) -> PyResult<Index> {
        tree(&path)?;
        match py.detach(|| trex::index::Index::load(&path)) {
            Some(index) => Ok(Index::of(&path, index)),
            None => {
                let file = path.join(trex::index::INDEX_FILE);
                if file.exists() {
                    Err(PyValueError::new_err(format!("{} is not an index this trex wrote", file.display())))
                } else {
                    Err(PyFileNotFoundError::new_err(format!(
                        "{} holds no index; Index.build writes one",
                        path.display()
                    )))
                }
            }
        }
    }

    /// The files of the tree a scan of `pattern`, a `Pattern` or a pattern's
    /// source, must open: every file the walk finds that the index cannot
    /// rule out, one it has not seen or that changed since among them.
    #[pyo3(signature = (pattern, *, hidden = false, no_ignore = false, globs = None))]
    fn candidates(
        &self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        hidden: bool,
        no_ignore: bool,
        globs: Option<Vec<String>>,
    ) -> PyResult<Vec<String>> {
        let compiled = if pattern.is_instance_of::<PyString>() {
            Pattern::new(pattern.extract::<&str>()?, None)?.inner
        } else {
            pattern
                .cast::<Pattern>()
                .map_err(|e| PyTypeError::new_err(format!("candidates() takes a trex.Pattern or a pattern's source: {e}")))?
                .get()
                .inner
                .clone()
        };
        let walk = walk_of(hidden, no_ignore, globs)?;
        let root = self.root.clone();
        let (sources, errors) = py.detach(|| trex::files::collect(std::slice::from_ref(&root), &walk));
        if !errors.is_empty() {
            return Err(PyOSError::new_err(errors.join("; ")));
        }
        Ok(py.detach(|| {
            sources
                .iter()
                .filter_map(|s| match s {
                    trex::files::Source::File(p) if !self.index.refuses(&compiled, p) => Some(s.name()),
                    trex::files::Source::File(_) | trex::files::Source::Stdin => None,
                })
                .collect()
        }))
    }

    /// The index as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("root", &self.root)?;
        d.set_item("path", &self.path)?;
        d.set_item("files", self.files)?;
        Ok(d)
    }

    fn __len__(&self) -> usize {
        self.files
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Index(root={}, files={})", repr_of(py, &self.root)?, self.files))
    }
}

/// A filter built over a corpus.
enum Built {
    Bloom(trex::prefilter::BloomFilter),
    Cuckoo(trex::prefilter::CuckooFilter),
    Xor(trex::prefilter::XorFilter),
}

impl Built {
    fn might_contain(&self, literal: &[u8]) -> bool {
        use trex::prefilter::Membership;
        match self {
            Built::Bloom(f) => f.might_contain(literal),
            Built::Cuckoo(f) => f.might_contain(literal),
            Built::Xor(f) => f.might_contain(literal),
        }
    }
}

/// The bytes of `literal`, a str or bytes.
fn literal_bytes(literal: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    Ok(Input::of(literal)?.bytes().to_vec())
}

/// What a filter answers for one literal, beside what an exact search of
/// the corpus finds.
#[pyclass(frozen, module = "trex")]
pub(crate) struct LiteralTest {
    /// The literal asked about, as it was given.
    #[pyo3(get)]
    literal: Py<PyAny>,
    /// The filter that answered: `"bloom"`, `"cuckoo"` or `"xor"`.
    #[pyo3(get)]
    filter: &'static str,
    /// Whether the filter says the literal might occur; `False` is exact.
    #[pyo3(get)]
    might_occur: bool,
    /// Whether an exact search finds the literal in the corpus. A literal
    /// the filter says might occur that does not is a false positive.
    #[pyo3(get)]
    occurs: bool,
}

#[pymethods]
impl LiteralTest {
    /// The test as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("literal", self.literal.bind(py))?;
        d.set_item("filter", self.filter)?;
        d.set_item("might_occur", self.might_occur)?;
        d.set_item("occurs", self.occurs)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "LiteralTest(literal={}, filter={}, might_occur={}, occurs={})",
            self.literal.bind(py).repr()?,
            repr_of(py, self.filter)?,
            if self.might_occur { "True" } else { "False" },
            if self.occurs { "True" } else { "False" }
        ))
    }
}

/// How one filter kept its contract over a corpus.
#[pyclass(frozen, module = "trex")]
pub(crate) struct FilterCheck {
    /// The filter probed: `"bloom"`, `"cuckoo"` or `"xor"`.
    #[pyo3(get)]
    filter: &'static str,
    /// The substrings of the corpus probed, each present by construction.
    #[pyo3(get)]
    present_probes: usize,
    /// The present probes the filter reported absent, which its contract
    /// forbids.
    #[pyo3(get)]
    false_negatives: usize,
    /// The byte strings probed that an exact search finds nowhere in the
    /// corpus.
    #[pyo3(get)]
    absent_probes: usize,
    /// The absent probes the filter rejected, each a scan it saves.
    #[pyo3(get)]
    absent_rejected: usize,
}

#[pymethods]
impl FilterCheck {
    /// The check as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("filter", self.filter)?;
        d.set_item("present_probes", self.present_probes)?;
        d.set_item("false_negatives", self.false_negatives)?;
        d.set_item("absent_probes", self.absent_probes)?;
        d.set_item("absent_rejected", self.absent_rejected)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "FilterCheck(filter={}, false_negatives={}, absent_rejected={} of {})",
            repr_of(py, self.filter)?,
            self.false_negatives,
            self.absent_rejected,
            self.absent_probes
        ))
    }
}

/// A presence filter built over a corpus's four-byte n-grams, which answers
/// whether a literal might occur in it: a `False` is exact, since a filter
/// never reports a literal that occurs as absent, and a `True` can be a
/// false positive, which `test` settles with an exact search. A literal
/// shorter than one n-gram is never rejected.
///
/// The corpus is `text`, a str or bytes, or the files `path=` names, read as
/// one; `kind` is `"bloom"`, `"cuckoo"` or `"xor"`.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Prefilter {
    /// Which filter it is: `"bloom"`, `"cuckoo"` or `"xor"`.
    #[pyo3(get)]
    filter: &'static str,
    /// The size of the corpus it was built over, in bytes as UTF-8.
    #[pyo3(get)]
    bytes: usize,
    built: Built,
    corpus: Vec<u8>,
}

#[pymethods]
impl Prefilter {
    #[new]
    #[pyo3(signature = (text = None, *, path = None, kind = "bloom"))]
    fn new(py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathArg>, kind: &str) -> PyResult<Self> {
        let corpus = corpus_of(py, text, path)?;
        let (filter, built) = match kind {
            "bloom" => ("bloom", py.detach(|| Built::Bloom(trex::prefilter::BloomFilter::build(&corpus)))),
            "cuckoo" => ("cuckoo", py.detach(|| Built::Cuckoo(trex::prefilter::CuckooFilter::build(&corpus)))),
            "xor" => ("xor", py.detach(|| Built::Xor(trex::prefilter::XorFilter::build(&corpus)))),
            other => {
                return Err(PyValueError::new_err(format!("kind takes 'bloom', 'cuckoo' or 'xor', not {other:?}")));
            }
        };
        Ok(Prefilter { filter, bytes: corpus.len(), built, corpus })
    }

    /// Whether `literal`, a str or bytes, might occur in the corpus: `False`
    /// is exact, `True` can be a false positive.
    fn might_contain(&self, literal: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.built.might_contain(&literal_bytes(literal)?))
    }

    /// Each of `literals` tested against the filter, beside what an exact
    /// search of the corpus finds, so a false positive reads as one.
    #[pyo3(signature = (*literals))]
    fn test(&self, py: Python<'_>, literals: Vec<Bound<'_, PyAny>>) -> PyResult<Vec<LiteralTest>> {
        let mut out = Vec::with_capacity(literals.len());
        for literal in &literals {
            let bytes = literal_bytes(literal)?;
            let might_occur = self.built.might_contain(&bytes);
            let occurs = might_occur && py.detach(|| trex::byte_simd::contains(&self.corpus, &bytes));
            out.push(LiteralTest { literal: literal.clone().unbind(), filter: self.filter, might_occur, occurs });
        }
        Ok(out)
    }

    /// Every filter's contract probed over the corpus: substrings of it,
    /// which each must report as present, and byte strings found nowhere in
    /// it, the share of which a filter rejects being the scanning it saves.
    fn verify(&self, py: Python<'_>) -> Vec<FilterCheck> {
        py.detach(|| trex::prefilter::verify(&self.corpus))
            .into_iter()
            .map(|check| FilterCheck {
                filter: check.name,
                present_probes: check.present,
                false_negatives: check.false_negatives,
                absent_probes: check.absent,
                absent_rejected: check.rejected,
            })
            .collect()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Prefilter(filter={}, bytes={})", repr_of(py, self.filter)?, self.bytes))
    }
}
