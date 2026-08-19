use std::fs;
use tempfile::tempdir;
use xcode_mcp_core::security::*;

fn make_root() -> std::path::PathBuf {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::mem::forget(dir);
    root
}

#[test]
fn accepts_xcodeproj_under_root() {
    let root = make_root();
    let proj = root.join("App.xcodeproj");
    fs::create_dir(&proj).unwrap();
    let validated = validate_project_or_workspace(proj.to_str().unwrap(), &root).unwrap();
    assert_eq!(validated, proj.canonicalize().unwrap());
}

#[test]
fn accepts_xcworkspace_under_root() {
    let root = make_root();
    let ws = root.join("App.xcworkspace");
    fs::create_dir(&ws).unwrap();
    let validated = validate_project_or_workspace(ws.to_str().unwrap(), &root).unwrap();
    assert_eq!(validated, ws.canonicalize().unwrap());
}

#[test]
fn rejects_path_outside_root() {
    let root = make_root();
    let outside = tempdir().unwrap();
    let proj = outside.path().join("Evil.xcodeproj");
    fs::create_dir(&proj).unwrap();
    assert!(validate_project_or_workspace(proj.to_str().unwrap(), &root).is_err());
}

#[test]
fn rejects_path_traversal() {
    let root = make_root();
    let evil = format!("{}/../../etc/passwd.xcodeproj", root.display());
    assert!(validate_project_or_workspace(&evil, &root).is_err());
}

#[test]
fn rejects_wrong_extension() {
    let root = make_root();
    let fake = root.join("App.txt");
    fs::write(&fake, "not a project").unwrap();
    assert!(validate_project_or_workspace(fake.to_str().unwrap(), &root).is_err());
}

#[test]
fn rejects_nonexistent_path() {
    let root = make_root();
    assert!(
        validate_project_or_workspace(root.join("Nope.xcodeproj").to_str().unwrap(), &root)
            .is_err()
    );
}

#[test]
fn scheme_accepts_normal_name() {
    assert_eq!(validate_scheme("App").unwrap(), "App");
    assert_eq!(validate_scheme("My-App 2.0").unwrap(), "My-App 2.0");
}

#[test]
fn scheme_rejects_shell_metachars() {
    assert!(validate_scheme("App; rm -rf /").is_err());
    assert!(validate_scheme("App && evil").is_err());
    assert!(validate_scheme("App`whoami`").is_err());
    assert!(validate_scheme("App\nnewline").is_err());
}

#[test]
fn scheme_rejects_too_long() {
    assert!(validate_scheme(&"A".repeat(129)).is_err());
    assert!(validate_scheme(&"A".repeat(128)).is_ok());
}

#[test]
fn configuration_validates() {
    assert!(validate_configuration("Debug").is_ok());
    assert!(validate_configuration("Release").is_ok());
    assert!(validate_configuration("debug").is_err());
    assert!(validate_configuration("Profile").is_err());
}

#[test]
fn action_validates() {
    assert!(validate_action("build").is_ok());
    assert!(validate_action("clean").is_ok());
    assert!(validate_action("clean+build").is_ok());
    assert!(validate_action("test").is_err());
}

#[test]
fn destination_accepts_known_formats() {
    assert!(validate_destination("generic/platform=iOS").is_ok());
    assert!(validate_destination("platform=macOS").is_ok());
    assert!(validate_destination("id=ABCD-1234").is_ok());
}

#[test]
fn destination_rejects_metachars() {
    assert!(validate_destination("generic/platform=iOS; rm -rf /").is_err());
    assert!(validate_destination("$(whoami)").is_err());
}

#[test]
fn timeout_defaults_and_validates() {
    assert_eq!(validate_timeout(None).unwrap(), 1800);
    assert_eq!(validate_timeout(Some(60)).unwrap(), 60);
    assert_eq!(validate_timeout(Some(7200)).unwrap(), 7200);
    assert!(validate_timeout(Some(0)).is_err());
    assert!(validate_timeout(Some(7201)).is_err());
}

#[test]
fn build_id_accepts_uuid() {
    assert!(validate_build_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
}

#[test]
fn build_id_rejects_slashes() {
    assert!(validate_build_id("../../etc/passwd").is_err());
    assert!(validate_build_id("foo/bar").is_err());
}
