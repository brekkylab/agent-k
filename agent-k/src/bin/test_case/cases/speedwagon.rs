use std::path::PathBuf;

use ailoy::message::{Message, Part, Role};

use super::Case;

/// Speedwagon corpus-QA cases. Each carries `corpus_files` (indexed into the
/// `knowledge` folder) and a question over them. Domains and languages are
/// mixed; answers should cite with `[^N]` + a `## Sources` block, and an
/// unanswerable question should decline with no citation.
///
///   0/1 — single corpus fact (en library / ko coop)
///   2/3 — a number from the corpus (en / ko)
///   4/5 — cross-document comparison, two docs cited (en / ko)
///   6/7 — fact present in a reused business doc (en market / ko 실적)
///   8/9 — unanswerable: not in the corpus, expect a decline with no citation (en / ko)
pub fn get_speedwagon_cases() -> Vec<Case> {
    let library = || {
        (
            include_bytes!("maple_library_guide.md").to_vec(),
            PathBuf::from("knowledge/maple_library_guide.md"),
        )
    };
    let coop = || {
        (
            include_bytes!("haneul_coop_rules.md").to_vec(),
            PathBuf::from("knowledge/haneul_coop_rules.md"),
        )
    };
    let market = || {
        (
            include_bytes!("market_research.txt").to_vec(),
            PathBuf::from("knowledge/market_research.txt"),
        )
    };
    let report_ko = || {
        (
            include_bytes!("2026_1분기_실적보고서.txt").to_vec(),
            PathBuf::from("knowledge/2026_1분기_실적보고서.txt"),
        )
    };

    vec![
        // Case 0 — single fact, en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "What is the library mascot, and when did Maplewood Public Library open?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![library()],
        },
        // Case 1 — single fact, ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "하늘협동조합의 상징 동물은 무엇이고, 본점은 어디에 있어?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![coop()],
        },
        // Case 2 — a number, en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "What was the library's 2025 operating budget, and how many visits did it record that year?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![library()],
        },
        // Case 3 — a number, ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "하늘협동조합의 2025년 총매출과 배당률은 얼마였어?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![coop()],
        },
        // Case 4 — cross-document comparison (two docs), en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Between Maplewood Library and Haneul Co-op, which one was founded earlier?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![library(), coop()],
        },
        // Case 5 — cross-document comparison (two docs), ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "메이플우드 도서관이랑 하늘협동조합 중에 더 늦게 생긴 데가 어디야?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![library(), coop()],
        },
        // Case 6 — fact from a reused business doc, en
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Summarize the key market findings in two sentences.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![market()],
        },
        // Case 7 — fact from a reused business doc, ko
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "2026년 1분기 실적 핵심만 두세 문장으로 정리해줘.",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![report_ko()],
        },
        // Case 8 — unanswerable: not in the corpus, en (expect decline, no citation)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "What is the library director's home phone number?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![library()],
        },
        // Case 9 — unanswerable: not in the corpus, ko (expect decline, no citation)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "하늘협동조합의 주차장 운영 정책은 어떻게 돼?",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
            corpus_files: vec![coop()],
        },
    ]
}
