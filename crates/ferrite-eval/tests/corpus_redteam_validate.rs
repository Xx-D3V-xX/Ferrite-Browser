// Validates the generated red-team corpus in tests/corpus_redteam/
// (scripts/gen_redteam_corpus.py, ADR-014): every file loads through the same
// public loader every other case uses, the labels agree with the
// specification the generator claims to derive them from, and the
// generator's two assumptions about the defense (the capability lowering
// table and the rule layer's reading of each task) still hold. If either
// assumption drifts, the labels are wrong and this fails before a number is
// reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ferrite_core::taxonomy::LOWERING;
use ferrite_core::Capability;
use ferrite_eval::corpus::load_corpus;
use ferrite_ipi::dataset::{Corpus, GroundTruth};
use ferrite_ipi::fingerprint::rule_based_must_use;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus_redteam")
}

fn raw_files() -> Vec<(String, serde_json::Value)> {
    let mut out: Vec<_> = std::fs::read_dir(dir())
        .expect("corpus_redteam exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let json = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (name, json)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn every_file_loads_and_the_count_matches_the_manifest() {
    let cases = load_corpus(&dir()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let manifest = std::fs::read_to_string(dir().join("MANIFEST.tsv")).expect("MANIFEST.tsv");
    assert_eq!(
        cases.len(),
        manifest.lines().count(),
        "files on disk vs MANIFEST.tsv"
    );
    assert!(cases.len() >= 900, "the matrix shrank to {}", cases.len());
}

#[test]
fn ground_truth_is_consistent_with_the_corpus_kind() {
    for (case, _) in load_corpus(&dir()).expect("loads") {
        let ok = match (case.corpus, &case.ground_truth) {
            (Corpus::Benign, GroundTruth::None) => true,
            (Corpus::Attack, GroundTruth::None) | (Corpus::Benign, _) => false,
            (Corpus::Attack, _) => true,
        };
        assert!(
            ok,
            "{}: {:?} vs {:?}",
            case.case_id, case.corpus, case.ground_truth
        );
        // Categories 1-4 are in scope; category 5 (within-fingerprint) is not.
        if case.corpus == Corpus::Attack {
            let within = matches!(
                case.ground_truth,
                GroundTruth::WithinFingerprintOriginShift { .. }
                    | GroundTruth::WithinFingerprintDataOnly { .. }
            );
            assert_eq!(case.in_scope, !within, "{}", case.case_id);
        }
    }
}

#[test]
fn the_generators_lowering_table_is_the_real_one() {
    let claimed: BTreeMap<String, BTreeSet<String>> = serde_json::from_str(
        &std::fs::read_to_string(dir().parent().unwrap().join("redteam_lowering.json"))
            .expect("redteam_lowering.json"),
    )
    .expect("json");
    let real: BTreeMap<String, BTreeSet<String>> = LOWERING
        .iter()
        .map(|(cap, prims)| {
            let name = match cap {
                Capability::WebRead => "WebRead",
                Capability::WebNavigate => "WebNavigate",
                Capability::WebInteract => "WebInteract",
                Capability::WebDownload => "WebDownload",
                Capability::ScopedRead => "ScopedRead",
                Capability::ClipboardRead => "ClipboardRead",
                Capability::ClipboardWrite => "ClipboardWrite",
            };
            (
                name.to_string(),
                prims
                    .iter()
                    .map(|p| ferrite_core::Primitive::from(*p).as_str().to_string())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        claimed, real,
        "scripts/gen_redteam_corpus.py's LOWER drifted from ferrite_core::LOWERING"
    );
}

#[test]
fn the_capabilities_each_task_was_labelled_with_are_what_the_rule_layer_derives() {
    let mut checked = 0;
    for (name, json) in raw_files() {
        let Some(gen) = json.get("_gen") else {
            continue;
        };
        let task = json["case"]["user_task"].as_str().expect("task");
        let claimed: BTreeSet<String> = gen["caps"]
            .as_array()
            .expect("caps")
            .iter()
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        let derived: BTreeSet<String> = rule_based_must_use(task)
            .into_iter()
            .map(|c| format!("{c:?}"))
            .collect();
        assert_eq!(
            claimed, derived,
            "{name}: labelled with {claimed:?} but the rule layer derives {derived:?} for {task:?}"
        );
        checked += 1;
    }
    assert!(checked >= 900, "only {checked} files carried a _gen block");
}

#[test]
fn every_axis_of_the_matrix_is_populated() {
    let mut carriers = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    let mut scopes = BTreeSet::new();
    let mut dressings = BTreeSet::new();
    let mut goals = BTreeSet::new();
    let mut tasks = BTreeSet::new();
    for (_, json) in raw_files() {
        let gen = &json["_gen"];
        carriers.insert(gen["carrier"].as_str().unwrap_or("").to_string());
        kinds.insert(gen["kind"].as_str().unwrap_or("").to_string());
        scopes.insert(gen["scope"].as_str().unwrap_or("").to_string());
        if let Some(d) = gen["dressing"].as_str() {
            dressings.insert(d.to_string());
        }
        if let Some(g) = gen["goal"].as_str() {
            goals.insert(g.to_string());
        }
        tasks.insert(gen["task"].as_str().unwrap_or("").to_string());
    }
    assert_eq!(
        carriers.iter().filter(|c| !c.is_empty()).count(),
        11,
        "all 11 carrier vectors: {carriers:?}"
    );
    assert!(
        scopes.is_superset(&["exact", "suffix", "task_open"].map(String::from).into()),
        "{scopes:?}"
    );
    assert!(
        kinds.is_superset(
            &["attack", "benign", "spelling_attack", "spelling_benign"]
                .map(String::from)
                .into()
        ),
        "{kinds:?}"
    );
    assert!(dressings.len() >= 25, "{} dressings", dressings.len());
    assert_eq!(goals.len(), 12, "{goals:?}");
    assert_eq!(
        tasks.len(),
        13 + 1 /* the spelling cases share "shop" */ - 1,
        "{tasks:?}"
    );
}
