//! Byte-pair encoding: a subword tokenizer learned from a corpus, for text
//! whose words the lexer's whole-token boundaries do not fit. A model is the
//! ordered list of the merges training learned. Encoding splits each word
//! into its characters and applies the merges in order, so any word, one
//! unseen in training too, splits into subwords the model knows, the last of
//! each word ending in `</w>`.

use pwrs::prelude::*;

use crate::axes::each_input;
use crate::common::{arg_err, read_err};

/// A byte-pair encoder: the merges training learned, in order.
#[psclass(name = "Trex.Bpe", mode = proxy)]
pub struct TrexBpe {
    /// How many merges the model holds.
    pub merges: i64,
    /// The model as text, one merge a line with its two symbols separated by
    /// a tab, as Export-TrexBpe writes it.
    pub model: String,
    #[psfield(skip)]
    pub(crate) bpe: trex::bpe::Bpe,
}

impl TrexBpe {
    fn of(bpe: trex::bpe::Bpe) -> Self {
        TrexBpe { merges: bpe.len() as i64, model: bpe.to_lines(), bpe }
    }

    /// The encoder a model's text holds.
    fn read(model: &str) -> PsResult<Self> {
        trex::bpe::Bpe::parse(model).map(TrexBpe::of).map_err(|e| arg_err("TrexBpeModel", e))
    }
}

#[psmethods]
impl TrexBpe {
    /// Reads a model from its text, one merge a line with its two symbols
    /// separated by a tab.
    pub fn new(model: String) -> PsResult<Self> {
        TrexBpe::read(&model)
    }

    /// `text` split into the model's subwords, word by word.
    pub fn encode(&self, text: String) -> PsResult<Vec<String>> {
        Ok(self.bpe.encode(text.as_bytes()))
    }
}

/// Learns a byte-pair encoder from a corpus.
///
/// Each word of the corpus is its characters and an end-of-word mark, and
/// every round merges the most frequent adjacent pair of symbols, ties
/// broken by the pair's text, so the same corpus always learns the same
/// model. The files named, or the strings piped in, are read as one corpus.
/// -MaxBytes trains on the start of a large corpus, cut back to a whole
/// word.
///
/// # Examples
/// $bpe = New-TrexBpe -Path ./corpus.txt -Merges 2000
/// New-TrexBpe -Path ./wiki.txt -MaxBytes 10MB | Export-TrexBpe ./wiki.bpe
/// Get-Content ./notes.txt | New-TrexBpe -Merges 200
#[cmdlet(verb = "New", noun = "TrexBpe", alias = "New-TxBpe", default_parameter_set = "Path", output = ["Trex.Bpe"])]
#[derive(Default)]
pub struct NewTrexBpe {
    /// The corpus files; wildcards expand, and every file is read into the
    /// one corpus.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Vec<String>,
    /// The corpus files, read as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The corpus as text; every string piped in is read into the one
    /// corpus.
    #[param(mandatory, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// How many merges to learn: 1000 when absent.
    #[param]
    pub merges: Option<u32>,
    /// Trains on at most this many bytes from the start of the corpus, cut
    /// back to the end of a word.
    #[param]
    pub max_bytes: Option<u64>,
    corpus: Vec<u8>,
}

impl Cmdlet for NewTrexBpe {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let corpus = &mut self.corpus;
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            if !corpus.is_empty() {
                corpus.push(b'\n');
            }
            corpus.extend_from_slice(input.text.as_bytes());
            Ok(())
        })
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let merges = self.merges.unwrap_or(1000) as usize;
        let sample = match self.max_bytes {
            Some(cap) => trex::bpe::sample(&self.corpus, cap as usize),
            None => &self.corpus[..],
        };
        let began = std::time::Instant::now();
        let bpe = trex::bpe::Bpe::train(sample, merges);
        ps.verbose(&format!(
            "learned {} merges from {} bytes in {:.1} s",
            bpe.len(),
            sample.len(),
            began.elapsed().as_secs_f64()
        ))?;
        ps.write(TrexBpe::of(bpe))
    }
}

