//! The files a scan reads, without the scan: what a walk finds under the
//! filters Select-TrexMatch takes, what each file's text reads as where
//! asked, and the file types the type filters name.

use pwrs::prelude::*;

use crate::atoms::like;
use crate::common::{arg_err, read_err};
use crate::matching::{SortKey, files_of};
use crate::structure::{RegionKind, region_kind};

/// One file a walk found.
#[psclass(name = "Trex.File")]
#[derive(Clone, Default)]
pub struct TrexFile {
    /// The file's path.
    pub path: String,
    /// What the file's text reads as mostly, where -Classify or a texture
    /// filter read it; `$null` where neither did, or where the text holds
    /// no region.
    pub texture: PsObject,
    /// A table's period in tokens; 0 for every other texture.
    pub period: i32,
}

/// One file type the type filters name.
#[psclass(name = "Trex.FileType")]
#[derive(Clone, Default)]
pub struct TrexFileType {
    /// The type's name, as -FileType takes it.
    pub name: String,
    /// The globs a file of the type matches.
    pub globs: Vec<String>,
}

/// Lists the files a scan of the paths would read, without scanning them.
///
/// The walk is Select-TrexMatch's: .gitignore and .ignore rules applied,
/// hidden files skipped unless -Hidden asks for them, and -Include and
/// -FileType keeping what it finds, while a file named outright is listed
/// whatever they say. A file holding a NUL byte is binary and is left out,
/// named or found, as Select-TrexMatch leaves it unread, unless -Binary asks
/// for it. -Texture and -ExcludeTexture read every file listed,
/// named or found, as the trex command's `--files` does, and -Classify
/// reads each file's texture into its Texture without filtering.
///
/// # Examples
/// Get-TrexFile ./src
/// Get-TrexFile ./src -FileType rust -Sort Modified -Descending
/// Get-TrexFile ./data -Classify | Group-Object Texture
/// Get-TrexFile ./exports -Texture Table | Get-Content -TotalCount 3
#[cmdlet(verb = "Get", noun = "TrexFile", alias = "Get-TxFile", default_parameter_set = "Path", output = ["Trex.File"])]
#[derive(Default)]
pub struct GetTrexFile {
    /// Files or directories to walk; wildcards expand. The current
    /// directory where none is named.
    #[param(position = 0, set = "Path", value_from_pipeline)]
    pub path: Vec<String>,
    /// Files or directories to walk, read as written, as Get-ChildItem pipes
    /// them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Lists hidden files and directories a walk would skip.
    #[param]
    pub hidden: bool,
    /// Lists files an ignore rule excludes.
    #[param]
    pub no_ignore: bool,
    /// Lists files that hold a NUL byte, which a scan treats as binary and
    /// leaves unread.
    #[param]
    pub binary: bool,
    /// Keeps a walked file only when a glob matches it (`*.log`), or drops
    /// it for a glob that starts with `!`.
    #[param]
    pub include: Vec<String>,
    /// Keeps a walked file only when it is of one of these types, under
    /// ripgrep's names, which Get-TrexFileType lists.
    #[param]
    pub file_type: Vec<String>,
    /// Drops a walked file of one of these types.
    #[param]
    pub exclude_file_type: Vec<String>,
    /// Keeps a file only when its text reads mostly as one of these.
    #[param]
    pub texture: Vec<RegionKind>,
    /// Drops a file whose text reads mostly as one of these.
    #[param]
    pub exclude_texture: Vec<RegionKind>,
    /// Orders the files: by path, or by the time each was last written,
    /// read or created, oldest first.
    #[param]
    pub sort: Option<SortKey>,
    /// Reverses the order -Sort names.
    #[param]
    pub descending: bool,
    /// Reads what each file's text reads as mostly into its Texture, and a
    /// table's period into its Period.
    #[param]
    pub classify: bool,
}

impl Cmdlet for GetTrexFile {
    fn begin(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if self.descending && self.sort.is_none() {
            return Err(arg_err("TrexSort", "-Descending reverses the order -Sort names; give -Sort"));
        }
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (given, literal) = match (self.literal_path.is_empty(), self.path.is_empty()) {
            (false, _) => (self.literal_path.clone(), true),
            (true, true) => (vec![".".to_string()], false),
            (true, false) => (self.path.clone(), false),
        };
        let opts = trex::files::WalkOptions {
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            globs: self.include.clone(),
            types: self.file_type.clone(),
            types_not: self.exclude_file_type.clone(),
            sort: self.sort.map(|k| trex::files::Sort { key: k.key(), reverse: self.descending }),
        };
        opts.check().map_err(|e| arg_err("TrexWalk", e))?;
        let filters = !self.texture.is_empty() || !self.exclude_texture.is_empty();
        for source in files_of(ps, &given, literal, &opts)? {
            if ps.stopping() {
                break;
            }
            let trex::files::Source::File(path) = source else {
                continue;
            };
            let mut file = TrexFile { path: path.display().to_string(), ..TrexFile::default() };
            if !self.binary {
                match trex::files::is_binary_file(&path) {
                    Ok(false) => {}
                    Ok(true) => continue,
                    Err(e) => {
                        ps.write_error(&read_err(&file.path, e))?;
                        continue;
                    }
                }
            }
            if self.classify || filters {
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        ps.write_error(&read_err(&file.path, e))?;
                        continue;
                    }
                };
                let read = trex::shape::dominant_kind(&trex::encoding::decode(bytes)).map(region_kind);
                let kind = read.map(|(kind, _)| kind);
                if kind.is_some_and(|k| self.exclude_texture.contains(&k)) {
                    continue;
                }
                if !self.texture.is_empty() && !kind.is_some_and(|k| self.texture.contains(&k)) {
                    continue;
                }
                if let Some((kind, period)) = read {
                    file.texture = kind.into_ps()?;
                    file.period = period;
                }
            }
            ps.write(file)?;
        }
        Ok(())
    }
}

/// Lists the file types -FileType and -ExcludeFileType take, each with the
/// globs a file of it matches: ripgrep's types, under ripgrep's names.
///
/// # Examples
/// Get-TrexFileType
/// Get-TrexFileType py*
/// Get-TrexFileType | Where-Object Globs -Contains '*.rs'
#[cmdlet(verb = "Get", noun = "TrexFileType", alias = "Get-TxFileType", output = ["Trex.FileType"])]
#[derive(Default)]
pub struct GetTrexFileType {
    /// Only the types whose name matches; wildcards apply.
    #[param(position = 0)]
    pub name: Option<String>,
}

impl Cmdlet for GetTrexFileType {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        for (name, globs) in trex::files::type_list() {
            if self.name.as_deref().is_none_or(|wanted| like(&name, wanted)) {
                ps.write(TrexFileType { name, globs })?;
            }
        }
        Ok(())
    }
}
