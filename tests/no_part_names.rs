//! grh names no part or tool: what a tool does lives in its manifest. Reads
//! every `.rs` file under `src/`, drops its `#[cfg(test)]` modules and
//! comments, and fails on a string literal that names a part or a tool.
//! `claude` may appear only in the files that talk to the host
//! (`HOST_FILES`).

use std::path::Path;

/// Every part and tool name, in the catalog's names and the old ones, and the
/// things a part's manifest knows about (`husky` and `lefthook` are hook
/// managers a manifest's conflicts name). Compared lowercased.
const PARTS: &[&str] = &[
    "greengate",
    "greenpiste",
    "gate",
    "guardrails",
    "decisions",
    "conform",
    "router",
    "attest",
    "workflow",
    "design",
    "amont",
    "aval",
    "relais",
    "duro",
    "husky",
    "lefthook",
];
const HOST: &str = "claude";
/// The files that talk to the host (Claude Code) and may name it.
const HOST_FILES: &[&str] = &["extension.rs", "skills.rs", "audit.rs"];

/// Every `.rs` file under `dir`, relative to `dir`.
fn sources(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// `source` without its `#[cfg(test)]` modules.
fn without_test_modules(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(open) = after.find('{') else {
            rest = "";
            break;
        };
        let mut depth = 0usize;
        let mut end = after.len();
        for (i, c) in after[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// The text of every string literal outside comments. State carries across
/// newlines, so a literal that spans lines is one literal; raw strings
/// (`r"…"`, `r#"…"#`) end only at their own closing hashes.
fn literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == 'r' && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')) {
            let mut j = i + 1;
            while chars.get(j) == Some(&'#') {
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                let hashes = j - i - 1;
                let mut text = String::new();
                j += 1;
                while j < chars.len() {
                    let closes =
                        chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#'));
                    if closes {
                        j += 1 + hashes;
                        break;
                    }
                    text.push(chars[j]);
                    j += 1;
                }
                found.push(text);
                i = j;
            } else {
                i += 1;
            }
        } else if c == '"' {
            let mut text = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    text.push(' ');
                    i += 1;
                } else {
                    text.push(chars[i]);
                }
                i += 1;
            }
            i += 1;
            found.push(text);
        } else if c == '\'' {
            // a char literal such as '"' or '\'' must not open a string
            i += 1;
            if chars.get(i) == Some(&'\\') {
                i += 1;
            }
            if chars.get(i + 1) == Some(&'\'') {
                i += 2;
            }
        } else {
            i += 1;
        }
    }
    found
}

fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// What a file names: each literal word that is a part or tool name, or the
/// host outside the files allowed to name it.
fn names_in(file: &str, source: &str) -> Vec<String> {
    let mut named = Vec::new();
    for literal in literals(&without_test_modules(source)) {
        for word in words(&literal) {
            let word = word.to_ascii_lowercase();
            let banned =
                PARTS.contains(&word.as_str()) || (word == HOST && !HOST_FILES.contains(&file));
            if banned {
                named.push(format!("{file}: {literal:?} names {word}"));
            }
        }
    }
    named
}

#[test]
fn grh_names_no_part_or_tool() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = sources(&src);
    assert!(
        files.len() >= 15,
        "the scan found only {} sources: {files:?}",
        files.len()
    );
    // The walk must reach the modules that hold the messages, wherever they
    // move to.
    for module in [
        "main.rs",
        "audit.rs",
        "install.rs",
        "catalog.rs",
        "manifest.rs",
    ] {
        assert!(
            files.iter().any(|f| f.ends_with(module)),
            "the scan missed {module}: {files:?}"
        );
    }
    let mut named = Vec::new();
    let mut silent = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(src.join(file)).unwrap();
        // A source with code but no literal at all means the scanner lost
        // its place in it (an unclosed raw string, say), not a clean file.
        if literals(&without_test_modules(&source)).is_empty() && source.contains("fn ") {
            silent.push(file.clone());
        }
        named.extend(names_in(file, &source));
    }
    assert!(silent.is_empty(), "no literal read from {silent:?}");
    assert!(named.is_empty(), "{}", named.join("\n"));
}

#[test]
fn every_host_file_exists() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in HOST_FILES {
        assert!(
            src.join(file).is_file(),
            "{file} is allowed to name the host but does not exist"
        );
    }
}

#[test]
fn a_part_name_in_any_scanned_file_is_caught() {
    let found = names_in("probe.rs", "fn f() { run(\"Aval\"); run(\"claude\"); }");
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(names_in("extension.rs", "fn f() { run(\"claude\"); }").is_empty());
}

#[test]
fn the_check_sees_a_part_name_in_a_literal() {
    let source =
        "fn f() { run(\"amont\"); }\n#[cfg(test)]\nmod tests { fn g() { run(\"aval\"); } }\n";
    let kept = without_test_modules(source);
    let found: Vec<String> = literals(&kept);
    assert_eq!(found, vec!["amont".to_string()]);
}

#[test]
fn the_check_sees_a_part_name_in_a_two_line_literal() {
    let source = "fn f() {\n    run(\"one\n amont two\");\n    run(r#\"raw \" relais\"#);\n}\n";
    let found = literals(source);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(words(&found[0]).contains(&"amont"));
    assert!(words(&found[1]).contains(&"relais"));
}
