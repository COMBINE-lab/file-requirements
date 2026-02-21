use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// A boolean file existence requirement expression.
///
/// - [`FileRequirement::All`] is a conjunction (`AND`)
/// - [`FileRequirement::Any`] is a disjunction (`OR`)
#[derive(Debug, Clone)]
pub enum FileRequirement {
    /// A single file term that must exist.
    File(PathBuf),
    /// All children must be satisfied.
    All(Vec<FileRequirement>),
    /// At least one child must be satisfied.
    Any(Vec<FileRequirement>),
}

/// Errors produced while building a requirement expression.
#[derive(Debug, Error)]
pub enum FileRequirementBuildError {
    /// A term was inserted twice anywhere in the tree.
    #[error(
        "File term `{path}` was inserted more than once. Each file can appear in at most one clause."
    )]
    DuplicateFile { path: String },
    /// A group was created but no children were added.
    #[error("Cannot create an empty `{group}` group.")]
    EmptyGroup { group: &'static str },
}

/// Errors produced when checking a built requirement expression.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct FileRequirementCheckError {
    message: String,
}

impl FileRequirementCheckError {
    fn from_context(ctx: CheckContext) -> Self {
        let mut sections: Vec<String> = Vec::new();
        if !ctx.missing_files.is_empty() {
            sections.push(format!(
                "missing files: {}",
                ctx.missing_files.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if !ctx.io_errors.is_empty() {
            sections.push(format!(
                "path check errors: {}",
                ctx.io_errors.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if !ctx.unsatisfied_disjunctions.is_empty() {
            sections.push(format!(
                "unsatisfied disjunction(s): {}",
                ctx.unsatisfied_disjunctions
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Self {
            message: format!(
                "Required input files were missing or incomplete ({})",
                sections.join("; ")
            ),
        }
    }
}

/// Builder for composable file requirements.
///
/// The root group is an implicit `AND` group.
pub struct FileRequirementBuilder {
    root_terms: Vec<FileRequirement>,
    seen_terms: HashSet<PathBuf>,
}

impl FileRequirementBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            root_terms: Vec::new(),
            seen_terms: HashSet::new(),
        }
    }

    /// Add a required file to the root conjunction.
    pub fn require_file<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<&mut Self, FileRequirementBuildError> {
        GroupBuilder::new(&mut self.root_terms, &mut self.seen_terms).require_file(path)?;
        Ok(self)
    }

    /// Add a nested conjunction (`AND`) to the root conjunction.
    pub fn require_all<F>(&mut self, f: F) -> Result<&mut Self, FileRequirementBuildError>
    where
        F: FnOnce(&mut GroupBuilder<'_>) -> Result<(), FileRequirementBuildError>,
    {
        GroupBuilder::new(&mut self.root_terms, &mut self.seen_terms).require_all(f)?;
        Ok(self)
    }

    /// Add a nested disjunction (`OR`) to the root conjunction.
    pub fn require_any<F>(&mut self, f: F) -> Result<&mut Self, FileRequirementBuildError>
    where
        F: FnOnce(&mut GroupBuilder<'_>) -> Result<(), FileRequirementBuildError>,
    {
        GroupBuilder::new(&mut self.root_terms, &mut self.seen_terms).require_any(f)?;
        Ok(self)
    }

    /// Build the final requirement expression.
    pub fn build(self) -> FileRequirement {
        FileRequirement::All(self.root_terms)
    }
}

impl Default for FileRequirementBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Nested group builder used to create `AND` / `OR` sub-expressions.
pub struct GroupBuilder<'a> {
    target: &'a mut Vec<FileRequirement>,
    seen_terms: &'a mut HashSet<PathBuf>,
}

impl<'a> GroupBuilder<'a> {
    fn new(target: &'a mut Vec<FileRequirement>, seen_terms: &'a mut HashSet<PathBuf>) -> Self {
        Self { target, seen_terms }
    }

    /// Add a required file term to this group.
    pub fn require_file<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<&mut Self, FileRequirementBuildError> {
        let owned_path = path.as_ref().to_path_buf();
        if !self.seen_terms.insert(owned_path.clone()) {
            return Err(FileRequirementBuildError::DuplicateFile {
                path: owned_path.display().to_string(),
            });
        }
        self.target.push(FileRequirement::File(owned_path));
        Ok(self)
    }

    /// Add a nested conjunction (`AND`) group.
    pub fn require_all<F>(&mut self, f: F) -> Result<&mut Self, FileRequirementBuildError>
    where
        F: FnOnce(&mut GroupBuilder<'_>) -> Result<(), FileRequirementBuildError>,
    {
        let mut child_terms = Vec::new();
        f(&mut GroupBuilder::new(&mut child_terms, self.seen_terms))?;
        if child_terms.is_empty() {
            return Err(FileRequirementBuildError::EmptyGroup { group: "AND" });
        }
        self.target.push(FileRequirement::All(child_terms));
        Ok(self)
    }

    /// Add a nested disjunction (`OR`) group.
    pub fn require_any<F>(&mut self, f: F) -> Result<&mut Self, FileRequirementBuildError>
    where
        F: FnOnce(&mut GroupBuilder<'_>) -> Result<(), FileRequirementBuildError>,
    {
        let mut child_terms = Vec::new();
        f(&mut GroupBuilder::new(&mut child_terms, self.seen_terms))?;
        if child_terms.is_empty() {
            return Err(FileRequirementBuildError::EmptyGroup { group: "OR" });
        }
        self.target.push(FileRequirement::Any(child_terms));
        Ok(self)
    }
}

impl FileRequirement {
    /// Validate this requirement expression against the local filesystem.
    pub fn check(&self) -> Result<(), FileRequirementCheckError> {
        let mut ctx = CheckContext::default();
        if self.evaluate(&mut ctx) {
            Ok(())
        } else {
            Err(FileRequirementCheckError::from_context(ctx))
        }
    }

    fn evaluate(&self, ctx: &mut CheckContext) -> bool {
        match self {
            FileRequirement::File(path) => match path.try_exists() {
                Ok(true) => true,
                Ok(false) => {
                    ctx.missing_files.insert(path.display().to_string());
                    false
                }
                Err(e) => {
                    ctx.io_errors.insert(format!("{} ({})", path.display(), e));
                    false
                }
            },
            FileRequirement::All(children) => {
                let mut all_ok = true;
                for child in children {
                    if !child.evaluate(ctx) {
                        all_ok = false;
                    }
                }
                all_ok
            }
            FileRequirement::Any(children) => {
                let mut branch_contexts = Vec::with_capacity(children.len());
                for child in children {
                    let mut branch_ctx = CheckContext::default();
                    if child.evaluate(&mut branch_ctx) {
                        return true;
                    }
                    branch_contexts.push(branch_ctx);
                }
                for branch_ctx in branch_contexts {
                    ctx.merge(branch_ctx);
                }
                ctx.unsatisfied_disjunctions.insert(self.to_string());
                false
            }
        }
    }
}

impl std::fmt::Display for FileRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileRequirement::File(path) => write!(f, "{}", path.display()),
            FileRequirement::All(children) => {
                let joined = children
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" AND ");
                write!(f, "({})", joined)
            }
            FileRequirement::Any(children) => {
                let joined = children
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" OR ");
                write!(f, "({})", joined)
            }
        }
    }
}

#[derive(Default)]
struct CheckContext {
    missing_files: BTreeSet<String>,
    io_errors: BTreeSet<String>,
    unsatisfied_disjunctions: BTreeSet<String>,
}

impl CheckContext {
    fn merge(&mut self, other: CheckContext) {
        self.missing_files.extend(other.missing_files);
        self.io_errors.extend(other.io_errors);
        self.unsatisfied_disjunctions
            .extend(other.unsatisfied_disjunctions);
    }
}

#[cfg(test)]
mod tests {
    use super::{FileRequirementBuildError, FileRequirementBuilder};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn builder_rejects_duplicate_term_across_nested_groups() {
        let mut b = FileRequirementBuilder::new();
        let err = match b.require_any(|any| {
            any.require_file("a.txt")?;
            any.require_all(|all| {
                all.require_file("b.txt")?;
                all.require_file("a.txt")?;
                Ok(())
            })?;
            Ok(())
        }) {
            Ok(_) => panic!("expected duplicate file insertion to fail"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            FileRequirementBuildError::DuplicateFile { .. }
        ));
    }

    #[test]
    fn checker_succeeds_when_or_clause_satisfied_by_compound_branch() {
        let td = tempdir().unwrap();
        let base = td.path().join("idx");
        fs::write(base.with_extension("ctab"), "").unwrap();
        fs::write(base.with_extension("ssi"), "").unwrap();
        fs::write(base.with_extension("ssi.mphf"), "").unwrap();

        let mut b = FileRequirementBuilder::new();
        b.require_file(base.with_extension("ctab")).unwrap();
        b.require_any(|any| {
            any.require_file(base.with_extension("sshash"))?;
            any.require_all(|all| {
                all.require_file(base.with_extension("ssi"))?;
                all.require_file(base.with_extension("ssi.mphf"))?;
                Ok(())
            })?;
            Ok(())
        })
        .unwrap();
        let req = b.build();
        assert!(req.check().is_ok());
    }

    #[test]
    fn checker_fails_when_no_or_branch_is_satisfied() {
        let td = tempdir().unwrap();
        let base = td.path().join("idx");
        fs::write(base.with_extension("ctab"), "").unwrap();
        fs::write(base.with_extension("ssi"), "").unwrap();

        let mut b = FileRequirementBuilder::new();
        b.require_file(base.with_extension("ctab")).unwrap();
        b.require_any(|any| {
            any.require_file(base.with_extension("sshash"))?;
            any.require_all(|all| {
                all.require_file(base.with_extension("ssi"))?;
                all.require_file(base.with_extension("ssi.mphf"))?;
                Ok(())
            })?;
            Ok(())
        })
        .unwrap();
        let req = b.build();
        let err = req.check().expect_err("expected OR clause to fail");
        let rendered = format!("{:#}", err);
        assert!(rendered.contains("unsatisfied disjunction"));
        assert!(rendered.contains("sshash"));
        assert!(rendered.contains("ssi.mphf"));
    }
}
