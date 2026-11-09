use std::{collections::BTreeSet, fs, path::Path};
fn sources(path: &Path, out: &mut String) {
    if !path.exists() {
        return;
    }
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|s| s == "rs") {
            out.push_str(&fs::read_to_string(path).unwrap());
            out.push('\n');
        }
    }
}
fn definitions(text: &str) -> BTreeSet<String> {
    let mut defined = BTreeSet::new();
    let mut oracle = false;
    for line in text.lines() {
        if line == "## The oracle" {
            oracle = true;
        } else if line.starts_with("## ") {
            oracle = false;
        }
        if oracle
            && let Some((n, _)) = line.split_once(". **")
            && n.parse::<u32>().is_ok()
        {
            defined.insert(format!("ORACLE-{n}"));
        }
        if (line.starts_with("- `")
            || line.starts_with("| `RETRY-UNKNOWN-")
            || (line.starts_with('`') && line.contains("`. ")))
            && let Some(id) = line.split('`').nth(1)
        {
            defined.insert(id.to_owned());
        }
    }
    defined
}
fn citations(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("/// Cases:"))
        .flat_map(|line| line.split(',').map(|id| id.trim().to_owned()))
        .collect()
}
#[test]
fn cited_cases_are_defined_and_uncited_cases_are_reported() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let defined =
        definitions(&fs::read_to_string(root.join("../docs/architecture/testing.md")).unwrap());
    let mut text = String::new();
    sources(&root.join("src"), &mut text);
    sources(&root.join("tests"), &mut text);
    let cited = citations(&text);
    for id in &cited {
        assert!(defined.contains(id), "unknown case {id}");
    }
    for id in defined.difference(&cited) {
        eprintln!("uncited case {id}");
    }
}

#[test]
fn case_discovery_includes_standalone_definitions() {
    super::bounded(|| {
        assert!(
            definitions("`HANDOFF`. Integration evidence must show preservation.")
                .contains("HANDOFF")
        );
        assert!(!definitions("`proptest` is a suitable dev-dependency.").contains("proptest"));
    });
}
