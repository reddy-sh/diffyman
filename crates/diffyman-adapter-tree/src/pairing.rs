//! Pairing two listings (spec §19.4).

use std::collections::{HashMap, VecDeque};

use crate::path::basename;
use crate::{Entry, EntryType, Listing};

/// Status of a pair (spec §6.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairStatus {
    /// Only on the right.
    Added,
    /// Only on the left.
    Removed,
    /// Same type, different content.
    Modified,
    /// Same type, equal content.
    Unchanged,
    /// A left-only and a right-only file with equal content.
    Moved,
    /// Different types.
    TypeChange,
}

/// One entry of the tree block: a path and the listing entries on each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The path (the right path for `moved`).
    pub path: String,
    /// The left path of a `moved` pair.
    pub from_path: Option<String>,
    /// What happened.
    pub status: PairStatus,
    /// Index into the left listing's entries.
    pub left: Option<usize>,
    /// Index into the right listing's entries.
    pub right: Option<usize>,
}

/// Compares two entries at the same path (spec §19.4 rule 1).
fn compare(l: &Entry, r: &Entry) -> PairStatus {
    if l.entry_type != r.entry_type {
        return PairStatus::TypeChange;
    }
    let same = match l.entry_type {
        EntryType::File => match (&l.sha256, &r.sha256) {
            (Some(a), Some(b)) => a == b,
            (None, None) => l.stored.is_some() && l.stored == r.stored,
            _ => false,
        },
        EntryType::Symlink | EntryType::Hardlink => l.target.is_some() && l.target == r.target,
        EntryType::Directory | EntryType::Other => true,
    };
    if same {
        PairStatus::Unchanged
    } else {
        PairStatus::Modified
    }
}

/// Pairs two listings, sorted by path (spec §19.4).
pub fn pair(left: &Listing, right: &Listing, detect_moves: bool) -> Vec<Pair> {
    use std::cmp::Ordering;
    let (lefts, rights) = (&left.entries, &right.entries);
    let mut pairs = Vec::with_capacity(lefts.len().max(rights.len()));
    let (mut li, mut ri) = (0, 0);
    while li < lefts.len() || ri < rights.len() {
        let order = match (lefts.get(li), rights.get(ri)) {
            (Some(le), Some(re)) => le.path.as_bytes().cmp(re.path.as_bytes()),
            (Some(_), None) => Ordering::Less,
            _ => Ordering::Greater,
        };
        let (status, path, left_index, right_index) = match order {
            Ordering::Less => (PairStatus::Removed, &lefts[li].path, Some(li), None),
            Ordering::Greater => (PairStatus::Added, &rights[ri].path, None, Some(ri)),
            Ordering::Equal => (
                compare(&lefts[li], &rights[ri]),
                &lefts[li].path,
                Some(li),
                Some(ri),
            ),
        };
        pairs.push(Pair {
            path: path.clone(),
            from_path: None,
            status,
            left: left_index,
            right: right_index,
        });
        li += usize::from(left_index.is_some());
        ri += usize::from(right_index.is_some());
    }
    if detect_moves {
        pair_moves(&mut pairs, left, right);
    }
    pairs
}

/// The SHA-256 of a file that may take part in a move.
fn move_key(entry: &Entry) -> Option<[u8; 32]> {
    (entry.entry_type == EntryType::File && entry.size.is_some_and(|s| s > 0))
        .then_some(entry.sha256)
        .flatten()
}

/// Turns removed and added files with equal content into `moved` pairs
/// (spec §19.4 rule 3): same last component first, then path order.
fn pair_moves(pairs: &mut Vec<Pair>, left: &Listing, right: &Listing) {
    // Added candidates per content, in path order, also by last component.
    struct Group<'p> {
        all: Vec<usize>,
        next: usize,
        by_name: HashMap<&'p str, VecDeque<usize>>,
    }
    let mut added: HashMap<[u8; 32], Group<'_>> = HashMap::new();
    for (k, p) in pairs.iter().enumerate() {
        if p.status != PairStatus::Added {
            continue;
        }
        let Some(key) = p.right.and_then(|j| move_key(&right.entries[j])) else {
            continue;
        };
        let group = added.entry(key).or_insert_with(|| Group {
            all: Vec::new(),
            next: 0,
            by_name: HashMap::new(),
        });
        group.all.push(k);
        group
            .by_name
            .entry(basename(&p.path))
            .or_default()
            .push_back(k);
    }
    if added.is_empty() {
        return;
    }
    let removed: Vec<(usize, [u8; 32])> = pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.status == PairStatus::Removed)
        .filter_map(|(k, p)| {
            p.left
                .and_then(|i| move_key(&left.entries[i]))
                .map(|key| (k, key))
        })
        .collect();
    let mut taken = vec![false; pairs.len()];
    let mut matches: Vec<(usize, usize)> = Vec::new();
    let mut unmatched = Vec::new();
    // Pass 1: the first unpaired added file with the same last component.
    for &(k, key) in &removed {
        let found = added.get_mut(&key).and_then(|group| {
            let queue = group.by_name.get_mut(basename(&pairs[k].path))?;
            while let Some(a) = queue.pop_front() {
                if !taken[a] {
                    return Some(a);
                }
            }
            None
        });
        match found {
            Some(a) => {
                taken[a] = true;
                matches.push((k, a));
            }
            None => unmatched.push((k, key)),
        }
    }
    // Pass 2: the remaining removed files take the remaining added files.
    for (k, key) in unmatched {
        let Some(group) = added.get_mut(&key) else {
            continue;
        };
        while group.next < group.all.len() && taken[group.all[group.next]] {
            group.next += 1;
        }
        if let Some(&a) = group.all.get(group.next) {
            taken[a] = true;
            matches.push((k, a));
        }
    }
    drop(added);
    let mut drop_pair = vec![false; pairs.len()];
    for (k, a) in matches {
        let from = pairs[k].path.clone();
        let left_index = pairs[k].left;
        let moved = &mut pairs[a];
        moved.status = PairStatus::Moved;
        moved.from_path = Some(from);
        moved.left = left_index;
        drop_pair[k] = true;
    }
    let mut index = 0;
    pairs.retain(|_| {
        let keep = !drop_pair[index];
        index += 1;
        keep
    });
}
