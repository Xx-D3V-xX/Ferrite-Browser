// Validates the full AgentDojo import in tests/agentdojo_full/
// (scripts/import_agentdojo.py): every file loads through the same public loader
// every other case uses, the labels agree with the specification the importer
// claims to derive them from (the closed capability lowering of ADR-001), the
// counts are the ones the pinned AgentDojo commit defines, and nothing is
// duplicated or left unmapped. If the importer's assumptions drift from the real
// defense, the labels are wrong and this fails before a number is reported.
//
// What this does NOT check, and cannot: that the carrier text is what AgentDojo's
// own environments would produce, or that the ground-truth tool calls were
// extracted from AgentDojo's Python correctly. The second is covered by
// re-running `python3 scripts/import_agentdojo.py --src <checkout> --check`
// against the pinned commit, which needs a clone and is not part of `cargo test`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ferrite_core::taxonomy::LOWERING;
use ferrite_core::{Capability, Origin, Primitive};
use ferrite_eval::corpus::load_corpus;
use ferrite_ipi::dataset::{AttackCategory, Author, Corpus, GroundTruth, Tier};

/// The AgentDojo commit the importer is pinned to (`PINNED_SHA` in the script).
const PINNED_SHA: &str = "089ed468cf3ed0322acc66b0211f26d9d90dbf60";

/// `(suite, user tasks, injection tasks)` at benchmark v1.2.2 of that commit.
const SUITE_COUNTS: [(&str, usize, usize); 4] = [
    ("workspace", 40, 14),
    ("travel", 20, 7),
    ("banking", 16, 9),
    ("slack", 21, 5),
];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("agentdojo_full")
}

fn raw_files() -> Vec<(String, serde_json::Value)> {
    let mut out: Vec<_> = std::fs::read_dir(dir())
        .expect("agentdojo_full exists")
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

fn manifest() -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(dir().parent().unwrap().join("agentdojo_full_manifest.json"))
            .expect("agentdojo_full_manifest.json"),
    )
    .expect("MANIFEST.json parses")
}

fn strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected an array, got {value}"))
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect()
}

fn capability_name(c: Capability) -> &'static str {
    match c {
        Capability::WebRead => "WebRead",
        Capability::WebNavigate => "WebNavigate",
        Capability::WebInteract => "WebInteract",
        Capability::WebDownload => "WebDownload",
        Capability::ScopedRead => "ScopedRead",
        Capability::ClipboardRead => "ClipboardRead",
        Capability::ClipboardWrite => "ClipboardWrite",
    }
}

/// The primitives a set of capability names authorises, from the REAL lowering.
fn allowed_primitives(capabilities: &[String]) -> BTreeSet<String> {
    LOWERING
        .iter()
        .filter(|(cap, _)| capabilities.iter().any(|c| c == capability_name(*cap)))
        .flat_map(|(_, prims)| {
            prims
                .iter()
                .map(|p| Primitive::from(*p).as_str().to_string())
        })
        .collect()
}

