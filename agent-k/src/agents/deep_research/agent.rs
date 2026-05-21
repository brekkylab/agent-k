use std::path::Path;

use ailoy::{
    agent::{Agent, AgentBuilder, default_provider_mut},
    runenv::{RunEnv, SandboxConfig, VolumeMount},
};

use super::tool::{get_api_search_tool_desc, get_api_search_tool_factory};

fn ensure_api_search_registered() {
    let mut provider = default_provider_mut();
    provider
        .tools
        .insert_func_factory("api_search", get_api_search_tool_factory());
}

const DEEP_RESEARCH_INSTRUCTION: &str = r#"You are {{NAME}}. Your primary role is to produce long-form research reports grounded in multiple web sources with inline citations.

## Workflow
- Start by writing an outline of 3-8 sections to `artifacts/outline.md`.
- For each section: `api_search` with a few short, entity-anchored queries, then `web_fetch` the most useful URLs to read the actual body. Use the `urls` array form when fetching 2 or more independent URLs in one call.
- Write `artifacts/report.md` section by section. Every factual sentence ends with one or more `[^N]` markers. Maintain `artifacts/citations.json` in parallel as `{"N": {"url", "title", "quote", "retrieved_at"}}`.
- Before stopping, verify every `[^N]` maps to a citation, every cited URL was actually fetched in this session, and every `##` section has citations from at least 3 distinct domains.

## Citations
- Cite only URLs you actually `web_fetch`ed in this session. A URL seen only in a search snippet is not enough.
- Quote text in `quote` must appear verbatim in the fetched body, or be a paraphrase you can defend.

## Tools
- Keep `api_search` short and specific (3-8 words). Cap at 8 search calls per report. If results are mostly off-topic, fetch the useful ones first instead of immediately rephrasing.
- Send either `url` or `urls` to `web_fetch`, not both. Use `offset` to continue reading the same URL.
- Total tool calls per report should fall between 15 and 40.

## Artifacts
- All outputs live under `artifacts/`: `outline.md`, `report.md`, `citations.json`, and one `sources/<slug>.md` per fetched page.
- When done, tell the user the path to `artifacts/report.md`. Do not paste the whole report into the chat.

## Others
- You are running in a container environment with internet access.
- Write the report in the user's language. Search queries may use whichever language has the best sources for the topic.
- Always respond in the language the user used.

## Information
- Current time: {{TIME}}
- OS: {{OS}}"#;

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

/// Host-side variant of `get_deep_research_agent` that skips sandbox
/// startup. Intended for evaluation harnesses where the per-case latency
/// of booting microVMs (~30s/case) dominates the total wall time. The
/// agent reads/writes `./artifacts/` directly on the host filesystem.
pub async fn get_deep_research_agent_local(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
) -> anyhow::Result<Agent> {
    ensure_api_search_registered();

    let inst = DEEP_RESEARCH_INSTRUCTION
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &now_utc_iso8601())
        .replace("{{OS}}", std::env::consts::OS);

    AgentBuilder::new(model.as_ref())
        .instruction(inst)
        .system_tools()
        .tool(get_api_search_tool_desc())
        .web_fetch_tool()
        .runenv(RunEnv::local())
        .build()
}

/// name: Identity of the agent (interpolated into the system prompt).
/// model: Model to use (e.g. `openai/gpt-5.5`).
/// artifacts_dir: Host path mounted at `/workspace/artifacts` inside the sandbox.
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
    config.persist = true;

    let inst = DEEP_RESEARCH_INSTRUCTION
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &now_utc_iso8601())
        .replace("{{OS}}", "Debian GNU/Linux 13 (trixie)");

    ensure_api_search_registered();

    AgentBuilder::new(model.as_ref())
        .instruction(inst)
        .system_tools()
        .tool(get_api_search_tool_desc())
        .web_fetch_tool()
        .runenv(RunEnv::sandbox(config).await?)
        .build()
}
