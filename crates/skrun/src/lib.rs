//! Public API for the skrun executable skill runtime.
//!
//! This crate is intentionally self-contained so downstream projects can depend
//! on `skrun` from crates.io without pulling the internal workspace crates.

pub mod runtime;

pub use runtime::{
    ArtifactKind, ArtifactProtocol, ArtifactSchema, ArtifactSource, BuildOptions, InstallOptions,
    RunOptions, ScaffoldOptions, SkillArtifact, SkillRunOutput, build_skill, default_skills_dir,
    install_local_skill, list_installed_skills, load_artifact, run_skill, save_artifact,
    scaffold_skill,
};
