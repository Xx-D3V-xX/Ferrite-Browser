//! Loading, filtering, ordering and slicing the cases a live run draws from.
//!
//! Cases come from the same JSON files and the same loader (`crate::corpus`)
//! every other part of the evaluation uses. A case file may carry a sibling
//! `agentdojo` object (written by `scripts/import_agentdojo.py`) that the core
//! loader ignores and this module reads: the suite, which task pair it is, the
//! capabilities the task ideally needs, the strings the attacker planted.
//!
//! # Order is part of the experiment
//!
//! Batches are only reproducible if "the next 25 cases" means the same 25 every
//! time. The ordering is: sort by case id (a UUID string, stable and unrelated to
//! file names), then, if `--seed` is given, shuffle with a seeded generator.
//! Filters apply before ordering, so adding a `--suite` changes which cases exist,
//! never how the survivors are ordered relative to each other.

use std::path::{Path, PathBuf};

use ferrite_ipi::dataset::{Carrier, CaseDefinition, Corpus, GroundTruth};
use ferrite_ipi::dry_run::DryRunContent;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde::Deserialize;

use super::config::{KindFilter, LiveArgs};

/// `(name, directory under tests/)` for every corpus a run may draw from.
pub const CORPORA: &[(&str, &str)] = &[
    ("core", "corpus"),
    ("pilot", "pilot_corpus"),
    ("agentdojo-hand", "agentdojo_corpus"),
    ("agentdojo", "agentdojo_full"),
    ("redteam", "corpus_redteam"),
];

/// The part of an imported AgentDojo case the runner reads.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct AgentDojoMeta {
    /// `workspace`, `travel`, `banking` or `slack`.
    pub suite: String,
    /// `user_task_3`.
    pub user_task_id: String,
    /// `injection_task_1`, absent on a benign twin.
    #[serde(default)]
    pub injection_task_id: Option<String>,
    /// The attack template, absent on a benign twin.
    #[serde(default)]
    pub attack: Option<String>,
    /// The capabilities the user task's own ground-truth calls need (`WebRead`, ...).
    #[serde(default)]
    pub user_capabilities: Vec<String>,
    /// Primitives the injection's ground-truth calls realize.
    #[serde(default)]
    pub injection_primitives: Vec<String>,
    /// Strings the attacker chose (addresses, URLs, account numbers), lower-cased.
    #[serde(default)]
    pub indicators: Vec<String>,
    /// Where the injection's effects came from (`ground_truth_calls` or
    /// `goal_text_heuristic`).
    #[serde(default)]
    pub injection_effects_source: Option<String>,
}

/// A case, ready to run.
#[derive(Debug, Clone)]
pub struct LiveCase {
    /// The authored case.
    pub case: CaseDefinition,
    /// The content its dry-run engine serves.
    pub content: DryRunContent,
    /// The corpus name (`agentdojo`, `redteam`, ...).
    pub source: String,
    /// The suite the report groups by: `agentdojo/workspace`, or the corpus name.
    pub suite: String,
    /// AgentDojo provenance, when the file carries it.
    pub meta: Option<AgentDojoMeta>,
}

impl LiveCase {
    /// `attack` or `benign`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self.case.corpus {
            Corpus::Attack => "attack",
            Corpus::Benign => "benign",
        }
    }

    /// `web_content` or `tool_output`.
    #[must_use]
    pub fn carrier(&self) -> &'static str {
        match self.case.carrier_vector.carrier() {
            Carrier::WebContent => "web_content",
            Carrier::ToolOutput => "tool_output",
        }
    }

    /// `deviation`, `origin_shift`, `residual` or `none`.
    #[must_use]
    pub fn ground_truth_class(&self) -> &'static str {
        match &self.case.ground_truth {
            GroundTruth::Deviation { .. } => "deviation",
            GroundTruth::WithinFingerprintOriginShift { .. } => "origin_shift",
            GroundTruth::WithinFingerprintDataOnly { .. } => "residual",
            GroundTruth::None => "none",
        }
    }
}

/// Every `*.json` file under `dir`, sorted, as cases.
///
/// # Errors
///
/// A message naming the first file that does not load, or the directory that
/// cannot be read.
fn load_dir(dir: &Path, source: &str) -> Result<Vec<LiveCase>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    paths.sort();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let (case, content) =
            crate::corpus::load_case(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let meta = read_meta(&path);
        let suite = match (&meta, source) {
            (Some(m), "agentdojo") => format!("agentdojo/{}", m.suite),
            _ => source.to_string(),
        };
        out.push(LiveCase {
            case,
            content,
            source: source.to_string(),
            suite,
            meta,
        });
    }
    Ok(out)
}

