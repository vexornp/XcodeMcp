use std::fs;
use xcode_mcp_core::scheme::*;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("tests/fixtures/list/{name}")).unwrap()
}

#[test]
fn parses_typical_output() {
    let info = parse_list_output(&fixture("typical.txt")).unwrap();
    assert_eq!(info.schemes, vec!["App", "AppTests"]);
    assert_eq!(info.targets, vec!["App", "AppTests"]);
    assert_eq!(info.configurations, vec!["Debug", "Release"]);
    assert!(info.parse_warnings.is_empty());
}

#[test]
fn handles_no_schemes() {
    let info = parse_list_output(&fixture("no_schemes.txt")).unwrap();
    assert!(info.schemes.is_empty());
    assert_eq!(info.targets, vec!["App"]);
    assert!(!info.parse_warnings.is_empty());
}

#[test]
fn handles_reordered_sections() {
    let info = parse_list_output(&fixture("reordered.txt")).unwrap();
    assert_eq!(info.schemes, vec!["App", "AppTests"]);
    assert_eq!(info.targets, vec!["App", "AppTests"]);
    assert_eq!(info.configurations, vec!["Debug", "Release"]);
}

#[test]
fn unknown_sections_go_to_warnings() {
    let info = parse_list_output(&fixture("extra_sections.txt")).unwrap();
    assert_eq!(info.schemes, vec!["App"]);
    assert!(info
        .parse_warnings
        .iter()
        .any(|w| w.contains("Swift Packages")));
}

#[test]
fn malformed_output_returns_error() {
    assert!(parse_list_output(&fixture("malformed.txt")).is_err());
}

#[test]
fn empty_input_returns_error() {
    assert!(parse_list_output("").is_err());
}

#[test]
fn parses_realistic_xcodebuild_output() {
    let info = parse_list_output(&fixture("xcode_realistic.txt")).unwrap();
    assert_eq!(info.schemes, vec!["MiniApp", "MiniAppBroken"]);
    assert_eq!(info.targets, vec!["MiniApp", "MiniAppBroken"]);
    assert_eq!(info.configurations, vec!["Debug", "Release"]);
}
