use std::path::PathBuf;
use xcode_mcp_core::pod::build_pod_command;

fn args_of(cmd: tokio::process::Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|s| s.to_str().unwrap().to_string())
        .collect()
}

#[test]
fn pod_command_install_has_required_flags() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "install");
    let args = args_of(cmd);
    assert!(args.contains(&"install".into()));
    assert!(args.contains(&"--no-ansi".into()));
}

#[test]
fn pod_command_update_uses_update_action() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "update");
    let args = args_of(cmd);
    assert!(args.contains(&"update".into()));
    assert!(args.contains(&"--no-ansi".into()));
}

#[test]
fn pod_command_no_shell_invocation() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "install");
    assert_eq!(cmd.as_std().get_program().to_str().unwrap(), "pod");
}

#[test]
fn pod_command_no_extra_flags() {
    let cmd = build_pod_command(&PathBuf::from("/tmp/proj"), "install");
    let args = args_of(cmd);
    assert_eq!(args, vec!["install", "--no-ansi"]);
}
