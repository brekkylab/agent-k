use std::path::PathBuf;

use ailoy::message::{Message, Part, Role};

use super::Case;

/// Ten GAIA / BrowseComp-style multi-hop research queries spanning five
/// distinct domains and two difficulty levels. Each topic appears as an
/// English / Korean pair:
///
///   0/1 — space / astronomy, medium
///   2/3 — history, medium
///   4/5 — climate / energy, medium
///   6/7 — philosophy, hard
///   8/9 — cuisine / food, hard
pub fn get_deep_research_cases() -> Vec<Case> {
    vec![
        // Case 0 — space / astronomy, en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Which space agencies have successfully landed on the Moon since 2020, \
                 and what specific payload did each lander carry?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 1 — space / astronomy, ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "2020년 이후 달 착륙에 성공한 우주 기관들은 어디고, \
                 각 착륙선이 어떤 페이로드를 실었는지 정리해줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 2 — history, en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Which of the first ten US presidents (Washington through Tyler) owned \
                 slaves at the time of their inauguration? Cross-reference at least two sources.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 3 — history, ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "초대 미국 대통령부터 10대 대통령(워싱턴부터 타일러까지) 중에서 \
                 취임 시점에 노예를 소유하고 있던 대통령이 누구인지 \
                 최소 두 개의 출처를 교차 검증해서 정리해줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 4 — climate / energy, en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Compare LCOE (levelized cost of energy) trajectories from 2015 to 2024 \
                 for utility-scale solar, onshore wind, offshore wind, and natural gas \
                 combined cycle. Which one had the steepest absolute decline?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 5 — climate / energy, ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "유틸리티 규모 태양광, 육상 풍력, 해상 풍력, 천연가스 복합화력에 대해 \
                 2015년부터 2024년까지의 LCOE(균등화 발전 원가) 추이를 비교하고, \
                 어떤 기술의 절대 하락폭이 가장 컸는지 정리해줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 6 — philosophy, en, hard
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Compare Derek Parfit's view of personal identity in 'Reasons and Persons' \
                 (1984) with the bundle theory associated with David Hume. In what specific \
                 respect does Parfit go beyond Hume, and where does he agree with Buddhist anatta?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 7 — philosophy, ko, hard
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Derek Parfit이 '이유와 인격(Reasons and Persons, 1984)'에서 제시한 \
                 인격 동일성 견해를 David Hume의 다발 이론(bundle theory)과 비교해줘. \
                 Parfit이 Hume보다 어느 지점에서 더 멀리 갔는지, 그리고 불교의 무아(anātman) \
                 개념과 정확히 어디서 겹치는지 분리해서 정리해줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 8 — cuisine / food, en, hard
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Compare the traditional preparation of three fermented soybean products: \
                 Korean doenjang, Japanese miso, and Chinese douchi. Which microorganisms \
                 are dominant in each, and how do salt content and fermentation duration differ?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 9 — cuisine / food, ko, hard
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "전통 발효 콩 식품 세 가지 — 한국 된장, 일본 미소, 중국 두시 — 의 \
                 전통적 제조 과정을 비교해줘. 각각의 우점 미생물이 무엇이고 \
                 염도와 발효 기간이 어떻게 다른지 정리해줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: Vec::new(),
        },
        // Case 10 — delegation (en): a real research-and-write task whose own
        // baseline numbers live in the project's documents. Deep Research does
        // the web research itself but must pull our library's figures from the
        // corpus (via the `subagent_speedwagon` sub-agent) and fold that corpus
        // citation into its `citations.json` numbering alongside the web sources.
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Write a short briefing on how public libraries are adapting to \
                 e-book demand, and benchmark our own library's collection size and \
                 annual circulation against the trends you find.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![(
                include_bytes!("maple_library_guide.md").to_vec(),
                PathBuf::from("knowledge/maple_library_guide.md"),
            )],
        },
        // Case 11 — delegation (ko): same shape, Korean corpus + research task.
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "국내 생활협동조합 시장이 요즘 어떻게 돌아가는지 조사해서 짧은 \
                 보고서로 써주고, 우리 조합의 조합원 수랑 작년 매출이 업계에서 \
                 어느 정도 위치인지도 같이 짚어줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![(
                include_bytes!("haneul_coop_rules.md").to_vec(),
                PathBuf::from("knowledge/haneul_coop_rules.md"),
            )],
        },
    ]
}
