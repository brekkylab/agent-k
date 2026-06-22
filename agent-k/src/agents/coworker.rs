use std::path::{Path, PathBuf};

use ailoy::{
    agent::{Agent, AgentSpec, default_provider},
    runenv::{FileEntry, RunEnv, SandboxConfig, VolumeMount},
};

use crate::agents::speedwagon::{
    SPEEDWAGON_DELEGATION_NOTE, register_corpus_tools, speedwagon_subagent_spec,
};
use crate::knowledge_base::SharedStore;

const XLSX_SKILL_DIR: &str = "/workspace/skills/xlsx";
pub const GUEST_ATTACHED_DIR: &str = "/workspace/attached";
pub const GUEST_SHARED_DIR: &str = "/workspace/shared";
pub const GUEST_ARTIFACTS_DIR: &str = "/workspace/artifacts";
pub const PPTX_SKILL_DIR: &str = "/workspace/skills/pptx";

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

## Skills
- An "Available Skills" table is appended to this system prompt. It lists every loaded skill with the absolute path to its `SKILL.md`.
- For any task whose domain matches an entry in that table, your FIRST action MUST be to read the SKILL.md at the listed path — either via `cat <SKILL.md path>` or the `read` tool — and you must follow that file literally, including every "DO NOT" rule.
- **The first read of a SKILL.md must fetch the whole file** — use `cat <SKILL.md path>` via shell, or call the `read` tool with only the `path` argument (omit `offset` and `limit`). Partial reads skip rules near the end of the file.
- Inside a skill directory, the ONLY path you may pass to the `read` tool (or `cat`) is the `SKILL.md`. Do not `read` supporting files (scripts, data, etc.) — even partial / offset+limit reads are forbidden — unless the SKILL.md explicitly directs you to.

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

#[derive(Default, Clone)]
pub struct CoworkerSandboxOptions {
    pub sandbox_name: Option<String>,
    pub persist: bool,
    /// When true, the bundled PPTX and XLSX skills are materialised under
    /// [`PPTX_SKILL_DIR`] / [`XLSX_SKILL_DIR`] and surfaced via the
    /// auto-rendered "Available Skills" table. Default `false`; the CLI
    /// wrappers (`run`, `test_case`) flip this on explicitly.
    pub with_skill: bool,
    /// When set, a Speedwagon sub-agent (`subagent_speedwagon`) bound to this
    /// document store is attached, letting Coworker delegate corpus questions.
    pub corpus_store: Option<SharedStore>,
    /// Model for the Speedwagon sub-agent. `None` inherits Coworker's own model;
    /// set it to the corpus-recommended model so a parent on a model that is
    /// poor for the corpus loop doesn't drag the sub-agent down.
    pub corpus_model: Option<String>,
}

/// name: Identity of the model
/// model: Model to be used (e.g. openai/gpt-4.5)
pub async fn get_coworker_agent(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    input_dir: impl AsRef<Path>,
    shared_data_dir: impl AsRef<Path>,
    artifacts_dir: impl AsRef<Path>,
    with_skill: bool,
) -> anyhow::Result<Agent> {
    get_coworker_agent_with_opts(
        name,
        model,
        input_dir,
        shared_data_dir,
        artifacts_dir,
        CoworkerSandboxOptions {
            with_skill,
            ..Default::default()
        },
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
    config.image = "brekkylab/agent-k-libreoffice:latest".into();
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

    let mut spec = AgentSpec::new(model.as_ref())
        .instruction(inst.clone())
        .system_tools()
        .web_search_tool(vec![])
        .max_tokens(32_000);
    if opts.with_skill {
        let pptx_dir = PathBuf::from(PPTX_SKILL_DIR);
        let xlsx_dir = PathBuf::from(XLSX_SKILL_DIR);
        spec = spec
            .skill(
                &pptx_dir,
                [
                    FileEntry::new(
                        pptx_dir.join("SKILL.md"),
                        include_bytes!("skill/pptx/SKILL.md").to_vec(),
                    ),
                    FileEntry::new(
                        pptx_dir.join("script/verify_pptx.py"),
                        include_bytes!("skill/pptx/script/verify_pptx.py").to_vec(),
                    ),
                    FileEntry::new(
                        pptx_dir.join("script/html2pptx.py"),
                        include_bytes!("skill/pptx/script/html2pptx.py").to_vec(),
                    ),
                    FileEntry::new(
                        pptx_dir.join("script/components.css"),
                        include_bytes!("skill/pptx/script/components.css").to_vec(),
                    ),
                    FileEntry::new(
                        pptx_dir.join("script/template.html"),
                        include_bytes!("skill/pptx/script/template.html").to_vec(),
                    ),
                    FileEntry::new(
                        pptx_dir.join("script/contact_sheet.py"),
                        include_bytes!("skill/pptx/script/contact_sheet.py").to_vec(),
                    ),
                ],
            )
            .skill(
                &xlsx_dir,
                [
                    FileEntry::new(
                        xlsx_dir.join("SKILL.md"),
                        include_bytes!("skill/xlsx/SKILL.md").to_vec(),
                    ),
                    FileEntry::new(
                        xlsx_dir.join("xlsx_skill.py"),
                        include_bytes!("skill/xlsx/script/xlsx_skill.py").to_vec(),
                    ),
                ],
            );
    }
    let runenv = RunEnv::sandbox(config).await?;
    match opts.corpus_store {
        // A corpus store lets Coworker delegate document questions to a
        // Speedwagon sub-agent. The sub-agent's corpus tools resolve against
        // this provider, so register them here alongside the default tools.
        Some(store) => {
            let mut provider = default_provider().clone();
            register_corpus_tools(&mut provider.tools, store);
            let sub_model = opts.corpus_model.as_deref().unwrap_or(model.as_ref());
            let spec = spec
                .instruction(format!("{inst}{SPEEDWAGON_DELEGATION_NOTE}"))
                .subagent(speedwagon_subagent_spec(name.as_ref(), sub_model));
            Agent::try_with_provider_and_runenv(spec, &provider, runenv)
        }
        None => Agent::try_with_runenv(spec, runenv),
    }
}
