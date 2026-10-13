mod contract;
mod engine;
mod platform;
#[cfg(target_os = "linux")]
mod process;
#[cfg(not(target_os = "linux"))]
#[path = "process_portable.rs"]
mod process;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "macos")]
mod macos;

use serde_json::{Value, json};
use std::{env, fs, path::Path};

fn main() {
    process::install_signal_handlers();
    let result = dispatch();
    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value).unwrap());
            if value.get("cancelled") == Some(&Value::Bool(true)) {
                std::process::exit(5);
            }
            if value.get("success") == Some(&Value::Bool(false)) {
                std::process::exit(3);
            }
        }
        Err(err) => {
            println!(
                "{}",
                json!({"schema_version":1,"success":false,"error":{"code":err.code,"message":err.message}})
            );
            std::process::exit(err.exit_code);
        }
    }
}

fn dispatch() -> contract::Result<Value> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 1 && args[0] == "capabilities" {
        return Ok(json!({"schema_version":1,"success":true,"name":"dcc-media-enhance",
            "version":env!("CARGO_PKG_VERSION"),"target":env!("DCC_MEDIA_TARGET"),
            "commands":["capabilities","doctor","schema","probe","plan","run","batch","status","cancel"],
            "job_schema_version":1,"batch_schema_version":1,"stdout":"one_json_value",
            "backend_installation":"explicit_external_provisioning_only",
            "cache_reuse_supported":process::cache_reuse_supported(),
            "memory_enforcement":process::memory_enforcement(),
            "execution":"synchronous_owned_supervisor","cancellation":"owned_state_sentinel",
            "dcc_mcp":{"integration":"private_skill","discovery":"DCC_MCP_SKILL_PATHS","tools_manifest":"skill/media-enhance/tools.yaml"},
            "exit_codes":{"0":"success","2":"invalid_request_or_existing_output","3":"backend_io_or_batch_failure","4":"budget_exceeded","5":"cancelled"}}));
    }
    if args.len() == 1 && args[0] == "doctor" {
        return engine::doctor();
    }
    if args.len() == 1 && args[0] == "schema" {
        return Ok(serde_json::from_str(include_str!(
            "../schemas/job.schema.json"
        ))?);
    }
    if args.len() == 1 && args[0] == "--version" {
        return Ok(
            json!({"name":"dcc-media-enhance","version":env!("CARGO_PKG_VERSION"),"schema_version":1,"target":env!("DCC_MEDIA_TARGET"),"source_commit":env!("DCC_MEDIA_BUILD_COMMIT")}),
        );
    }
    if args.len() == 2 && args[0] == "probe" {
        return engine::probe(Path::new(&args[1]));
    }
    if args.len() == 2 && matches!(args[0].as_str(), "plan" | "run" | "batch") {
        if args[0] == "batch" {
            return engine::batch(Path::new(&args[1]));
        }
        let (job, base) = contract::load_job(Path::new(&args[1]))?;
        if args[0] == "plan" {
            return Ok(serde_json::to_value(engine::plan(&job, &base)?)?);
        }
        return engine::run(&job, &base);
    }
    if args.len() == 2 && args[0] == "status" {
        let mut value = contract::read_json_bounded(Path::new(&args[1]), 1_048_576)?;
        validate_state(&value)?;
        if value["status"] == "running" || value["status"] == "committing" {
            if value["supervisor_pid_namespace"].as_str() != process::pid_namespace().as_deref() {
                value["liveness"] = json!("not_observable_from_this_pid_namespace");
                return Ok(value);
            }
            let alive = value["supervisor_pid"]
                .as_u64()
                .and_then(|n| process::process_identity(n as u32));
            if alive.is_none() || alive.as_deref() != value["supervisor_identity"].as_str() {
                value["status"] = json!("interrupted");
                value["recovery"] = json!(
                    "Supervisor no longer owns this job. Inspect staging; retry the same job to reacquire its released OS lock. Only a committed manifest proves completion."
                );
            }
        }
        return Ok(value);
    }
    if args.len() == 2 && args[0] == "cancel" {
        let state = contract::read_json_bounded(Path::new(&args[1]), 1_048_576)?;
        validate_state(&state)?;
        if state["status"] != "running" {
            return Ok(json!({"success":true,"cancel_requested":false,"status":state["status"]}));
        }
        // A sentinel is consumed by the owning supervisor. Never signal a stored PID.
        let path = Path::new(&args[1]).with_extension("cancel");
        use std::io::Write;
        let id = state["job_id"].as_str().unwrap();
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(id.as_bytes())?;
                f.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                contract::regular_file(&path, 256)?;
                if fs::read_to_string(&path)? != id {
                    return Err(contract::Error::new(
                        "CANCEL_CONFLICT",
                        "Unrecognized cancellation sentinel",
                        3,
                    ));
                }
            }
            Err(e) => return Err(e.into()),
        }
        return Ok(json!({"success":true,"cancel_requested":true,"job_id":state["job_id"]}));
    }
    Err(contract::Error::new(
        "USAGE",
        "Use capabilities | doctor | schema | --version | probe INPUT | plan JOB.json | run JOB.json | batch BATCH.json | status STATE.json | cancel STATE.json",
        2,
    ))
}

fn validate_state(v: &Value) -> contract::Result<()> {
    if v["tool"] != "dcc-media-enhance"
        || v["schema_version"] != 1
        || v["job_id"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 128)
    {
        return Err(contract::Error::new(
            "INVALID_STATE",
            "Not a recognized dcc-media-enhance job sidecar",
            2,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod runtime_tests;
