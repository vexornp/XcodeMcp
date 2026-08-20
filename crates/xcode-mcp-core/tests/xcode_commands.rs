use std::path::PathBuf;
use xcode_mcp_core::xcode::*;

fn args_of(cmd: tokio::process::Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|s| s.to_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_schemes_command_for_project() {
    let args = args_of(build_list_schemes_command(&PathBuf::from(
        "/tmp/App.xcodeproj",
    )));
    assert!(args.contains(&"xcodebuild".into()));
    assert!(args.contains(&"-list".into()));
    assert!(args.contains(&"-project".into()));
    assert!(args.contains(&"/tmp/App.xcodeproj".into()));
}

#[test]
fn list_schemes_command_for_workspace() {
    let args = args_of(build_list_schemes_command(&PathBuf::from(
        "/tmp/App.xcworkspace",
    )));
    assert!(args.contains(&"-workspace".into()));
    assert!(args.contains(&"/tmp/App.xcworkspace".into()));
}

#[test]
fn build_command_has_required_flags() {
    let cmd = build_xcodebuild_command(
        &PathBuf::from("/tmp/App.xcodeproj"),
        "App",
        "build",
        Some("Debug"),
        Some("generic/platform=iOS"),
        &PathBuf::from("/tmp/result.xcresult"),
    );
    let args = args_of(cmd);
    assert!(args.contains(&"-scheme".into()));
    assert!(args.contains(&"App".into()));
    assert!(args.contains(&"-configuration".into()));
    assert!(args.contains(&"Debug".into()));
    assert!(args.contains(&"-destination".into()));
    assert!(args.contains(&"generic/platform=iOS".into()));
    assert!(args.contains(&"-resultBundlePath".into()));
    // Invariant: server never overrides DerivedData location — inherits
    // Xcode's configured default so IDE build cache is reused.
    assert!(!args.contains(&"-derivedDataPath".into()));
    assert!(args.contains(&"-quiet".into()));
    assert!(args.contains(&"build".into()));
    assert!(args.contains(&"CODE_SIGNING_ALLOWED=NO".into()));
}

#[test]
fn build_command_clean_plus_build_passes_two_actions() {
    let cmd = build_xcodebuild_command(
        &PathBuf::from("/tmp/App.xcodeproj"),
        "App",
        "clean+build",
        None,
        None,
        &PathBuf::from("/tmp/r.xcresult"),
    );
    let args = args_of(cmd);
    assert!(args.contains(&"clean".into()));
    assert!(args.contains(&"build".into()));
    assert!(!args.contains(&"-configuration".into()));
    assert!(!args.contains(&"-destination".into()));
}

#[test]
fn build_command_uses_workspace_for_xcworkspace() {
    let cmd = build_xcodebuild_command(
        &PathBuf::from("/tmp/App.xcworkspace"),
        "App",
        "build",
        None,
        None,
        &PathBuf::from("/tmp/r.xcresult"),
    );
    let args = args_of(cmd);
    assert!(args.contains(&"-workspace".into()));
    assert!(!args.contains(&"-project".into()));
}

#[test]
fn xcresulttool_command_format() {
    let args = args_of(build_xcresulttool_command(&PathBuf::from(
        "/tmp/r.xcresult",
    )));
    assert!(args.contains(&"xcresulttool".into()));
    assert!(args.contains(&"get".into()));
    assert!(args.contains(&"build-results".into()));
    assert!(args.contains(&"--format".into()));
    assert!(args.contains(&"json".into()));
    assert!(args.contains(&"--path".into()));
    assert!(args.contains(&"/tmp/r.xcresult".into()));
}

#[test]
fn no_shell_invocation() {
    let cmd = build_list_schemes_command(&PathBuf::from("/tmp/App.xcodeproj"));
    assert_eq!(cmd.as_std().get_program().to_str().unwrap(), "xcrun");
}

#[test]
fn build_params_accepts_pod_action_fields() {
    use xcode_mcp_core::xcode::BuildParams;
    let params = BuildParams {
        project_or_workspace: "/tmp/App.xcodeproj".into(),
        scheme: "App".into(),
        action: Some("build".into()),
        configuration: None,
        destination: None,
        timeout_secs: None,
        pod_action: Some("install".into()),
        pod_timeout_secs: Some(300),
    };
    assert_eq!(params.pod_action.as_deref(), Some("install"));
    assert_eq!(params.pod_timeout_secs, Some(300));
}