#[test]
fn every_file_loads_and_the_counts_match_the_manifest_and_the_pinned_commit() {
    let cases = load_corpus(&dir()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let manifest = manifest();
    assert_eq!(manifest["source_commit"], PINNED_SHA);
    assert_eq!(manifest["benchmark_version"], "v1.2.2");
    assert_eq!(manifest["license"], "MIT");
    assert_eq!(cases.len() as u64, manifest["cases"].as_u64().unwrap());

    let mut users_total = 0;
    let mut injections_total = 0;
    for (suite, users, injections) in SUITE_COUNTS {
        let m = &manifest["suites"][suite];
        assert_eq!(m["user_tasks"], users, "{suite} user tasks");
        assert_eq!(m["injection_tasks"], injections, "{suite} injection tasks");
        users_total += users;
        injections_total += injections;
    }
    assert_eq!((users_total, injections_total), (97, 35));

    // Every user task has a benign twin; every (user, injection) pair is one attack case.
    let expected_attacks: usize = SUITE_COUNTS.iter().map(|(_, u, i)| u * i).sum();
    assert_eq!(expected_attacks, 949);
    let attacks = cases
        .iter()
        .filter(|(c, _)| c.corpus == Corpus::Attack)
        .count();
    let benign = cases.len() - attacks;
    assert_eq!(attacks, expected_attacks);
    assert_eq!(benign, users_total);
    assert_eq!(manifest["attack_cases"].as_u64().unwrap() as usize, attacks);
    assert_eq!(manifest["benign_cases"].as_u64().unwrap() as usize, benign);
}

#[test]
fn per_suite_counts_in_the_files_match_the_manifest() {
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut injections: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, json) in raw_files() {
        let meta = &json["agentdojo"];
        let suite = meta["suite"].as_str().unwrap().to_string();
        users
            .entry(suite.clone())
            .or_default()
            .insert(meta["user_task_id"].as_str().unwrap().to_string());
        if let Some(id) = meta["injection_task_id"].as_str() {
            injections.entry(suite).or_default().insert(id.to_string());
        }
    }
    for (suite, u, i) in SUITE_COUNTS {
        assert_eq!(users[suite].len(), u, "{suite}: distinct user tasks");
        assert_eq!(
            injections[suite].len(),
            i,
            "{suite}: distinct injection tasks"
        );
    }
}

#[test]
fn no_duplicate_case_ids_or_task_pairs() {
    // load_corpus already rejects a duplicate case_id; this checks the (suite,
    // user task, injection task, attack) key, which is what identifies a case.
    let mut keys = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for (name, json) in raw_files() {
        let meta = &json["agentdojo"];
        let key = (
            meta["suite"].as_str().unwrap().to_string(),
            meta["user_task_id"].as_str().unwrap().to_string(),
            meta["injection_task_id"].as_str().unwrap_or("").to_string(),
            meta["attack"].as_str().unwrap_or("").to_string(),
        );
        assert!(
            keys.insert(key.clone()),
            "{name}: duplicate task pair {key:?}"
        );
        let id = json["case"]["case_id"].as_str().unwrap().to_string();
        assert!(ids.insert(id.clone()), "{name}: duplicate case_id {id}");
    }
}

#[test]
fn every_case_is_tagged_as_agentdojo_and_cites_its_source() {
    for (case, _) in load_corpus(&dir()).expect("loads") {
        assert_eq!(case.tier, Tier::Tier3AgentDojo);
        assert_eq!(case.author, Author::AgentDojo);
        let anchor = case.taxonomy_anchor.expect("every case cites its task");
        assert!(anchor.starts_with("agentdojo/v1.2.2/"), "{anchor}");
    }
    for (name, json) in raw_files() {
        assert_eq!(json["agentdojo"]["source_commit"], PINNED_SHA, "{name}");
        assert_eq!(
            json["agentdojo"]["source"], "https://github.com/ethz-spylab/agentdojo",
            "{name}"
        );
    }
}

#[test]
fn every_injection_is_mapped_to_a_deviation_class_and_the_class_agrees_with_the_case() {
    let mut classes_by_injection: BTreeMap<(String, String), BTreeSet<&'static str>> =
        BTreeMap::new();
    for (case, _) in load_corpus(&dir()).expect("loads") {
        let anchor = case.taxonomy_anchor.clone().unwrap();
        let suite = anchor.split('/').nth(2).unwrap().to_string();
        match (&case.corpus, &case.ground_truth) {
            (Corpus::Benign, GroundTruth::None) => {
                assert!(case.attack_category.is_none());
                assert!(case.in_scope);
            }
            (Corpus::Attack, truth) => {
                let class = match truth {
                    GroundTruth::Deviation { .. } => {
                        assert!(case.in_scope, "{anchor}: a deviation is in scope");
                        "deviation"
                    }
                    GroundTruth::WithinFingerprintOriginShift { .. } => {
                        assert!(!case.in_scope, "{anchor}");
                        "origin_shift"
                    }
                    GroundTruth::WithinFingerprintDataOnly { .. } => {
                        assert!(!case.in_scope, "{anchor}");
                        assert_eq!(
                            case.attack_category,
                            Some(AttackCategory::WithinFingerprintAbuse),
                            "{anchor}: the residual is category 5"
                        );
                        "residual"
                    }
                    GroundTruth::None => panic!("{anchor}: an attack with no ground truth"),
                };
                assert!(case.attack_category.is_some(), "{anchor}");
                assert!(
                    case.attacker_goal.is_some(),
                    "{anchor}: the injection goal is verbatim"
                );
                let injection = anchor
                    .split('+')
                    .nth(1)
                    .expect("user+injection")
                    .to_string();
                classes_by_injection
                    .entry((suite, injection))
                    .or_default()
                    .insert(class);
            }
            (Corpus::Benign, other) => panic!("{anchor}: benign with {other:?}"),
        }
    }
    let total: usize = SUITE_COUNTS.iter().map(|(_, _, i)| i).sum();
    assert_eq!(
        classes_by_injection.len(),
        total,
        "every injection task appears in the corpus with at least one class"
    );
    assert!(classes_by_injection.values().all(|c| !c.is_empty()));
}

