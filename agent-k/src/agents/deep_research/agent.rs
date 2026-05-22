use std::path::Path;

use ailoy::{
    agent::{Agent, AgentSpec, default_provider_mut},
    runenv::{RunEnv, SandboxConfig, VolumeMount},
};

use super::tool::{get_api_search_tool_desc, get_api_search_tool_factory};

const DEEP_RESEARCH_INSTRUCTION: &str = r#"You are {{NAME}}. Your primary role is to produce long-form research reports grounded in multiple web sources with inline citations.

## Workflow
- Start by writing an outline of 3-8 sections to `artifacts/outline.md`.
- For each section: `api_search` with a few short, entity-anchored queries, then `web_fetch` the most useful URLs to read the actual body.
- Write `artifacts/report.md` one section at a time. Every factual sentence ends with one or more `[^N]` markers. Maintain `artifacts/citations.json` in parallel as `{"N": {"url", "title", "quote", "retrieved_at"}}`.
- Before stopping, verify every `[^N]` maps to a citation, every cited URL was actually fetched in this session, and every `##` section has citations from at least 3 distinct domains.

## Parallel tool calls (important)
- When you decide you need N independent pieces of information at the same point, issue them as a **single batched tool_calls block** — N tool calls fired in parallel, results return together.
- Concretely: if a section needs queries about three entities, fire three `api_search` calls in one batch, not three turns. If you have a list of 2-5 URLs to read, fire one `web_fetch` with `urls: [...]` (the array form) instead of one call per URL.
- Sequential is correct only when later calls genuinely depend on earlier results.

## Editing discipline
- Write each section once, completely, then move on. Do not iteratively edit prose you already wrote — small wording fixes are not worth the round trips.
- Use `edit` only for (a) inserting a new section into `report.md`, (b) updating `citations.json`, or (c) fixing a specific objective error (wrong citation index, malformed JSON). Cosmetic rewrites are off-limits.

## Citations
- Cite only URLs you actually `web_fetch`ed in this session. A URL seen only in a search snippet is not enough.
- Quote text in `quote` must appear verbatim in the fetched body, or be a paraphrase you can defend.

## Tools
- Keep `api_search` short and specific (3-8 words). Cap at 8 search calls per report.
- Send either `url` or `urls` to `web_fetch`, not both. Use `offset` to continue reading the same URL.
- Total tool calls per report must stay between 15 and 32 — if you find yourself approaching 32, write what you have, do the final verification pass, and stop.

## Artifacts
- All outputs live under `artifacts/`: `outline.md`, `report.md`, `citations.json`, and one `sources/<slug>.md` per fetched page.
- When done, tell the user the path to `artifacts/report.md`. Do not paste the whole report into the chat.

## Language
- Write the final report and your reply in the language the user used.
- Search queries should be in the language with the best sources for the topic. For technical, scientific, historical, or international topics the best sources are usually English. When the user asks in Korean about such topics, still search in English, read English sources, and translate into Korean only at the writing step.

## Others
- You are running in a container environment with internet access.

## Information
- Current time: {{TIME}}
- OS: {{OS}}"#;

fn ensure_api_search_registered() {
    let mut provider = default_provider_mut();
    provider
        .tools
        .insert_func_factory("api_search", get_api_search_tool_factory());
}

// Howard Hinnant's civil_from_days: days since 1970-01-01 → (year, month, day).
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

/// `artifacts_dir` is bind-mounted into the sandbox at `/workspace/artifacts`,
/// which is where the prompt instructs the model to write its outputs.
pub async fn get_deep_research_agent(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    artifacts_dir: impl AsRef<Path>,
) -> anyhow::Result<Agent> {
    let mut config = SandboxConfig::default();
    config.image = "brekkylab/agent-k:latest".into();
    config.cpus = 8;
    config.memory_mib = 1024;
    config.workdir = "/workspace".into();
    config.env.insert("HOME".into(), "/workspace".into());
    config.volumes.push(VolumeMount::Bind {
        host: artifacts_dir.as_ref().into(),
        guest: "/workspace/artifacts".into(),
        readonly: false,
    });

    let inst = DEEP_RESEARCH_INSTRUCTION
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &now_utc_iso8601())
        .replace("{{OS}}", "Debian GNU/Linux 13 (trixie)");

    ensure_api_search_registered();

    let spec = AgentSpec::new(model.as_ref())
        .instruction(inst)
        .system_tools()
        .tool(get_api_search_tool_desc())
        .web_fetch_tool()
        .max_tokens(32_000);
    Agent::try_with_runenv(spec, RunEnv::sandbox(config).await?)
}
