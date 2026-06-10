use std::path::Path;

use ailoy::{
    agent::{Agent, AgentSpec, default_provider, default_provider_mut},
    runenv::{RunEnv, SandboxConfig, VolumeMount},
};

use super::tool::{get_api_search_tool_desc, get_api_search_tool_factory};
use crate::agents::speedwagon::{
    SPEEDWAGON_DELEGATION_NOTE_DEEP_RESEARCH, register_corpus_tools, speedwagon_subagent_spec,
};
use crate::knowledge_base::SharedStore;

const DEEP_RESEARCH_INSTRUCTION: &str = r#"You are {{NAME}}. Your primary role is to produce long-form research reports grounded in multiple web sources with inline citations.

## Workflow
1. Outline first: write an outline of 3-8 sections to `artifacts/outline.md`.
2. Research phase: for each section, do `api_search` then `web_fetch`. Build up `artifacts/citations.json` as you go (`{"N": {"url", "title", "quote", "retrieved_at"}}`). Do not touch `artifacts/report.md` yet.
3. Writing phase: when research is done, write the whole `artifacts/report.md` in one `write` call. Every factual sentence ends with one or more `[^N]` markers.
4. Verify phase: confirm every `[^N]` maps to a citation, every cited URL was actually fetched in this session, and every `##` section has citations from at least 3 distinct domains.

## Parallel tool calls
Whenever you need N independent pieces of information at the same point, fire all N as one batched `tool_calls` block (results return together) instead of N sequential turns:
- 3 entities to research → three `api_search` calls in one batch.
- 4 URLs to read → four `web_fetch` calls in one batch (one URL per call).
Sequential is correct only when a later call genuinely depends on an earlier result (e.g. you need a URL from a search before fetching it).

## Citations
- Cite only URLs you actually `web_fetch`ed in this session. A URL seen only in a search snippet is not enough.
- Quote text in `quote` must appear verbatim in the fetched body, or be a paraphrase you can defend.

## Tool budget
- `api_search`: ≤ 8 calls per report. Keep queries short and specific (3-8 words).
- `web_fetch`: one `url` per call (no array form). Call as many times as you need — fetching more sources is good, just batch the parallel ones into a single tool_calls block. Use `offset` to continue reading the same URL.
- `write`: ≤ 2 calls for `artifacts/report.md` (the writing-phase call, plus at most one corrective rewrite).
- `edit`: only for `artifacts/citations.json` JSON updates and fixing objective errors (wrong citation index, malformed JSON). Do not use `edit` for prose changes in `report.md`; if `report.md` needs a meaningful change, do a single corrective `write`.
- Hard cap: 32 total tool calls. If you are approaching it, run the verify phase and stop.

## Artifacts
- All outputs live under `artifacts/`: `outline.md`, `report.md`, `citations.json`, and one `sources/<slug>.md` per fetched page.
- When done, tell the user the path to `artifacts/report.md`. Do not paste the whole report into the chat.

## Language
- Write the final report and your reply in the language the user used.
- Search queries should be in the language with the best sources for the topic. For technical, scientific, historical, or international topics the best sources are usually English. When the user asks in Korean about such topics, still search in English, read English sources, and translate into Korean only at the writing step.

## Others
- You are running in a container environment with internet access.

## Information
- Current time: {{TIME}}"#;

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
    corpus_store: Option<SharedStore>,
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
        .replace("{{TIME}}", &now_utc_iso8601());

    ensure_api_search_registered();

    let spec = AgentSpec::new(model.as_ref())
        .instruction(inst.clone())
        .system_tools()
        .tool(get_api_search_tool_desc())
        .web_fetch_tool()
        .max_tokens(32_000);

    let runenv = RunEnv::sandbox(config).await?;
    match corpus_store {
        // A corpus store lets Deep Research delegate document questions to a
        // Speedwagon sub-agent whose corpus tools resolve against this provider.
        Some(store) => {
            let mut provider = default_provider().clone();
            register_corpus_tools(&mut provider.tools, store);
            let spec = spec
                .instruction(format!("{inst}{SPEEDWAGON_DELEGATION_NOTE_DEEP_RESEARCH}"))
                .subagent(speedwagon_subagent_spec(name.as_ref(), model.as_ref()));
            Agent::try_with_provider_and_runenv(spec, &provider, runenv)
        }
        None => Agent::try_with_runenv(spec, runenv),
    }
}
