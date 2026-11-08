//! Kind `tree` (spec §19): the tree block, its stats and child documents,
//! and folders given by path.

use diffyman_adapter_tree::{
    self as adapter, Contents, Entry, EntryType as ListedType, Exclude, Issues, Limits, ListError,
    ListOptions, Listing, PairStatus, Source,
};
pub(crate) use diffyman_adapter_tree::{LIBRARY_VERSIONS, TREE_ID, archive_format};
use diffyman_algo::key::KeyBuilder;
use diffyman_model::{
    Adapter, ChildDocument, EntrySide, EntryStatus, EntryType, FileMeta, Kind, Options, Side,
    Stats, Status, Tree, TreeEntry, TreeStats, Warning, WarningCode,
};
use serde_json::Value;

use crate::{Input, empty_document, finish_warnings, has_error, similarity};

/// What a whole top-level diff shares, down through its child documents.
pub(crate) struct Context {
    /// 0 for the top-level document, plus 1 per child level (spec §19.5).
    pub depth: u32,
    /// What is left of each side's archive content budget (spec §19.6).
    budget: [u64; 2],
    /// What is left of `tree.childBudget`.
    child_budget: u64,
    limits: Limits,
    /// The comparison keys of the text options, unless they failed.
    pub key_builder: Option<KeyBuilder>,
    /// Option errors, which apply whatever the kind (spec §9).
    pub option_warnings: Vec<Warning>,
    exclude: Option<Exclude>,
}

impl Context {
    /// Compiles the options once for the whole diff.
    pub fn new(options: &Options) -> Context {
        let limits = Limits::default();
        let mut option_warnings = Vec::new();
        let key_builder = crate::key_builder(options, &mut option_warnings);
        let exclude = match Exclude::new(&options.tree.exclude) {
            Ok(exclude) => exclude,
            Err(e) => {
                if e.invalid {
                    option_warnings.push(Warning::new(WarningCode::InvalidGlob, None));
                }
                if e.limit {
                    option_warnings.push(Warning::new(WarningCode::LimitExceeded, None));
                }
                None
            }
        };
        Context {
            depth: 0,
            budget: [limits.content_budget; 2],
            child_budget: u64::from(options.tree.child_budget),
            limits,
            key_builder,
            option_warnings,
            exclude,
        }
    }

    /// [`Context::new`] with other limits (for tests).
    #[cfg(test)]
    pub fn with_limits(options: &Options, limits: Limits) -> Context {
        Context {
            budget: [limits.content_budget; 2],
            limits,
            ..Context::new(options)
        }
    }
}

/// One side of a tree: its source (`None` when it is not an archive) and
/// its file meta so far.
pub(crate) struct TreeSide<'a> {
    source: Option<Source<'a>>,
    meta: FileMeta,
    folder: bool,
}

