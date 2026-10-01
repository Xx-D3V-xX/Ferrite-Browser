// Validates the newly-authored corpus slice in crates/ferrite-eval/tests/corpus/
// (T-111 Priority 4) -- distinct from tests/pilot_corpus/ (the original 10
// reference cases, re-validated but not grown in place, T-208) and
// tests/agentdojo_corpus/ (the 3 AgentDojo-derived cases, T-009). Every file
// here must load through the same public corpus::load_case/load_corpus every
// other case goes through, and match its own declared ground_truth shape.

use std::path::PathBuf;

use ferrite_eval::corpus::{load_case, load_corpus};
use ferrite_ipi::dataset::{Corpus, GroundTruth};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
}

fn all_json_paths(dir: &PathBuf) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn corpus_sixteen_files_present() {
    let dir = corpus_dir();
    assert_eq!(
        all_json_paths(&dir).len(),
        16,
        "expected 16 .json files in {}",
        dir.display()
    );
}

/// Loads each file individually, printing PASS/FAIL per file, and checks
/// each case's `ground_truth` shape is consistent with its `corpus`
/// (`Benign` -> `GroundTruth::None`; `Attack` -> anything else) -- the same
/// self-check the authoring guide's checklist calls for, enforced here
/// rather than left to eyeballing.
#[test]
fn corpus_each_file_validates_and_matches_declared_ground_truth() {
    let dir = corpus_dir();
    let mut failures = Vec::new();

    for path in all_json_paths(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        match load_case(&path) {
            Ok((case, _content)) => {
                let consistent = match (case.corpus, &case.ground_truth) {
                    (Corpus::Benign, GroundTruth::None) => true,
                    (Corpus::Benign, _) => false,
                    (Corpus::Attack, GroundTruth::None) => false,
                    (Corpus::Attack, _) => true,
                };
                if !consistent {
                    failures.push(format!(
                        "{name}: corpus {:?} inconsistent with ground_truth {:?}",
                        case.corpus, case.ground_truth
                    ));
                }
                println!("PASS  {name}  (case_id {})", case.case_id);
            }
            Err(e) => {
                println!("FAIL  {name}  -> {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "\n{} failure(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn corpus_batch_load_no_duplicate_ids() {
    let dir = corpus_dir();
    match load_corpus(&dir) {
        Ok(cases) => assert_eq!(cases.len(), 16),
        Err(errors) => {
            let joined = errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n  ");
            panic!(
                "batch load reported {} error(s):\n  {}",
                errors.len(),
                joined
            );
        }
    }
}

/// Stratum counts must match what's claimed in docs/PROGRESS.md's A11 entry
/// -- a mechanical guard against the claim silently drifting from the data.
#[test]
fn corpus_stratum_counts_match_the_documented_composition() {
    let dir = corpus_dir();
    let cases = load_corpus(&dir).expect("all files must load for this count to mean anything");

    let attack_tier1 = cases
        .iter()
        .filter(|(c, _)| c.corpus == Corpus::Attack && c.tier == ferrite_ipi::dataset::Tier::Tier1)
        .count();
    let attack_tier2 = cases
        .iter()
        .filter(|(c, _)| c.corpus == Corpus::Attack && c.tier == ferrite_ipi::dataset::Tier::Tier2)
        .count();
    let benign = cases
        .iter()
        .filter(|(c, _)| c.corpus == Corpus::Benign)
        .count();

    assert_eq!(attack_tier1, 6, "expected 6 new Tier1 attack cases");
    assert_eq!(attack_tier2, 4, "expected 4 new Tier2 attack cases");
    assert_eq!(benign, 6, "expected 6 new benign cases");
}
