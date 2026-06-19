use std::path::{Path, PathBuf};

use ailoy::{
    agent::AgentSpec,
    runenv::{FileEntry, Sandbox, SandboxBuilder, VolumeMount},
};

const XLSX_SKILL_DIR: &str = "/root/skills/xlsx";
pub const GUEST_ATTACHED_DIR: &str = "/root/attached";
pub const GUEST_SHARED_DIR: &str = "/root/shared";
pub const GUEST_ARTIFACTS_DIR: &str = "/root/artifacts";
pub const PPTX_SKILL_DIR: &str = "/root/skills/pptx";

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
- These files are reside in the `{{ATTACHMENTS}}` (attachment files) or `{{SHARED_DATA}}` (shared data files) directories.

## Artifacts
- Artifacts are output files produced by the task and shown to the user as the result.
- Artifacts must be placed under `{{ARTIFACTS}}`.
- Files outside of artifacts cannot be inspected by the user. Make sure every file you want to show is placed under `{{ARTIFACTS}}`.
- When referring to artifact files, you must use paths relative to `{{ARTIFACTS}}` (report.md, not {{ARTIFACTS}}/report.md).

## Others
- Current time: {{TIME}}
- Always respond in the language the user used."#;

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

/// name: Identity of the model
/// model: Model to be used (e.g. openai/gpt-4.5)
pub fn get_coworker_agent_spec(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    with_skill: bool,
) -> AgentSpec {
    let inst = COWORKER_INSTRUCTION
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &now_utc_iso8601())
        .replace("{{HOME}}", "/root")
        .replace("{{ATTACHMENTS}}", GUEST_ATTACHED_DIR)
        .replace("{{SHARED_DATA}}", GUEST_SHARED_DIR)
        .replace("{{ARTIFACTS}}", GUEST_ARTIFACTS_DIR)
        .replace("{{OS}}", "Debian GNU/Linux 13 (trixie)");

    let mut spec = AgentSpec::new(model.as_ref())
        .instruction(inst)
        .system_tools()
        .web_search_tool(vec![])
        .max_tokens(32_000);
    if with_skill {
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
    spec
}

pub async fn get_coworker_agent_runenv(
    input_dir: impl AsRef<Path>,
    shared_data_dir: impl AsRef<Path>,
    artifacts_dir: impl AsRef<Path>,
) -> anyhow::Result<Sandbox> {
    SandboxBuilder::new()
        .image("brekkylab/agent-k-libreoffice:latest")
        .cpus(8)
        .memory_mib(1024)
        .mount(VolumeMount::Bind {
            host: input_dir.as_ref().to_path_buf(),
            guest: GUEST_ATTACHED_DIR.to_string(),
            readonly: false,
        })
        .mount(VolumeMount::Bind {
            host: shared_data_dir.as_ref().to_path_buf(),
            guest: GUEST_SHARED_DIR.to_string(),
            readonly: true,
        })
        .mount(VolumeMount::Bind {
            host: artifacts_dir.as_ref().to_path_buf(),
            guest: GUEST_ARTIFACTS_DIR.to_string(),
            readonly: false,
        })
        .build()
        .await
}
