// STAGE 1 PILOT VALIDATION — THROWAWAY. Delete once the pilot corpus is finalized.
// Runs load_corpus on tests/pilot_corpus/ and reports per-file validation results.
// Not a real test suite — a one-shot falsification check for the 10 reference cases.

use std::path::PathBuf;

use ferrite_eval::corpus::{load_case, load_corpus};

/// Resolve tests/pilot_corpus relative to this crate, robust to the test CWD.
fn corpus_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/ferrite-eval at compile time — the reliable anchor.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("pilot_corpus")
}

#[test]
fn pilot_corpus_all_ten_files_present() {
    let dir = corpus_dir();
    let count = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .count();
    assert_eq!(count, 10, "expected 10 .json files in {}", dir.display());
}

/// Loads each file individually and prints a per-file PASS/FAIL line, so a failure
/// names the offending file and error instead of collapsing into one opaque result.
#[test]
fn pilot_corpus_each_file_validates() {
    let dir = corpus_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    paths.sort();

    let mut failures = Vec::new();
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy();
        match load_case(path) {
            Ok((case, _content)) => {
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
        "\n{} of {} pilot files failed validation:\n  {}",
        failures.len(),
        paths.len(),
        failures.join("\n  ")
    );
}

/// Also exercise the batch path (sorted load + duplicate-case_id detection across
/// all 10 at once). Prints the collected errors if any.
#[test]
fn pilot_corpus_batch_load() {
    let dir = corpus_dir();
    match load_corpus(&dir) {
        Ok(cases) => {
            println!("batch load: {} cases loaded clean", cases.len());
            assert_eq!(cases.len(), 10);
        }
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
