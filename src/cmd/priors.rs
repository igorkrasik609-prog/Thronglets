use super::*;

use thronglets::ambient::{AMBIENT_PRIOR_SCHEMA_VERSION, AmbientPriorRequest, ambient_prior_data};

pub(crate) fn version(_base: &BaseCtx, json: bool) {
    let binary_path = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("thronglets"))
        .display()
        .to_string();
    let data = VersionData {
        summary: VersionSummary {
            status: "ready",
            version: env!("CARGO_PKG_VERSION").to_string(),
            bootstrap_schema_version: BOOTSTRAP_SCHEMA_VERSION,
            identity_schema_version: IDENTITY_SCHEMA_VERSION,
        },
        binary_path,
        source_hint: "If you are operating inside the Thronglets repo, prefer `cargo run --quiet -- <command>` so the binary matches the checked-out docs and source.",
        capabilities: VersionCapabilities {
            connection_export_surfaces: vec!["thronglets", "oasyce"],
            managed_runtime_surface: "thronglets-managed",
            managed_runtime_refresh_command: "thronglets setup",
        },
    };
    if json {
        print_machine_json_with_schema(VERSION_SCHEMA_VERSION, "version", &data);
    } else {
        render_version_report(&data);
    }
}

pub(crate) fn ambient_priors(base: &BaseCtx, json: bool) {
    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        std::process::exit(0);
    }

    let request: AmbientPriorRequest = match serde_json::from_str(&input) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("ambient-priors JSON parse error: {error}");
            std::process::exit(1);
        }
    };

    let data = ambient_prior_data(&open_store(&base.dir), &request);
    if json {
        print_machine_json_with_schema(AMBIENT_PRIOR_SCHEMA_VERSION, "ambient-priors", &data);
    } else {
        for prior in &data.priors {
            println!("{} ({:.2})", prior.summary, prior.confidence);
        }
    }
}

pub(crate) fn rebuild_priors(base: &BaseCtx, json: bool) {
    let store = open_store(&base.dir);
    let data = rebuild_priors_data(&base.dir, &store);
    if json {
        print_machine_json_with_schema(DERIVED_GUIDANCE_SCHEMA_VERSION, "rebuild-priors", &data);
    } else {
        render_rebuild_priors_report(&data);
    }
}
