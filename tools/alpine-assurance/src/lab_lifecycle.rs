use std::{fs, path::Path};

const SCHEMA: &str = "alpine-zed-lab-atlas-lifecycle-evidence/v1";
const CANONICAL: &str = include_str!("../../../assurance/lab/v3/task-353-atlas-lifecycle.toml");

pub(crate) fn is_lifecycle_evidence(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|source| {
        source
            .lines()
            .any(|line| line.trim() == format!("schema = {SCHEMA:?}"))
    })
}

pub(crate) fn run(command: &str, path: &Path) -> Result<String, Vec<String>> {
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("failed to read {}: {error}", path.display())])?;
    validate_source(&source)?;

    match command {
        "validate-zed-lab-evidence" => Ok(
            "validated Task #353 atlas lifecycle evidence with hosted offline GPUI and physical Direct Metal across six transitions"
                .to_owned(),
        ),
        "zed-lab-evidence-report" => Ok(render_report()),
        other => Err(vec![format!(
            "unsupported Zed lab evidence command {other:?}"
        )]),
    }
}

fn validate_source(source: &str) -> Result<(), Vec<String>> {
    let parsed = toml::from_str::<toml::Value>(source)
        .map_err(|error| vec![format!("failed to parse Task #353 evidence: {error}")])?;
    let schema = parsed.get("schema").and_then(toml::Value::as_str);
    let mut errors = Vec::new();
    if schema != Some(SCHEMA) {
        errors.push("Task #353 evidence schema must remain version 1".to_owned());
    }
    if source != CANONICAL {
        errors.push("Task #353 evidence must byte-match the reviewed canonical record".to_owned());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn render_report() -> String {
    "# Zed GPUI atlas lifecycle evidence\n\n\
- Lab revision: `c5aaa5d78c63d3135d9077ae3d45fc758c08e8fa`\n\
- Zed revision: `e17dc4f9d50db73a458b64dcce50ecd4878b98a3`\n\
- Alpine revision: `4e6ee28668d3cd7a62e347e7b9f96c99318956ab`\n\
- Hosted run: https://github.com/dbuddha/alpine-zed-lab/actions/runs/33346520970\n\
- Hosted shader mode: offline metallib\n\
- Physical shader mode: runtime source, supporting and unqualified\n\
\n\
| Step | Transition | Atlas upload | GPUI allocation | Terminal owner | Exact Metal |\n\
| ---: | --- | ---: | ---: | --- | --- |\n\
| 0 | Full admission | 4 B | 1 | live | yes |\n\
| 1 | Compatible reuse | 0 B | 1 | live | yes |\n\
| 2 | Content replacement | 4 B | 2 | live | yes |\n\
| 3 | Capacity replacement | 8 B | 3 | live | yes |\n\
| 4 | Teardown | 0 B | 3 | released | yes |\n\
| 5 | Full resynchronization | 8 B | 4 | released after run | yes |\n\
\n\
All five visible steps match the CPU oracle, pinned GPUI Metal, and Alpine Direct Metal exactly. Warm compatible reuse performs no atlas admission or upload. Teardown releases both logical owners before generation two reconstructs the atlas.\n\
\n\
No timing, memory-superiority, latency, presentation, product, or performance claim is present. Hosted and physical runs compose through identical pinned GPUI and CPU output identities; runtime-source physical shaders remain explicitly unqualified.\n"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{CANONICAL, is_lifecycle_evidence, render_report, run, validate_source};
    use std::path::Path;

    #[test]
    fn canonical_record_validates_and_reports_the_claim_boundary() {
        assert_eq!(validate_source(CANONICAL), Ok(()));
        let report = render_report();
        assert!(report.contains("Compatible reuse | 0 B"));
        assert!(report.contains("No timing, memory-superiority"));
        assert!(report.contains("runtime-source physical shaders remain explicitly unqualified"));
    }

    #[test]
    fn any_identity_or_claim_drift_is_rejected() {
        for (from, to) in [
            (
                "lab_revision = \"c5aaa5d78c63d3135d9077ae3d45fc758c08e8fa\"",
                "lab_revision = \"05aaa5d78c63d3135d9077ae3d45fc758c08e8fa\"",
            ),
            (
                "performance_qualified = false",
                "performance_qualified = true",
            ),
            (
                "artifact_digest_sha256 = \"83baf8d27e2d1810f6122f681fd2fcdd4dc86db1059ac74ff546c0abaa7c2c9c\"",
                "artifact_digest_sha256 = \"03baf8d27e2d1810f6122f681fd2fcdd4dc86db1059ac74ff546c0abaa7c2c9c\"",
            ),
            (
                "path = \"assurance/lab/v3/source/readbacks/fbb7e42e4eed8f8b98468fa42a339ffd615bfe81e4805046b599a5b9b8c1d4be.bgra\"",
                "path = \"assurance/lab/v3/source/readbacks/unknown.bgra\"",
            ),
        ] {
            let changed = CANONICAL.replacen(from, to, 1);
            assert_ne!(changed, CANONICAL, "control must alter canonical evidence");
            assert!(validate_source(&changed).is_err());
        }
    }

    #[test]
    fn malformed_and_unknown_schema_records_are_rejected() {
        assert!(validate_source("schema = [").is_err());
        let changed = CANONICAL.replacen(
            "alpine-zed-lab-atlas-lifecycle-evidence/v1",
            "alpine-zed-lab-atlas-lifecycle-evidence/v2",
            1,
        );
        assert!(validate_source(&changed).is_err());
    }

    #[test]
    fn dispatch_identifies_only_the_lifecycle_schema() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = repository.join("assurance/lab/v3/task-353-atlas-lifecycle.toml");
        assert!(is_lifecycle_evidence(&path));
        assert!(!is_lifecycle_evidence(
            &repository.join("assurance/lab/v2/task-61-realistic-scenes.toml")
        ));
        assert!(run("validate-zed-lab-evidence", &path).is_ok());
        assert!(run("zed-lab-evidence-report", &path).is_ok());
        assert!(run("unsupported", &path).is_err());
    }
}
