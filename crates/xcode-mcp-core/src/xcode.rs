use std::path::Path;
use tokio::process::Command;

pub fn build_list_schemes_command(project_or_workspace: &Path) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcodebuild").arg("-list");
    if project_or_workspace
        .to_string_lossy()
        .ends_with(".xcworkspace")
    {
        cmd.arg("-workspace").arg(project_or_workspace);
    } else {
        cmd.arg("-project").arg(project_or_workspace);
    }
    cmd
}

#[allow(clippy::too_many_arguments)]
pub fn build_xcodebuild_command(
    project_or_workspace: &Path,
    scheme: &str,
    action: &str,
    configuration: Option<&str>,
    destination: Option<&str>,
    xcresult_path: &Path,
    derived_data_path: &Path,
) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcodebuild").arg("-scheme").arg(scheme);
    if project_or_workspace
        .to_string_lossy()
        .ends_with(".xcworkspace")
    {
        cmd.arg("-workspace").arg(project_or_workspace);
    } else {
        cmd.arg("-project").arg(project_or_workspace);
    }
    if let Some(cfg) = configuration {
        cmd.arg("-configuration").arg(cfg);
    }
    if let Some(dest) = destination {
        cmd.arg("-destination").arg(dest);
    }
    cmd.arg("-resultBundlePath")
        .arg(xcresult_path)
        .arg("-derivedDataPath")
        .arg(derived_data_path)
        .arg("-quiet");
    match action {
        "clean+build" => {
            cmd.arg("clean").arg("build");
        }
        "clean" => {
            cmd.arg("clean");
        }
        _ => {
            cmd.arg("build");
        }
    }
    cmd
}

pub fn build_xcresulttool_command(xcresult_path: &Path) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcresulttool")
        .arg("get")
        .arg("build-results")
        .arg("--format")
        .arg("json")
        .arg("--path")
        .arg(xcresult_path);
    cmd
}
