use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    DECLARATION_SCHEMA_VERSION, DeclarationCandidate, DeclarationKind, LeanCancellation,
    LeanExportError, LeanRunStatus, LeanRunner, LeanRunnerOptions, ManagedLeanProfile,
    runner::LeanCommand,
};

const EXPORT_DIRECTORY: &str = ".mathscape/exporter";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarationExportRequest {
    names: Vec<String>,
    max_results: usize,
    max_type_chars: usize,
}

impl DeclarationExportRequest {
    pub fn exact_names<I, S>(names: I) -> Result<Self, LeanExportError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let names = names.into_iter().map(Into::into).collect::<Vec<_>>();
        validate_names(&names)?;
        Ok(Self {
            max_results: names.len().max(1),
            names,
            max_type_chars: 16 * 1024,
        })
    }

    pub fn all(max_results: usize) -> Result<Self, LeanExportError> {
        if max_results == 0 {
            return Err(LeanExportError::InvalidOption {
                field: "lean.export.max_results",
                reason: "must be greater than zero",
            });
        }
        Ok(Self {
            names: Vec::new(),
            max_results,
            max_type_chars: 16 * 1024,
        })
    }

    pub fn max_type_chars(mut self, maximum: usize) -> Result<Self, LeanExportError> {
        if maximum == 0 {
            return Err(LeanExportError::InvalidOption {
                field: "lean.export.max_type_chars",
                reason: "must be greater than zero",
            });
        }
        self.max_type_chars = maximum;
        Ok(self)
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub const fn max_results(&self) -> usize {
        self.max_results
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeclarationExportResult {
    pub declarations: Vec<DeclarationCandidate>,
    pub truncated: bool,
    pub profile_fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct DeclarationExporter {
    runner: LeanRunner,
}

impl DeclarationExporter {
    pub fn new(runner: LeanRunner) -> Self {
        Self { runner }
    }

    pub fn export(
        &self,
        profile: &ManagedLeanProfile,
        request: &DeclarationExportRequest,
    ) -> Result<DeclarationExportResult, LeanExportError> {
        let project_root = &profile.metadata().project_root;
        let lake = profile.probe().lake_executable.as_deref().ok_or(
            LeanExportError::CapabilityUnavailable {
                reason: "Lake executable is unavailable",
            },
        )?;
        if !profile.probe().is_ready() {
            return Err(LeanExportError::CapabilityUnavailable {
                reason: "Lean profile is not ready",
            });
        }
        let managed_root = profile
            .fingerprint()
            .cache_root
            .as_deref()
            .map(PathBuf::from)
            .ok_or(LeanExportError::CapabilityUnavailable {
                reason: "managed profile cache root is unavailable",
            })?;
        let script = write_script(&managed_root, request)?;
        let command = LeanCommand::new(lake.to_owned(), project_root.to_owned()).args([
            "env".to_owned(),
            "lean".to_owned(),
            script.to_string_lossy().into_owned(),
        ]);
        let run = self.runner.run(&command, &LeanCancellation::new())?;
        let cleanup = fs::remove_file(&script);
        if let Err(error) = cleanup
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(LeanExportError::Cleanup {
                path: script,
                source: error,
            });
        }
        if run.status() != LeanRunStatus::Succeeded {
            return Err(LeanExportError::RunFailed {
                status: format!("{:?}", run.status()),
                stderr: run.stderr().lossy(),
            });
        }
        if run.stdout().is_truncated() {
            return Err(LeanExportError::OutputTruncated);
        }
        let mut declarations = Vec::new();
        for (line_number, line) in run.stdout().lossy().lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let raw: RawDeclaration =
                serde_json::from_str(line).map_err(|source| LeanExportError::InvalidJson {
                    line: line_number + 1,
                    source,
                })?;
            let type_text = truncate_chars(&raw.type_text, request.max_type_chars);
            declarations.push(DeclarationCandidate {
                schema_version: DECLARATION_SCHEMA_VERSION,
                profile_fingerprint: profile.fingerprint().digest.clone(),
                name: raw.name,
                kind: raw.kind,
                module: raw.module,
                type_truncated: type_text.len() < raw.type_text.len(),
                type_text,
                docstring: None,
                source_file: None,
                source_line: None,
                source_column: None,
                dependencies: Vec::new(),
                metadata_complete: false,
            });
        }
        declarations.sort_by(|left, right| left.name.cmp(&right.name));
        let truncated = request.names.is_empty() && declarations.len() >= request.max_results;
        Ok(DeclarationExportResult {
            declarations,
            truncated,
            profile_fingerprint: profile.fingerprint().digest.clone(),
        })
    }
}

impl Default for DeclarationExporter {
    fn default() -> Self {
        let options = LeanRunnerOptions::new(Duration::from_secs(5 * 60))
            .expect("non-zero exporter timeout")
            .max_output_bytes(16 * 1024 * 1024)
            .expect("non-zero exporter capture limit");
        Self::new(LeanRunner::new(options))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDeclaration {
    name: String,
    kind: DeclarationKind,
    module: String,
    #[serde(rename = "type")]
    type_text: String,
}

fn validate_names(names: &[String]) -> Result<(), LeanExportError> {
    for name in names {
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(LeanExportError::InvalidDeclarationName(name.clone()));
        }
    }
    Ok(())
}

fn write_script(
    managed_root: &Path,
    request: &DeclarationExportRequest,
) -> Result<PathBuf, LeanExportError> {
    let directory = managed_root.join(EXPORT_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|source| LeanExportError::Access {
        path: directory.clone(),
        source,
    })?;
    let path = directory.join(format!("export-{}.lean", std::process::id()));
    let names = serde_json::to_string(&request.names).map_err(LeanExportError::Serialize)?;
    let source = format!(
        r#"import Mathlib

open Lean Elab Command Meta

private def kindOf : ConstantInfo → String
  | .axiomInfo _ => "axiom"
  | .defnInfo _ => "definition"
  | .thmInfo _ => "theorem"
  | .opaqueInfo _ => "opaque"
  | .quotInfo _ => "quotient"
  | .inductInfo _ => "inductive"
  | .ctorInfo _ => "constructor"
  | .recInfo _ => "recursor"

elab "mathscape_export" : command => do
  let env ← getEnv
        let requested : Array String := #{names}
  let mut emitted := 0
  for (name, info) in env.constants.toList do
    if emitted < {max_results} && (requested.isEmpty || requested.contains name.toString) then
      let ty ← liftTermElabM <| Meta.ppExpr info.type
      let module := (env.getModuleIdxFor? name).map (fun idx => env.allImportedModuleNames[idx.toNat]!.toString) |>.getD ""
      let row := Json.mkObj [
        ("name", name.toString),
        ("kind", kindOf info),
        ("type", ty.pretty),
        ("module", module)
      ]
      IO.println row.compress
      emitted := emitted + 1

mathscape_export
"#,
        names = names,
        max_results = request.max_results,
    );
    fs::write(&path, source).map_err(|source| LeanExportError::Access {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_rejects_empty_names_and_accepts_exact_names() {
        assert!(DeclarationExportRequest::exact_names([""]).is_err());
        let request = DeclarationExportRequest::exact_names(["Nat.add_comm"]).unwrap();
        assert_eq!(request.names(), &["Nat.add_comm".to_owned()]);
    }

    #[test]
    fn truncates_type_text_at_character_boundary() {
        assert_eq!(truncate_chars("αβγ", 2), "αβ");
    }
}
