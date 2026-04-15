//! Target classification for Level 2 (Typed) abstraction.
//!
//! Extracts semantic file type and language from trace context,
//! producing a 16-bit bucket that groups similar operations across projects.
//! "edit a Rust source file" becomes the same bucket regardless of which project.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Semantic file type — what role this file plays in a project.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum TargetKind {
    SourceFile,
    TestFile,
    ConfigFile,
    BuildOutput,
    Documentation,
    Schema,
}

impl TargetKind {
    /// Classify a file path into its semantic role.
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        let basename = lower.rsplit('/').next().unwrap_or(&lower);

        // Test files — check before source to win priority
        if basename.starts_with("test_")
            || basename.ends_with("_test.rs")
            || basename.ends_with("_test.go")
            || basename.ends_with("_test.py")
            || basename.ends_with("_test.ts")
            || basename.ends_with("_test.js")
            || basename.ends_with(".spec.ts")
            || basename.ends_with(".spec.js")
            || basename.ends_with(".test.ts")
            || basename.ends_with(".test.js")
            || lower.contains("/tests/")
            || lower.contains("/test/")
            || lower.starts_with("tests/")
        {
            return Self::TestFile;
        }

        // Build output
        if lower.contains("/target/")
            || lower.contains("/dist/")
            || lower.contains("/build/")
            || lower.contains("/node_modules/")
            || lower.starts_with("target/")
            || lower.starts_with("dist/")
        {
            return Self::BuildOutput;
        }

        // Documentation
        if basename.ends_with(".md")
            || lower.contains("/docs/")
            || lower.starts_with("docs/")
            || basename == "readme"
            || basename == "changelog"
            || basename == "license"
        {
            return Self::Documentation;
        }

        // Schema / migration
        if basename.ends_with(".proto")
            || basename.ends_with(".graphql")
            || basename.ends_with(".sql")
            || lower.contains("/migrations/")
            || lower.starts_with("migrations/")
        {
            return Self::Schema;
        }

        // Config files
        if basename == "cargo.toml"
            || basename == "package.json"
            || basename == "pyproject.toml"
            || basename == "tsconfig.json"
            || basename == "go.mod"
            || basename == "go.sum"
            || basename == "makefile"
            || basename == "dockerfile"
            || basename.ends_with(".yaml")
            || basename.ends_with(".yml")
            || basename.ends_with(".toml")
            || basename.ends_with(".json")
            || basename.ends_with(".lock")
            || basename.starts_with('.')
        {
            return Self::ConfigFile;
        }

        // Default: source file
        Self::SourceFile
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::SourceFile => "src",
            Self::TestFile => "test",
            Self::ConfigFile => "cfg",
            Self::BuildOutput => "build",
            Self::Documentation => "doc",
            Self::Schema => "schema",
        }
    }
}

/// Detect programming language from file extension.
pub fn detect_language(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "go" => "go",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "java" | "kt" | "kts" => "jvm",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "swift" => "swift",
        "zig" => "zig",
        "lua" => "lua",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "proto" => "protobuf",
        "graphql" | "gql" => "graphql",
        "toml" | "yaml" | "yml" | "json" => "config",
        "md" | "rst" | "txt" => "text",
        _ => "unknown",
    }
}

/// Compute a 16-bit typed bucket from a file path.
/// Groups: TargetKind × language → stable hash.
pub fn typed_bucket(path: &str) -> i64 {
    let kind = TargetKind::from_path(path);
    let lang = detect_language(path);
    let mut hasher = DefaultHasher::new();
    kind.as_str().hash(&mut hasher);
    lang.hash(&mut hasher);
    (hasher.finish() & 0xFFFF) as i64
}

/// Compute a 16-bit project bucket from a space string.
/// This is what Level 1 (Project) uses as its bucket.
pub fn space_bucket(space: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    space.hash(&mut hasher);
    (hasher.finish() & 0xFFFF) as i64
}