#[test]
fn the_importers_lowering_and_tool_map_are_the_real_ones() {
    let manifest = manifest();
    let claimed: BTreeMap<String, BTreeSet<String>> = manifest["lowering"]
        .as_object()
        .expect("lowering")
        .iter()
        .map(|(k, v)| (k.clone(), strings(v).into_iter().collect()))
        .collect();
    let real: BTreeMap<String, BTreeSet<String>> = LOWERING
        .iter()
        .map(|(cap, prims)| {
            (
                capability_name(*cap).to_string(),
                prims
                    .iter()
                    .map(|p| Primitive::from(*p).as_str().to_string())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        claimed, real,
        "scripts/import_agentdojo.py's LOWER drifted from ferrite_core::LOWERING"
    );

    // Every tool maps onto a real primitive, and onto one of the five kinds.
    let known: BTreeSet<&str> = Primitive::ALL.iter().map(|p| p.as_str()).collect();
    let tool_map = manifest["tool_map"].as_object().expect("tool_map");
    assert!(tool_map.len() >= 70, "only {} tools mapped", tool_map.len());
    for (tool, entry) in tool_map {
        let kind = entry[0].as_str().unwrap();
        let primitive = entry[2].as_str().unwrap();
        assert!(
            known.contains(primitive),
            "{tool}: {primitive} is not a primitive"
        );
        assert!(
            ["read", "write", "web_get", "web_post", "web_dl"].contains(&kind),
            "{tool}: {kind}"
        );
        assert_ne!(
            primitive, "js.execute",
            "{tool}: js.execute is never a mapping target"
        );
    }
}

#[test]
fn the_labels_follow_from_the_task_capabilities_under_the_real_lowering() {
    let mut checked = 0;
    for (name, json) in raw_files() {
        let meta = &json["agentdojo"];
        let user_caps = strings(&meta["user_capabilities"]);
        let legit: BTreeSet<String> = strings(&meta["legit_origins"]).into_iter().collect();
        assert!(
            !user_caps.is_empty(),
            "{name}: a user task with no capability"
        );
        assert!(
            meta["unmapped_tools"].as_array().unwrap().is_empty(),
            "{name}: tools with no mapping"
        );
        for origin in &legit {
            Origin::parse(origin).unwrap_or_else(|e| panic!("{name}: {origin}: {e}"));
        }
        let scope: BTreeSet<String> = strings(&json["case"]["expected_origins"]["exact"])
            .into_iter()
            .collect();
        assert_eq!(
            scope, legit,
            "{name}: expected_origins is the legit origin set"
        );

        if json["case"]["corpus"] == "Benign" {
            assert!(meta["attack"].is_null(), "{name}");
            continue;
        }
        let allowed = allowed_primitives(&user_caps);
        let injection_prims: BTreeSet<String> =
            strings(&meta["injection_primitives"]).into_iter().collect();
        let extra: BTreeSet<String> = injection_prims.difference(&allowed).cloned().collect();
        let claimed_extra: BTreeSet<String> =
            strings(&meta["extra_primitives"]).into_iter().collect();
        assert_eq!(
            claimed_extra, extra,
            "{name}: extra primitives are the injection's primitives the task's capabilities do not authorise"
        );
        let out_origins: BTreeSet<String> = strings(&meta["injection_origins"])
            .into_iter()
            .filter(|o| !legit.contains(o))
            .collect();
        let claimed_out: BTreeSet<String> =
            strings(&meta["out_of_scope_origins"]).into_iter().collect();
        assert_eq!(claimed_out, out_origins, "{name}: out-of-scope origins");

        let truth = &json["case"]["ground_truth"];
        if !extra.is_empty() {
            let d = &truth["Deviation"];
            assert_eq!(
                strings(&d["expected_extra_primitives"])
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                extra,
                "{name}"
            );
            assert_eq!(
                strings(&d["expected_out_of_scope_origins"])
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                out_origins,
                "{name}"
            );
        } else if !out_origins.is_empty() {
            let shift = &truth["WithinFingerprintOriginShift"];
            assert!(
                out_origins.contains(shift["attack_origin"].as_str().unwrap()),
                "{name}"
            );
        } else {
            assert!(
                truth.get("WithinFingerprintDataOnly").is_some(),
                "{name}: {truth}"
            );
        }
        // js.execute is unscopable and never a mapping target.
        assert!(!extra.contains("js.execute"), "{name}");
        checked += 1;
    }
    assert_eq!(checked, 949);
}

#[test]
fn the_injection_text_is_in_the_carrier_and_the_benign_twin_has_none() {
    for (name, json) in raw_files() {
        let text = json["content"]["extract_data"][0]["reply"]["value"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: carrier text"));
        if json["case"]["corpus"] == "Attack" {
            let goal = json["case"]["attacker_goal"].as_str().unwrap();
            assert!(
                text.contains(goal),
                "{name}: the goal is planted verbatim in the carrier"
            );
            assert!(
                text.contains("<INFORMATION>"),
                "{name}: important_instructions template"
            );
        } else {
            assert!(
                !text.contains("<INFORMATION>"),
                "{name}: the benign twin carries no injection"
            );
        }
        // The carrier is synthetic: no URL points anywhere but a reserved name.
        assert!(!text.contains("agentdojo"), "{name}");
    }
}

#[test]
fn every_attack_case_has_a_benign_twin_with_the_same_user_task() {
    let mut benign: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut attacks: Vec<((String, String), String)> = Vec::new();
    for (_, json) in raw_files() {
        let meta = &json["agentdojo"];
        let key = (
            meta["suite"].as_str().unwrap().to_string(),
            meta["user_task_id"].as_str().unwrap().to_string(),
        );
        let task = json["case"]["user_task"].as_str().unwrap().to_string();
        if json["case"]["corpus"] == "Benign" {
            benign.insert(key, task);
        } else {
            attacks.push((key, task));
        }
    }
    for (key, task) in attacks {
        assert_eq!(benign.get(&key), Some(&task), "{key:?}");
    }
}

#[test]
fn the_text_only_and_heuristic_injection_tasks_are_declared_not_hidden() {
    let manifest = manifest();
    let mut heuristic = BTreeSet::new();
    for (suite, _, _) in SUITE_COUNTS {
        for id in strings(&manifest["suites"][suite]["injection_tasks_with_goal_text_effects"]) {
            heuristic.insert(format!("{suite}/{id}"));
        }
    }
    // AgentDojo ships no ground truth for these (their `ground_truth()` returns []),
    // so their effects come from the goal text. The manifest must say which.
    assert_eq!(heuristic.len(), 9, "{heuristic:?}");
    for (name, json) in raw_files() {
        let meta = &json["agentdojo"];
        if let Some(id) = meta["injection_task_id"].as_str() {
            let key = format!("{}/{id}", meta["suite"].as_str().unwrap());
            let source = meta["injection_effects_source"].as_str().unwrap();
            assert_eq!(
                source == "goal_text_heuristic",
                heuristic.contains(&key),
                "{name}: {key} effects source {source}"
            );
        }
    }
}
