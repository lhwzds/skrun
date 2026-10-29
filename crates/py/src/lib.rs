//! # skrun-native
//!
//! PyO3 module boundary for the skrun executable skill runtime.
//!
//! ## Owns
//! - skrun_native Python module
//! - executable skill runtime JSON ABI entrypoints
//! - Python-to-Rust error conversion
//!
//! ## Must Not
//! - duplicate runtime logic in Python
//! - render UI
//!
//! ## Inputs
//! - executable skill artifact directories
//! - executable skill JSON input
//!
//! ## Outputs
//! - executable skill runtime JSON values
//!
//! ## Depends On
//! - skrun
//!
//! ## Used By
//! - python/skrun SkillRuntime
//!
//! ## Verify
//! - cargo test -p skrun-native --features python-module

use anyhow::Result;
#[cfg(feature = "python-module")]
use pyo3::exceptions::PyRuntimeError;
#[cfg(feature = "python-module")]
use pyo3::prelude::*;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(feature = "python-module")]
#[pyfunction]
fn runtime_load_artifact_json(py: Python<'_>, root: &str) -> PyResult<String> {
    py.detach(|| load_artifact_json(root)).map_err(to_py_error)
}

#[cfg(feature = "python-module")]
#[pyfunction]
fn runtime_list_skills_json(py: Python<'_>, root: Option<String>) -> PyResult<String> {
    py.detach(|| list_skills_json(root)).map_err(to_py_error)
}

#[cfg(feature = "python-module")]
#[pyfunction]
fn runtime_build_skill_json(
    py: Python<'_>,
    root: &str,
    target_dir: Option<String>,
) -> PyResult<String> {
    py.detach(|| build_skill_json(root, target_dir))
        .map_err(to_py_error)
}

#[cfg(feature = "python-module")]
#[pyfunction]
fn runtime_run_skill_json(
    py: Python<'_>,
    root: &str,
    input_json: &str,
    timeout_seconds: u64,
) -> PyResult<String> {
    py.detach(|| run_skill_json(root, input_json, timeout_seconds))
        .map_err(to_py_error)
}

#[cfg(feature = "python-module")]
#[pyfunction]
fn runtime_install_local_skill_json(
    py: Python<'_>,
    source: &str,
    root: Option<String>,
    skill_id: Option<String>,
    overwrite: bool,
) -> PyResult<String> {
    py.detach(|| install_local_skill_json(source, root, skill_id, overwrite))
        .map_err(to_py_error)
}

#[cfg(feature = "python-module")]
#[pymodule]
fn skrun_native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(runtime_load_artifact_json, module)?)?;
    module.add_function(wrap_pyfunction!(runtime_list_skills_json, module)?)?;
    module.add_function(wrap_pyfunction!(runtime_build_skill_json, module)?)?;
    module.add_function(wrap_pyfunction!(runtime_run_skill_json, module)?)?;
    module.add_function(wrap_pyfunction!(runtime_install_local_skill_json, module)?)?;
    Ok(())
}

#[cfg(feature = "python-module")]
fn to_py_error(error: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

pub fn load_artifact_json(root: &str) -> Result<String> {
    Ok(serde_json::to_string(&skrun::load_artifact(root)?)?)
}

pub fn list_skills_json(root: Option<String>) -> Result<String> {
    let root = runtime_root(root)?;
    Ok(serde_json::to_string(&skrun::list_installed_skills(root)?)?)
}

pub fn build_skill_json(root: &str, target_dir: Option<String>) -> Result<String> {
    let options = skrun::BuildOptions {
        target_dir: target_dir.map(PathBuf::from),
        ..skrun::BuildOptions::default()
    };
    Ok(serde_json::to_string(&skrun::build_skill(root, &options)?)?)
}

pub fn run_skill_json(root: &str, input_json: &str, timeout_seconds: u64) -> Result<String> {
    let input: Value = serde_json::from_str(input_json)?;
    let options = skrun::RunOptions {
        timeout: Duration::from_secs(timeout_seconds),
        ..skrun::RunOptions::default()
    };
    Ok(serde_json::to_string(
        &skrun::run_skill(root, input, &options)?.value,
    )?)
}

pub fn install_local_skill_json(
    source: &str,
    root: Option<String>,
    skill_id: Option<String>,
    overwrite: bool,
) -> Result<String> {
    let mut options = skrun::InstallOptions::new(runtime_root(root)?).with_overwrite(overwrite);
    if let Some(skill_id) = skill_id {
        options = options.with_skill_id(skill_id);
    }
    Ok(serde_json::to_string(&skrun::install_local_skill(
        source, &options,
    )?)?)
}

fn runtime_root(root: Option<String>) -> Result<PathBuf> {
    match root {
        Some(root) if !root.is_empty() => Ok(PathBuf::from(root)),
        _ => skrun::default_skills_dir(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_load_artifact_json_returns_artifact() {
        let root = temp_dir("runtime-artifact");
        skrun::scaffold_skill(
            &root,
            skrun::ScaffoldOptions::python_uv("py-echo", "Python Echo", "0.1.0"),
        )
        .unwrap();

        let response = load_artifact_json(root.to_str().unwrap()).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&response).unwrap();

        assert_eq!(decoded["id"], "py-echo");
        assert_eq!(decoded["kind"], "python_uv");
    }

    #[test]
    fn runtime_install_and_list_skill_json_round_trip() {
        let source = temp_dir("runtime-install-source");
        let target = temp_dir("runtime-install-target");
        skrun::scaffold_skill(
            &source,
            skrun::ScaffoldOptions::python_uv("py-echo", "Python Echo", "0.1.0"),
        )
        .unwrap();

        let installed = install_local_skill_json(
            source.to_str().unwrap(),
            Some(target.to_str().unwrap().to_string()),
            None,
            false,
        )
        .unwrap();
        let listed = list_skills_json(Some(target.to_str().unwrap().to_string())).unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&installed).unwrap()["id"],
            "py-echo"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&listed).unwrap()[0]["id"],
            "py-echo"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_run_skill_json_returns_output_value() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("runtime-run");
        let artifact = skrun::SkillArtifact::rust_binary("echo", "Echo", "0.1.0");
        skrun::save_artifact(&root, &artifact).unwrap();
        let entry = artifact.entry_path(&root).unwrap();
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        fs::write(&entry, "#!/bin/sh\ncat\n").unwrap();
        let mut permissions = fs::metadata(&entry).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entry, permissions).unwrap();

        let response =
            run_skill_json(root.to_str().unwrap(), r#"{"message":"hello"}"#, 10).unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap(),
            json!({ "message": "hello" })
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("skrun-native-{name}-{}-{now}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