/// Reads a byte-pair encoder from a model file, one Export-TrexBpe or the
/// trex command's `bpe train` wrote.
///
/// A model line is two symbols separated by a tab, and a line that is not
/// is an error naming its number rather than a merge left out.
///
/// # Examples
/// $bpe = Import-TrexBpe ./wiki.bpe
#[cmdlet(verb = "Import", noun = "TrexBpe", alias = "Import-TxBpe", output = ["Trex.Bpe"])]
#[derive(Default)]
pub struct ImportTrexBpe {
    /// The model files; wildcards expand.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub path: Vec<String>,
}

impl Cmdlet for ImportTrexBpe {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        for given in &self.path {
            for file in ps.resolve_path(given, false)? {
                let model = match std::fs::read_to_string(&file) {
                    Ok(model) => model,
                    Err(e) => {
                        ps.write_error(&read_err(&file, e))?;
                        continue;
                    }
                };
                match trex::bpe::Bpe::parse(&model) {
                    Ok(bpe) => ps.write(TrexBpe::of(bpe))?,
                    Err(e) => ps.write_error(&arg_err("TrexBpeModel", format!("{file}: {e}")))?,
                }
            }
        }
        Ok(())
    }
}

/// Writes a byte-pair encoder's model to a file, one merge a line with its
/// two symbols separated by a tab, which Import-TrexBpe and the trex
/// command's `bpe encode --model` read.
///
/// # Examples
/// New-TrexBpe -Path ./corpus.txt | Export-TrexBpe ./corpus.bpe
#[cmdlet(verb = "Export", noun = "TrexBpe", alias = "Export-TxBpe", supports_should_process)]
#[derive(Default)]
pub struct ExportTrexBpe {
    /// The file to write.
    #[param(mandatory, position = 0)]
    pub path: String,
    /// The encoder whose model to write.
    #[param(mandatory, value_from_pipeline)]
    pub bpe: Option<PsProxy<TrexBpe>>,
}

impl Cmdlet for ExportTrexBpe {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(bpe) = &self.bpe else {
            return Err(arg_err("TrexNoBpe", "an encoder is required"));
        };
        let mut resolved = ps.resolve_path(&self.path, true)?;
        let Some(file) = resolved.pop() else {
            return Err(arg_err("TrexBpePath", format!("{} names no file", self.path)));
        };
        let model = bpe.with(|b| b.model.clone())?;
        if !ps.should_process(&file, "Write the byte-pair model")? {
            return Ok(());
        }
        std::fs::write(&file, model)
            .map_err(|e| PsError::new(ErrorCategory::WriteError, "TrexBpeWrite", format!("{file}: {e}")))
    }
}

/// Splits text into a byte-pair encoder's subwords, writing each subword.
///
/// Each word is split on its own, the last subword of a word ending in
/// `</w>`, so joining a word's subwords and dropping the mark gives the word
/// back.
///
/// # Examples
/// ConvertTo-TrexBpe -Bpe $bpe -InputObject 'the lowest newer'
/// Get-Content ./notes.txt | ConvertTo-TrexBpe -Bpe (Import-TrexBpe ./notes.bpe)
#[cmdlet(verb = "ConvertTo", noun = "TrexBpe", alias = "ConvertTo-TxBpe", default_parameter_set = "Text", output = ["System.String"])]
#[derive(Default)]
pub struct ConvertToTrexBpe {
    /// The encoder: a Trex.Bpe from New-TrexBpe or Import-TrexBpe.
    #[param(mandatory, position = 0)]
    pub bpe: Option<PsProxy<TrexBpe>>,
    /// The text to split; each string piped in is split on its own.
    #[param(mandatory, position = 1, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files to split, each whole; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files to split, read as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    model: Option<trex::bpe::Bpe>,
}

impl Cmdlet for ConvertToTrexBpe {
    fn begin(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(bpe) = &self.bpe else {
            return Err(arg_err("TrexNoBpe", "an encoder is required"));
        };
        self.model = Some(bpe.with(|b| b.bpe.clone())?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(model) = &self.model else {
            return Ok(());
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            for subword in model.encode(input.text.as_bytes()) {
                ps.write(subword)?;
            }
            Ok(())
        })
    }
}