impl<'a> TreeSide<'a> {
    fn archive(input: Input<'a>) -> TreeSide<'a> {
        let bytes = input.bytes();
        TreeSide {
            source: archive_format(bytes).map(|_| Source::Archive(bytes)),
            meta: FileMeta {
                name: input.name().map(str::to_owned),
                media_type: Some(
                    diffyman_detect::detect(bytes, input.name())
                        .media_type
                        .to_owned(),
                ),
                size: bytes.len() as u64,
                sha256: diffyman_model::sha256_hex(bytes),
                encoding: None,
                bom: bytes.starts_with(b"\xEF\xBB\xBF"),
                line_terminator: None,
                final_newline: None,
                line_count: None,
            },
            folder: false,
        }
    }

    #[cfg(feature = "fs")]
    fn folder(path: &'a std::path::Path, name: Option<String>) -> TreeSide<'a> {
        TreeSide {
            source: Some(Source::Folder(path)),
            meta: FileMeta {
                name,
                media_type: Some("inode/directory".to_owned()),
                size: 0,
                sha256: manifest_digest(None),
                encoding: None,
                bom: false,
                line_terminator: None,
                final_newline: None,
                line_count: None,
            },
            folder: true,
        }
    }
}

/// Two inputs given as bytes, compared as archives.
pub(crate) fn archive_document(
    left: Input<'_>,
    right: Input<'_>,
    options: &Options,
    ctx: &mut Context,
) -> diffyman_model::DiffDocument {
    tree_document(
        TreeSide::archive(left),
        TreeSide::archive(right),
        options,
        ctx,
    )
}

fn side_of(i: usize) -> Side {
    if i == 0 { Side::Left } else { Side::Right }
}

fn type_name(t: ListedType) -> &'static str {
    match t {
        ListedType::File => "file",
        ListedType::Directory => "directory",
        ListedType::Symlink => "symlink",
        ListedType::Hardlink => "hardlink",
        ListedType::Other => "other",
    }
}

/// The manifest digest of a folder side (spec §6.1): SHA-256 of the
/// canonical JSON of `[[path, type, sha256 or target or null], ...]`.
fn manifest_digest(listing: Option<&Listing>) -> String {
    let rows: Vec<Value> = listing
        .map(|l| {
            l.entries
                .iter()
                .map(|e| {
                    let content = e
                        .sha256
                        .as_ref()
                        .map(adapter::hex)
                        .or_else(|| e.target.clone())
                        .map_or(Value::Null, Value::String);
                    Value::Array(vec![
                        Value::String(e.path.clone()),
                        Value::String(type_name(e.entry_type).to_owned()),
                        content,
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let text = diffyman_model::jcs::canonicalize(&Value::Array(rows)).unwrap_or_default();
    diffyman_model::sha256_hex(text.as_bytes())
}

/// The warnings of one side's listing (spec §10).
fn issue_warnings(issues: Issues, side: Side, warnings: &mut Vec<Warning>) {
    for (flag, code) in [
        (issues.non_utf8_path, WarningCode::NonUtf8Path),
        (issues.unsafe_path, WarningCode::UnsafePath),
        (issues.duplicate_path, WarningCode::DuplicatePath),
        (issues.encrypted, WarningCode::EncryptedEntry),
        (issues.unreadable, WarningCode::EntryUnreadable),
        (issues.limit, WarningCode::TreeLimit),
    ] {
        if flag {
            warnings.push(Warning::new(code, Some(side)));
        }
    }
}

/// Content not read somewhere: the status is `partial` (spec §19.6).
fn partial(issues: Issues) -> bool {
    issues.encrypted || issues.unreadable || issues.limit
}

fn entry_side(entry: &Entry) -> EntrySide {
    let is_file = entry.entry_type == ListedType::File;
    EntrySide {
        entry_type: match entry.entry_type {
            ListedType::File => EntryType::File,
            ListedType::Directory => EntryType::Directory,
            ListedType::Symlink => EntryType::Symlink,
            ListedType::Hardlink => EntryType::Hardlink,
            ListedType::Other => EntryType::Other,
        },
        size: entry.size.filter(|_| is_file),
        sha256: entry.sha256.as_ref().map(adapter::hex),
        target: entry.target.clone(),
    }
}

fn entry_status(status: PairStatus) -> EntryStatus {
    match status {
        PairStatus::Added => EntryStatus::Added,
        PairStatus::Removed => EntryStatus::Removed,
        PairStatus::Modified => EntryStatus::Modified,
        PairStatus::Unchanged => EntryStatus::Unchanged,
        PairStatus::Moved => EntryStatus::Moved,
        PairStatus::TypeChange => EntryStatus::TypeChange,
    }
}

/// Counts the bytes written to it.
struct Counter(u64);

impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Lists both sides, pairs them and builds the tree document (spec §19).
pub(crate) fn tree_document(
    left: TreeSide<'_>,
    right: TreeSide<'_>,
    options: &Options,
    ctx: &mut Context,
) -> diffyman_model::DiffDocument {
    let mut warnings = ctx.option_warnings.clone();
    let sources = [left.source, right.source];
    let folders = [left.folder, right.folder];
    let mut metas = [left.meta, right.meta];
    let mut listings: [Option<Listing>; 2] = [None, None];
    if !has_error(&warnings) {
        let list_options = ListOptions {
            follow_symlinks: options.tree.follow_symlinks,
            ignore_files: options.tree.ignore_files,
            exclude: ctx.exclude.as_ref(),
        };
        for i in 0..2 {
            let Some(source) = &sources[i] else {
                warnings.push(Warning::new(WarningCode::InvalidArchive, Some(side_of(i))));
                continue;
            };
            match adapter::list(source, &list_options, &ctx.limits, &mut ctx.budget[i]) {
                Ok(listing) => listings[i] = Some(listing),
                Err(ListError::Invalid) => {
                    warnings.push(Warning::new(WarningCode::InvalidArchive, Some(side_of(i))));
                }
                Err(ListError::Limit) => {
                    warnings.push(Warning::new(WarningCode::LimitExceeded, None));
                }
            }
        }
    }
    for i in 0..2 {
        if folders[i] {
            metas[i].size = listings[i].as_ref().map_or(0, Listing::total_file_size);
            metas[i].sha256 = manifest_digest(listings[i].as_ref());
        }
    }
    let [left_meta, right_meta] = metas;
    let [Some(l), Some(r)] = listings else {
        let status = finish_warnings(&mut warnings, false);
        let mut doc = empty_document(Kind::Tree, left_meta, right_meta, options);
        doc.status = status;
        doc.warnings = warnings;
        return doc;
    };
    if has_error(&warnings) {
        // Not reached: listing only runs without option errors. Kept so an
        // error can never come with a tree.
        let status = finish_warnings(&mut warnings, false);
        let mut doc = empty_document(Kind::Tree, left_meta, right_meta, options);
        doc.status = status;
        doc.warnings = warnings;
        return doc;
    }
    issue_warnings(l.issues, Side::Left, &mut warnings);
    issue_warnings(r.issues, Side::Right, &mut warnings);
    let pairs = adapter::pair(&l, &r, options.tree.detect_moves);

    let mut stats = TreeStats::default();
    let mut entries = Vec::with_capacity(pairs.len());
    for p in &pairs {
        let left_entry = p.left.map(|i| &l.entries[i]);
        let right_entry = p.right.map(|i| &r.entries[i]);
        stats.entries_left += u64::from(left_entry.is_some());
        stats.entries_right += u64::from(right_entry.is_some());
        match p.status {
            PairStatus::Added => stats.added += 1,
            PairStatus::Removed => stats.removed += 1,
            PairStatus::Modified => stats.modified += 1,
            PairStatus::Unchanged => stats.unchanged += 1,
            PairStatus::Moved => stats.moved += 1,
            PairStatus::TypeChange => stats.type_changed += 1,
        }
        entries.push(TreeEntry {
            path: p.path.clone(),
            from_path: p.from_path.clone(),
            status: entry_status(p.status),
            left: left_entry.map(entry_side),
            right: right_entry.map(entry_side),
            child_document: None,
            child_ref: None,
        });
    }
    let changes = pairs.len() as u64 - stats.unchanged;
    let mut status = finish_warnings(&mut warnings, changes > 0);
    if status != Status::Error && (partial(l.issues) || partial(r.issues)) {
        status = Status::Partial;
    }
    let mut doc = empty_document(Kind::Tree, left_meta, right_meta, options);
    doc.status = status;
    doc.warnings = warnings;
    if status != Status::Error {
        doc.stats = Some(Stats {
            lines_left: 0,
            lines_right: 0,
            lines_added: 0,
            lines_removed: 0,
            lines_unchanged: 0,
            change_count: changes,
            hunk_count: 0,
            similarity: similarity(stats.unchanged, stats.entries_left, stats.entries_right),
            bytes: None,
            tree: Some(stats),
        });
        doc.tree = Some(Tree { entries });
        // A nested tree sets its own size without children aside before its
        // children draw from the budget (spec §19.5).
        let reserve = if ctx.depth == 0 {
            0
        } else {
            skeleton_len(&doc)
        };
        let (children, omitted) = children(&pairs, [&l, &r], &sources, options, reserve, ctx);
        if let Some(tree) = &mut doc.tree {
            for (entry, child) in tree.entries.iter_mut().zip(children) {
                entry.child_document = child;
            }
        }
        if omitted {
            doc.warnings
                .push(Warning::new(WarningCode::ChildrenOmitted, None));
            finish_warnings(&mut doc.warnings, false);
        }
    }
    doc
}

/// The size a tree document without child documents counts for as a child
/// (spec §19.5): its canonical JSON without provenance, with an `id`, and
/// with the `children-omitted` warning.
fn skeleton_len(doc: &diffyman_model::DiffDocument) -> u64 {
    let mut skeleton = doc.clone();
    skeleton.id = format!("sha256:{}", "0".repeat(64));
    skeleton
        .warnings
        .push(Warning::new(WarningCode::ChildrenOmitted, None));
    finish_warnings(&mut skeleton.warnings, false);
    let skeleton = ChildDocument::from(skeleton);
    canonical_len(|c| diffyman_model::jcs::to_canonical_writer(&skeleton, c)).unwrap_or(u64::MAX)
}

/// Child documents of the `modified` file entries (spec §19.5), and whether
/// one was left out. `reserve` bytes of the child budget are set aside for
/// the document that holds them.
fn children(
    pairs: &[adapter::Pair],
    listings: [&Listing; 2],
    sources: &[Option<Source<'_>>; 2],
    options: &Options,
    reserve: u64,
    ctx: &mut Context,
) -> (Vec<Option<Box<ChildDocument>>>, bool) {
    let mut out: Vec<Option<Box<ChildDocument>>> = vec![None; pairs.len()];
    let [Some(left_source), Some(right_source)] = sources else {
        return (out, false);
    };
    if options.tree.child_budget == 0 {
        return (out, false);
    }
    let candidates: Vec<(usize, usize, usize)> = pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.status == PairStatus::Modified)
        .filter_map(|(k, p)| Some((k, p.left?, p.right?)))
        .filter(|(_, li, ri)| {
            [(listings[0], *li), (listings[1], *ri)]
                .iter()
                .all(|(l, i)| {
                    let e = &l.entries[*i];
                    e.entry_type == ListedType::File && e.sha256.is_some()
                })
        })
        .collect();
    if candidates.is_empty() {
        return (out, false);
    }
    if reserve > ctx.child_budget {
        // The holder does not fit even without children: none is computed.
        return (out, true);
    }
    ctx.child_budget -= reserve;
    let wanted_left: Vec<usize> = candidates.iter().map(|c| c.1).collect();
    let wanted_right: Vec<usize> = candidates.iter().map(|c| c.2).collect();
    let mut left_contents = Contents::open(left_source, listings[0], &wanted_left, &ctx.limits);
    let mut right_contents = Contents::open(right_source, listings[1], &wanted_right, &ctx.limits);
    let child_options = Options {
        adapter: match options.adapter {
            Adapter::Tree => Adapter::Auto,
            other => other,
        },
        ..options.clone()
    };
    // Every child document holds its options and three SHA-256 digests (its
    // `id` and both sides), so none is smaller than this. Once less of the
    // budget is left, no further child can fit and none is computed.
    let min_child = canonical_len(|c| diffyman_model::jcs::to_canonical_writer(&child_options, c))
        .map_or(u64::MAX, |n| n.saturating_add(3 * 64));
    let archives = [
        matches!(left_source, Source::Archive(_)),
        matches!(right_source, Source::Archive(_)),
    ];
    let mut omitted = false;
    for (k, li, ri) in candidates {
        if ctx.child_budget < min_child {
            omitted = true;
            continue;
        }
        let (le, re) = (&listings[0].entries[li], &listings[1].entries[ri]);
        // Reading archive content again counts against the side's content
        // budget like the listing did (spec §19.6).
        let sizes = [le.size.unwrap_or(0), re.size.unwrap_or(0)];
        if (0..2).any(|i| archives[i] && sizes[i] > ctx.budget[i]) {
            omitted = true;
            continue;
        }
        for i in 0..2 {
            if archives[i] {
                ctx.budget[i] -= sizes[i];
            }
        }
        let (Some(left_bytes), Some(right_bytes)) = (
            left_contents.get(le, &ctx.limits),
            right_contents.get(re, &ctx.limits),
        ) else {
            omitted = true;
            continue;
        };
        let saved = ctx.child_budget;
        ctx.depth += 1;
        let mut child = crate::document(
            Input::named(&left_bytes, basename(&le.path)),
            Input::named(&right_bytes, basename(&re.path)),
            &child_options,
            ctx,
        );
        ctx.depth -= 1;
        // Nested children drew from the budget; the child's own size,
        // which includes them, is what counts.
        ctx.child_budget = saved;
        drop((left_bytes, right_bytes));
        let Ok(id) = diffyman_model::compute_id(&child) else {
            omitted = true;
            continue;
        };
        child.id = id;
        let child = ChildDocument::from(child);
        match canonical_len(|c| diffyman_model::jcs::to_canonical_writer(&child, c)) {
            Some(size) if size <= saved => {
                ctx.child_budget = saved - size;
                out[k] = Some(Box::new(child));
            }
            _ => omitted = true,
        }
    }
    ctx.child_budget += reserve;
    (out, omitted)
}

/// The length of the canonical JSON that `write` writes.
fn canonical_len(
    write: impl FnOnce(&mut Counter) -> Result<(), diffyman_model::JcsError>,
) -> Option<u64> {
    let mut counter = Counter(0);
    write(&mut counter).ok().map(|()| counter.0)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Diffs two paths (spec §19.1): two files as [`crate::diff_document_with`]
/// does, and a pair with a folder as a tree. The other side of a folder must
/// be a folder or an archive; anything else is an `invalid-archive` error
/// document. A path that cannot be read is [`crate::Error::Io`].
///
/// Feature `fs` (on by default, off for WASM).
#[cfg(feature = "fs")]
pub fn diff_paths(
    left: &std::path::Path,
    right: &std::path::Path,
    options: &Options,
    settings: &crate::Settings,
) -> Result<diffyman_model::DiffDocument, crate::Error> {
    use crate::Error;
    crate::validate_options(options)?;
    let read = |p: &std::path::Path| {
        std::fs::read(p).map_err(|e| Error::Io(format!("cannot read {}: {e}", p.display())))
    };
    let (left_dir, right_dir) = (left.is_dir(), right.is_dir());
    let (left_name, right_name) = (path_name(left), path_name(right));
    if !left_dir && !right_dir {
        let (lb, rb) = (read(left)?, read(right)?);
        return crate::diff_document_with(
            named(&lb, left_name.as_deref()),
            named(&rb, right_name.as_deref()),
            options,
            settings,
        );
    }
    let timer = crate::Timer::start();
    let lb = if left_dir { Vec::new() } else { read(left)? };
    let rb = if right_dir { Vec::new() } else { read(right)? };
    let left_side = if left_dir {
        TreeSide::folder(left, left_name)
    } else {
        TreeSide::archive(named(&lb, left_name.as_deref()))
    };
    let right_side = if right_dir {
        TreeSide::folder(right, right_name)
    } else {
        TreeSide::archive(named(&rb, right_name.as_deref()))
    };
    let mut ctx = Context::new(options);
    let doc = tree_document(left_side, right_side, options, &mut ctx);
    crate::finish_document(doc, options, settings, &timer)
}

#[cfg(feature = "fs")]
fn named<'a>(bytes: &'a [u8], name: Option<&'a str>) -> Input<'a> {
    match name {
        Some(name) => Input::named(bytes, name),
        None => Input::new(bytes),
    }
}

/// The final component of a path, resolving `.` and `..` first.
#[cfg(feature = "fs")]
fn path_name(path: &std::path::Path) -> Option<String> {
    let direct = path.file_name().map(|n| n.to_string_lossy().into_owned());
    direct.or_else(|| {
        std::fs::canonicalize(path)
            .ok()?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    })
}
