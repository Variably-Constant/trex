//! A tree's index: one small summary a file, the token kinds its lex made,
//! a filter over its words and the ranges its numbers and timestamps span,
//! written at the tree's root so a later scan opens only the files that can
//! match. It is an optimization only: a file the index has not seen, or one
//! that changed since, is scanned, so an index that is stale or absent costs
//! time and never a match.

use pwrs::prelude::*;

use crate::common::arg_err;
use crate::matching::files_of;

/// What a tree's index holds.
#[psclass(name = "Trex.IndexInfo")]
#[derive(Clone, Default)]
pub struct TrexIndexInfo {
    /// The tree the index covers.
    pub root: String,
    /// The index file, at the tree's root.
    pub path: String,
    /// How many files it covers.
    pub files: i64,
}

impl TrexIndexInfo {
    fn of(root: &std::path::Path, index: &trex::index::Index) -> Self {
        TrexIndexInfo {
            root: root.display().to_string(),
            path: root.join(trex::index::INDEX_FILE).display().to_string(),
            files: index.len() as i64,
        }
    }
}

/// The directories `paths` name, each once, refusing a path that is not a
/// directory, since an index covers a tree.
fn trees(ps: &Pipeline<'_>, paths: &[String]) -> PsResult<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    for given in paths {
        for resolved in ps.resolve_path(given, false)? {
            let dir = std::path::PathBuf::from(&resolved);
            if !dir.is_dir() {
                return Err(arg_err("TrexIndexTree", format!("{resolved} is not a directory; an index covers a tree")));
            }
            if !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    Ok(out)
}

/// Writes the index of each tree named at its root, so every later scan of
/// the tree opens only the files that can match.
///
/// The tree is walked as a scan walks it, with .gitignore and .ignore rules
/// applied and hidden files skipped unless asked for. Select-TrexMatch reads
/// the index of each directory it scans without being asked, and -Index
/// writes one while it scans.
///
/// # Examples
/// New-TrexIndex ./logs
/// New-TrexIndex ./src -Include '*.rs'
#[cmdlet(verb = "New", noun = "TrexIndex", alias = "New-TxIndex", supports_should_process, output = ["Trex.IndexInfo"])]
#[derive(Default)]
pub struct NewTrexIndex {
    /// The trees to index; wildcards expand.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub path: Vec<String>,
    /// Indexes hidden files and directories a walk would skip.
    #[param]
    pub hidden: bool,
    /// Indexes files an ignore rule excludes.
    #[param]
    pub no_ignore: bool,
    /// Indexes a walked file only when a glob matches it, or drops it for a
    /// glob that starts with `!`.
    #[param]
    pub include: Vec<String>,
}

impl Cmdlet for NewTrexIndex {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let opts = trex::files::WalkOptions {
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            globs: self.include.clone(),
            ..trex::files::WalkOptions::default()
        };
        opts.check().map_err(|e| arg_err("TrexWalk", e))?;
        for dir in trees(ps, &self.path)? {
            let shown = dir.display().to_string();
            let files: Vec<std::path::PathBuf> = files_of(ps, std::slice::from_ref(&shown), true, &opts)?
                .into_iter()
                .filter_map(|s| match s {
                    trex::files::Source::File(p) => Some(p),
                    trex::files::Source::Stdin => None,
                })
                .collect();
            if !ps.should_process(&dir.join(trex::index::INDEX_FILE).display().to_string(), "Write the index")? {
                continue;
            }
            let index = trex::index::Index::build(&dir, &files);
            index.save(&dir).map_err(|e| {
                PsError::new(
                    ErrorCategory::WriteError,
                    "TrexIndexWrite",
                    format!("{}: {e}", dir.join(trex::index::INDEX_FILE).display()),
                )
            })?;
            ps.write(TrexIndexInfo::of(&dir, &index))?;
        }
        Ok(())
    }
}

/// Writes what the index at each tree's root holds.
///
/// # Examples
/// Get-TrexIndex ./logs
#[cmdlet(verb = "Get", noun = "TrexIndex", alias = "Get-TxIndex", output = ["Trex.IndexInfo"])]
#[derive(Default)]
pub struct GetTrexIndex {
    /// The trees whose index to read; wildcards expand.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub path: Vec<String>,
}

impl Cmdlet for GetTrexIndex {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        for dir in trees(ps, &self.path)? {
            match trex::index::Index::load(&dir) {
                Some(index) => ps.write(TrexIndexInfo::of(&dir, &index))?,
                None => ps.write_error(&arg_err(
                    "TrexNoIndex",
                    format!("{} holds no index this trex wrote", dir.display()),
                ))?,
            }
        }
        Ok(())
    }
}
