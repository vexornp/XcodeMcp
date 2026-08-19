use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

const SCHEME_RE: &str = r"^[A-Za-z0-9_ .\-]{1,128}$";
const DESTINATION_RE: &str = r"^[A-Za-z0-9_ ./=\-,]{1,256}$";
const BUILD_ID_RE: &str = r"^[0-9a-fA-F\-]{1,64}$";

pub fn validate_project_or_workspace(path_str: &str, root: &Path) -> Result<PathBuf> {
    let path = Path::new(path_str);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| Error::PathRejected(format!("missing extension: {path_str}")))?;
    if ext != "xcodeproj" && ext != "xcworkspace" {
        return Err(Error::PathRejected(format!(
            "must be .xcodeproj or .xcworkspace, got .{ext}"
        )));
    }
    if !path.exists() {
        return Err(Error::PathNotFound(path.to_path_buf()));
    }
    let canonical = path.canonicalize()?;
    let canonical_root = root.canonicalize()?;
    if !is_under_or_equal(&canonical, &canonical_root) {
        return Err(Error::PathRejected(format!(
            "path {} is outside root {}",
            canonical.display(),
            canonical_root.display()
        )));
    }
    Ok(canonical)
}

fn is_under_or_equal(path: &Path, root: &Path) -> bool {
    if path == root {
        return true;
    }
    path.starts_with(root)
}

pub fn validate_scheme(s: &str) -> Result<String> {
    let re = regex::Regex::new(SCHEME_RE).unwrap();
    if !re.is_match(s) {
        return Err(Error::InvalidArgument(format!("invalid scheme: {s:?}")));
    }
    Ok(s.to_string())
}

pub fn validate_configuration(c: &str) -> Result<String> {
    match c {
        "Debug" | "Release" => Ok(c.to_string()),
        _ => Err(Error::InvalidArgument(format!(
            "configuration must be Debug or Release: {c:?}"
        ))),
    }
}

pub fn validate_action(a: &str) -> Result<String> {
    match a {
        "build" | "clean" | "clean+build" => Ok(a.to_string()),
        _ => Err(Error::InvalidArgument(format!(
            "action must be build/clean/clean+build: {a:?}"
        ))),
    }
}

pub fn validate_destination(d: &str) -> Result<String> {
    let re = regex::Regex::new(DESTINATION_RE).unwrap();
    if !re.is_match(d) {
        return Err(Error::InvalidArgument(format!(
            "invalid destination: {d:?}"
        )));
    }
    Ok(d.to_string())
}

pub fn validate_timeout(t: Option<u32>) -> Result<u32> {
    match t {
        None => Ok(1800),
        Some(v) if (1..=7200).contains(&v) => Ok(v),
        Some(v) => Err(Error::InvalidArgument(format!(
            "timeout_secs must be 1..=7200: {v}"
        ))),
    }
}

pub fn validate_build_id(id: &str) -> Result<String> {
    let re = regex::Regex::new(BUILD_ID_RE).unwrap();
    if !re.is_match(id) {
        return Err(Error::InvalidArgument(format!("invalid build_id: {id:?}")));
    }
    Ok(id.to_string())
}
