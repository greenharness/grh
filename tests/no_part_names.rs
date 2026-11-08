//! grh's install path names no part: what a tool does lives in its manifest.
//! Reads the sources of the generic modules, drops their `#[cfg(test)]`
//! modules and comments, and fails on a string literal that names a part.
//! `claude` may appear only in extension.rs, for an MCP registration.

use std::path::Path;

const PARTS: &[&str] = &[
    "amont",
    "aval",
    "relais",
    "gate",
    "greengate",
    "greenpiste",
    "attest",
];
const HOST: &str = "claude";
const FILES: &[&str] = &[
    "owner.rs",
    "uninstall.rs",
    "install.rs",
    "extension.rs",
    "status.rs",
    "ops.rs",
];

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

#[test]
fn the_generic_modules_name_no_part() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut named = Vec::new();
    for file in FILES {
        let source = std::fs::read_to_string(src.join(file)).unwrap();
        for literal in literals(&without_test_modules(&source)) {
            for word in words(&literal) {
                let banned = PARTS.contains(&word) || (word == HOST && *file != "extension.rs");
                if banned {
                    named.push(format!("{file}: {literal:?} names {word}"));
                }
            }
        }
    }
    assert!(named.is_empty(), "{}", named.join("\n"));
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