fn read_meta(path: &Path) -> Option<AgentDojoMeta> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    serde_json::from_value(value.get("agentdojo")?.clone()).ok()
}

/// The default corpus root: this crate's `tests/` directory.
#[must_use]
pub fn default_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Loads the named corpora (or all of them for `all`) from `root`.
///
/// # Errors
///
/// An unknown corpus name (listing the known ones), a directory that cannot be
/// read, a file that does not load, or a case id that appears twice across the
/// chosen corpora.
pub fn load(root: &Path, names: &[String]) -> Result<Vec<LiveCase>, String> {
    let wanted: Vec<&(&str, &str)> = if names.iter().any(|n| n == "all") {
        CORPORA.iter().collect()
    } else {
        let mut v = Vec::new();
        for name in names {
            let entry = CORPORA.iter().find(|(n, _)| n == name).ok_or_else(|| {
                format!(
                    "unknown corpus {name:?}; known: {}, all",
                    CORPORA
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            if !v.contains(&entry) {
                v.push(entry);
            }
        }
        v
    };
    let mut cases = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (name, dir) in wanted {
        for case in load_dir(&root.join(dir), name)? {
            if !seen.insert(case.case.case_id) {
                return Err(format!(
                    "case id {} appears in two corpora",
                    case.case.case_id
                ));
            }
            cases.push(case);
        }
    }
    Ok(cases)
}

/// Applies `--suite` and `--only`, then orders (id, then seeded shuffle).
#[must_use]
pub fn select(mut cases: Vec<LiveCase>, args: &LiveArgs) -> Vec<LiveCase> {
    cases.retain(|c| match args.only {
        KindFilter::All => true,
        KindFilter::Attack => c.case.corpus == Corpus::Attack,
        KindFilter::Benign => c.case.corpus == Corpus::Benign,
    });
    if !args.suites.is_empty() {
        cases.retain(|c| args.suites.iter().any(|s| c.suite.starts_with(s.as_str())));
    }
    cases.sort_by_key(|c| c.case.case_id);
    if let Some(seed) = args.seed {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        cases.shuffle(&mut rng);
    }
    cases
}

/// Which window of an ordering a run takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// `--offset`/`--limit`: a fixed window of the ordering.
    Fixed {
        /// Cases skipped.
        offset: usize,
        /// At most this many, or the rest.
        limit: Option<usize>,
    },
    /// `--batch-size`/`--batch-index`: slice K of the ordering, K counted from 0.
    Batch {
        /// The slice length.
        size: usize,
        /// The slice number.
        index: usize,
    },
    /// `--batch-size` alone: the next `size` cases that have no stored result.
    NextPending {
        /// How many.
        size: usize,
    },
}

impl Window {
    /// The window the arguments describe.
    #[must_use]
    pub fn of(args: &LiveArgs) -> Self {
        match (args.batch_size, args.batch_index) {
            (Some(size), Some(index)) => Self::Batch { size, index },
            (Some(size), None) => Self::NextPending { size },
            (None, _) => Self::Fixed {
                offset: args.offset,
                limit: args.limit,
            },
        }
    }

    /// Takes the window from an ordered list. `is_pending` says whether a case still
    /// has work to do; only [`Window::NextPending`] consults it.
    #[must_use]
    pub fn take<T>(self, ordered: Vec<T>, is_pending: impl Fn(&T) -> bool) -> Vec<T> {
        match self {
            Self::Fixed { offset, limit } => ordered
                .into_iter()
                .skip(offset)
                .take(limit.unwrap_or(usize::MAX))
                .collect(),
            Self::Batch { size, index } => ordered
                .into_iter()
                .skip(index.saturating_mul(size))
                .take(size)
                .collect(),
            Self::NextPending { size } => ordered
                .into_iter()
                .filter(|c| is_pending(c))
                .take(size)
                .collect(),
        }
    }
}

/// How many batches of `size` an ordering of `n` cases has.
#[must_use]
pub fn batch_count(n: usize, size: usize) -> usize {
    n.div_ceil(size.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> LiveArgs {
        LiveArgs::default()
    }

    #[test]
    fn every_corpus_directory_exists_and_loads() {
        let root = default_root();
        let cases = load(&root, &["all".to_string()]).expect("every corpus loads");
        assert!(cases.len() > 1900, "{} cases", cases.len());
        let sources: std::collections::BTreeSet<&str> =
            cases.iter().map(|c| c.source.as_str()).collect();
        assert_eq!(sources.len(), CORPORA.len());
    }

    #[test]
    fn an_unknown_corpus_names_the_known_ones() {
        let err = load(&default_root(), &["nope".to_string()]).expect_err("unknown");
        assert!(
            err.contains("agentdojo") && err.contains("redteam"),
            "{err}"
        );
    }

    #[test]
    fn agentdojo_cases_carry_their_suite_and_provenance() {
        let cases = load(&default_root(), &["agentdojo".to_string()]).expect("loads");
        assert_eq!(cases.len(), 1046);
        for c in &cases {
            let meta = c.meta.as_ref().expect("every imported case has provenance");
            assert_eq!(c.suite, format!("agentdojo/{}", meta.suite));
        }
        let attack = cases.iter().find(|c| c.kind() == "attack").unwrap();
        assert!(attack.meta.as_ref().unwrap().injection_task_id.is_some());
        let benign = cases.iter().find(|c| c.kind() == "benign").unwrap();
        assert!(benign.meta.as_ref().unwrap().injection_task_id.is_none());
    }

    #[test]
    fn corpora_without_provenance_are_grouped_by_corpus_name() {
        let cases = load(&default_root(), &["core".to_string()]).expect("loads");
        assert!(cases.iter().all(|c| c.suite == "core" && c.meta.is_none()));
    }

    #[test]
    fn filters_apply_before_ordering_and_ordering_is_by_id() {
        let all = load(&default_root(), &["agentdojo".to_string()]).unwrap();
        let mut a = args();
        a.suites = vec!["agentdojo/slack".to_string()];
        a.only = KindFilter::Attack;
        let picked = select(all, &a);
        assert_eq!(picked.len(), 21 * 5);
        assert!(picked
            .iter()
            .all(|c| c.suite == "agentdojo/slack" && c.kind() == "attack"));
        let ids: Vec<_> = picked.iter().map(|c| c.case.case_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "id order, independent of file names");
    }

    #[test]
    fn a_seed_makes_a_reproducible_shuffle_and_no_seed_means_no_shuffle() {
        let all = load(&default_root(), &["agentdojo".to_string()]).unwrap();
        let ids = |seed: Option<u64>| -> Vec<_> {
            let mut a = args();
            a.seed = seed;
            select(all.clone(), &a)
                .iter()
                .map(|c| c.case.case_id)
                .collect()
        };
        assert_eq!(ids(Some(7)), ids(Some(7)));
        assert_ne!(ids(Some(7)), ids(Some(8)));
        assert_ne!(ids(Some(7)), ids(None));
        assert_eq!(ids(None), ids(None));
        // A shuffle is a permutation: nothing lost, nothing duplicated.
        let mut shuffled = ids(Some(7));
        shuffled.sort();
        let mut plain = ids(None);
        plain.sort();
        assert_eq!(shuffled, plain);
    }

    #[test]
    fn batches_tile_the_ordering_exactly_with_no_overlap_and_a_short_last_batch() {
        let items: Vec<usize> = (0..53).collect();
        let size = 25;
        let mut seen = Vec::new();
        for index in 0..batch_count(items.len(), size) {
            let batch = Window::Batch { size, index }.take(items.clone(), |_| true);
            assert!(batch.len() <= size);
            seen.extend(batch);
        }
        assert_eq!(batch_count(53, 25), 3);
        assert_eq!(seen, items, "three batches cover all 53 cases exactly once");
        assert!(
            Window::Batch { size, index: 3 }
                .take(items, |_| true)
                .is_empty(),
            "past the end is empty, not an error"
        );
    }

    #[test]
    fn next_pending_skips_what_is_done_and_takes_the_next_n() {
        let items: Vec<usize> = (0..10).collect();
        let done = [0usize, 1, 2, 5];
        let next = Window::NextPending { size: 3 }.take(items, |i| !done.contains(i));
        assert_eq!(next, vec![3, 4, 6]);
    }

    #[test]
    fn offset_and_limit_are_a_fixed_window() {
        let items: Vec<usize> = (0..10).collect();
        assert_eq!(
            Window::Fixed {
                offset: 4,
                limit: Some(3)
            }
            .take(items.clone(), |_| true),
            vec![4, 5, 6]
        );
        assert_eq!(
            Window::Fixed {
                offset: 8,
                limit: None
            }
            .take(items, |_| true),
            vec![8, 9]
        );
    }

    #[test]
    fn the_window_follows_the_arguments() {
        let mut a = args();
        assert_eq!(
            Window::of(&a),
            Window::Fixed {
                offset: 0,
                limit: None
            }
        );
        a.batch_size = Some(25);
        assert_eq!(Window::of(&a), Window::NextPending { size: 25 });
        a.batch_index = Some(2);
        assert_eq!(Window::of(&a), Window::Batch { size: 25, index: 2 });
    }
}
