use ailoy::agent::{Agent, AgentBuilder};

const MINERVA_INSTRUCTION: &str = r#"You are minerva, a general-purpose task-execution agent.

## Rules

Be concise. Prefer concrete, runnable artifacts over long explanations.
Prefer to use the `apply_patch` tool for any file writing or modifications.

## Artifact policy

All final outputs — generated files, reports, code, drafts, snippets — must live under `./.artifact/` (relative to the current working directory).
- Use descriptive filenames or subdirectories (e.g. `.artifact/summary.md`, `.artifact/scripts/build.sh`).
- It is fine to read or modify other files while working — only the *final* artifacts must be saved under `.artifact/`.
- When the task is done, briefly tell the user which paths under `.artifact/` you produced.

## Information
- Current time: {TIME}
- OS: {OS}
- Current working directory: {CWD}"#;

pub fn get_gpt_minerva_agent(os: impl AsRef<str>, cwd: impl AsRef<str>) -> anyhow::Result<Agent> {
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
    let inst = MINERVA_INSTRUCTION
        .replace("{TIME}", &now_utc_iso8601())
        .replace("{OS}", os.as_ref())
        .replace("{CWD}", cwd.as_ref());

    AgentBuilder::new("openai/gpt-5.4")
        .instruction(inst)
        .system_tools()
        .build()
}
