use std::path::Path;

use ailoy::{
    agent::{Agent, AgentSpec},
    runenv::{RunEnv, SandboxConfig, VolumeMount},
};

pub const GUEST_ATTACHED_DIR: &str = "/workspace/attached";
pub const GUEST_SHARED_DIR: &str = "/workspace/shared";
pub const GUEST_ARTIFACTS_DIR: &str = "/workspace/artifacts";

const COWORKER_INSTRUCTION: &str = r#"You are {{NAME}}. Your primary role is to plan and perform tasks based on the user's query.

## System
- OS: {{OS}}
- You are running in a container environment.
- Internet access is available.

## Scripts
- You may write and execute a Python script to carry out the task.
- You can also obtain the information needed to perform a task by running a script.
- Prefer the available tools when they can accomplish the task.
- You are free to install and remove packages.

## Input files
- The user may mention files in the query outside the home directory.
- These files are reside in the `{{INPUTS}}` (input files) or `{{SHARED_DATA}}` (shared data files) directories.

## Artifacts
- Artifacts are output files produced by the task and shown to the user as the result.
- Artifacts must be placed under `{{ARTIFACTS}}`.
- Files outside of artifacts cannot be inspected by the user. Make sure every file you want to show is placed under `{{ARTIFACTS}}`.
- When referring to artifact files, you must use paths relative to `{{ARTIFACTS}}` (report.md, not {{ARTIFACTS}}/report.md).

## Others
- Current time: {{TIME}}
- Always respond in the language the user used."#;

#[derive(Default, Clone, Debug)]
pub struct CoworkerSandboxOptions {
    pub sandbox_name: Option<String>,
    pub persist: bool,
}

/// name: Identity of the model
/// model: Model to be used (e.g. openai/gpt-4.5)
pub async fn get_coworker_agent(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    input_dir: impl AsRef<Path>,
    shared_data_dir: impl AsRef<Path>,
    artifacts_dir: impl AsRef<Path>,
) -> anyhow::Result<Agent> {
    get_coworker_agent_with_opts(
        name,
        model,
        input_dir,
        shared_data_dir,
        artifacts_dir,
        CoworkerSandboxOptions::default(),
    )
    .await
}

pub async fn get_coworker_agent_with_opts(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    input_dir: impl AsRef<Path>,
    shared_data_dir: impl AsRef<Path>,
    artifacts_dir: impl AsRef<Path>,
    opts: CoworkerSandboxOptions,
) -> anyhow::Result<Agent> {
    /// Days since 1970-01-01 → (year, month, day). Howard Hinnant's `civil_from_days`.
    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        let y = if m <= 2 { y + 1 } else { y };
        (y, m, d)
    }

    /// UTC timestamp in ISO 8601 (`YYYY-MM-DDTHH:MM:SSZ`) using only stdlib.
    fn now_utc_iso8601() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400);
        let (y, mo, d) = civil_from_days(days);
        let h = sod / 3600;
        let mi = (sod % 3600) / 60;
        let s = sod % 60;
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    // Build instruction
    let mut config = SandboxConfig::default();
    config.name = opts.sandbox_name;
    config.persist = opts.persist;
    config.image = "brekkylab/agent-k:latest".into();
    config.cpus = 8;
    config.memory_mib = 1024;
    config.workdir = "/workspace".into();
    config.env.insert("HOME".into(), "/workspace".into());
    config.volumes.push(VolumeMount::Bind {
        host: input_dir.as_ref().into(),
        guest: GUEST_ATTACHED_DIR.into(),
        readonly: true,
    });
    config.volumes.push(VolumeMount::Bind {
        host: shared_data_dir.as_ref().into(),
        guest: GUEST_SHARED_DIR.into(),
        readonly: true,
    });
    config.volumes.push(VolumeMount::Bind {
        host: artifacts_dir.as_ref().into(),
        guest: GUEST_ARTIFACTS_DIR.into(),
        readonly: false,
    });
    let inst = COWORKER_INSTRUCTION
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &now_utc_iso8601())
        .replace("{{HOME}}", "/workspace")
        .replace("{{INPUTS}}", GUEST_ATTACHED_DIR)
        .replace("{{SHARED_DATA}}", GUEST_SHARED_DIR)
        .replace("{{ARTIFACTS}}", GUEST_ARTIFACTS_DIR)
        .replace("{{OS}}", "Debian GNU/Linux 13 (trixie)");

    let spec = AgentSpec::new(model.as_ref())
        .instruction(inst)
        .system_tools()
        .web_search_tool(vec![])
        .max_tokens(32_000);
    Agent::try_with_runenv(spec, RunEnv::sandbox(config).await?)
}