/// Try to extract a file path from trace context_text.
/// Handles common patterns from hook payloads:
///   "edit src/main.rs" → "src/main.rs"
///   "Read /Users/foo/bar.rs" → "/Users/foo/bar.rs"
///   JSON with "file_path" or "target" key
pub fn extract_file_path(context_text: &str) -> Option<&str> {
    // Try JSON first
    if context_text.starts_with('{') {
        // Quick substring extraction without full parse
        for key in ["file_path", "target", "path", "file"] {
            let pattern = format!("\"{}\":\"", key);
            if let Some(start) = context_text.find(&pattern) {
                let val_start = start + pattern.len();
                if let Some(end) = context_text[val_start..].find('"') {
                    return Some(&context_text[val_start..val_start + end]);
                }
            }
            // Also try with space after colon
            let pattern = format!("\"{}\": \"", key);
            if let Some(start) = context_text.find(&pattern) {
                let val_start = start + pattern.len();
                if let Some(end) = context_text[val_start..].find('"') {
                    return Some(&context_text[val_start..val_start + end]);
                }
            }
        }
    }

    // Find tokens that look like file paths
    context_text.split_whitespace().find(|token| {
        token.contains('/')
            && (token.contains('.')
                || token.ends_with("rs")
                || token.ends_with("py")
                || token.ends_with("go"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_kind_classification() {
        assert!(matches!(
            TargetKind::from_path("src/main.rs"),
            TargetKind::SourceFile
        ));
        assert!(matches!(
            TargetKind::from_path("src/pheromone_test.go"),
            TargetKind::TestFile
        ));
        assert!(matches!(
            TargetKind::from_path("tests/integration.rs"),
            TargetKind::TestFile
        ));
        assert!(matches!(
            TargetKind::from_path("test_utils.py"),
            TargetKind::TestFile
        ));
        assert!(matches!(
            TargetKind::from_path("component.spec.ts"),
            TargetKind::TestFile
        ));
        assert!(matches!(
            TargetKind::from_path("Cargo.toml"),
            TargetKind::ConfigFile
        ));
        assert!(matches!(
            TargetKind::from_path("package.json"),
            TargetKind::ConfigFile
        ));
        assert!(matches!(
            TargetKind::from_path(".env"),
            TargetKind::ConfigFile
        ));
        assert!(matches!(
            TargetKind::from_path("target/debug/binary"),
            TargetKind::BuildOutput
        ));
        assert!(matches!(
            TargetKind::from_path("README.md"),
            TargetKind::Documentation
        ));
        assert!(matches!(
            TargetKind::from_path("schema.proto"),
            TargetKind::Schema
        ));
        assert!(matches!(
            TargetKind::from_path("migrations/001_init.sql"),
            TargetKind::Schema
        ));
    }

    #[test]
    fn language_detection() {
        assert_eq!(detect_language("main.rs"), "rust");
        assert_eq!(detect_language("app.py"), "python");
        assert_eq!(detect_language("server.go"), "go");
        assert_eq!(detect_language("index.tsx"), "typescript");
        assert_eq!(detect_language("utils.js"), "javascript");
        assert_eq!(detect_language("Main.java"), "jvm");
        assert_eq!(detect_language("query.sql"), "sql");
        assert_eq!(detect_language("config.toml"), "config");
        assert_eq!(detect_language("README.md"), "text");
        assert_eq!(detect_language("binary"), "unknown");
    }

    #[test]
    fn typed_bucket_deterministic() {
        let b1 = typed_bucket("src/main.rs");
        let b2 = typed_bucket("src/main.rs");
        assert_eq!(b1, b2);
        assert!((0..=65535).contains(&b1));
    }

    #[test]
    fn typed_bucket_same_kind_same_lang() {
        // Two Rust source files → same bucket
        let b1 = typed_bucket("src/main.rs");
        let b2 = typed_bucket("src/pheromone.rs");
        assert_eq!(b1, b2, "same kind+lang should share bucket");
    }

    #[test]
    fn typed_bucket_different_kind() {
        // Source file vs test file in same language
        let b_src = typed_bucket("src/main.rs");
        let b_test = typed_bucket("tests/integration_test.rs");
        // They COULD collide in a 16-bit space but shouldn't with good hashing
        assert_ne!(b_src, b_test, "source and test should differ");
    }

    #[test]
    fn typed_bucket_different_lang() {
        let b_rust = typed_bucket("src/main.rs");
        let b_python = typed_bucket("src/main.py");
        assert_ne!(b_rust, b_python, "Rust and Python source should differ");
    }

    #[test]
    fn space_bucket_deterministic() {
        let b1 = space_bucket("Desktop/Thronglets");
        let b2 = space_bucket("Desktop/Thronglets");
        assert_eq!(b1, b2);
        assert!((0..=65535).contains(&b1));
    }

    #[test]
    fn space_bucket_different_projects() {
        let b1 = space_bucket("Desktop/Thronglets");
        let b2 = space_bucket("Desktop/oasyce-chain");
        assert_ne!(b1, b2);
    }

    #[test]
    fn extract_path_from_plain_text() {
        assert_eq!(extract_file_path("edit src/main.rs"), Some("src/main.rs"));
        assert_eq!(
            extract_file_path("Read /Users/foo/bar.py"),
            Some("/Users/foo/bar.py")
        );
    }

    #[test]
    fn extract_path_from_json() {
        let json = r#"{"file_path": "src/pheromone.rs", "tool": "Edit"}"#;
        assert_eq!(extract_file_path(json), Some("src/pheromone.rs"));
    }

    #[test]
    fn extract_path_returns_none_for_no_path() {
        assert_eq!(extract_file_path("git status"), None);
        assert_eq!(extract_file_path(""), None);
    }
}
