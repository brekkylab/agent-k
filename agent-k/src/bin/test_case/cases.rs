use std::path::PathBuf;

use ailoy::message::{Message, Part, Role};

pub struct Case {
    pub query: Message,
    pub files: Vec<(Vec<u8>, PathBuf)>,
}

pub fn get_coworker_cases() -> Vec<Case> {
    vec![
        // Case 0
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Make an HTML page that shows the current weather of major cities around the world",
            )]),
            files: Vec::new(),
        },
        // Case 1
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "세계 주요 도시의 현재 날씨를 보여주는 HTML 페이지 만들어줘",
            )]),
            files: Vec::new(),
        },
        // Case 2
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Split payslip document into separate single-page PDFs.",
            )]),
            files: vec![(
                include_bytes!("payslips.pdf").to_vec(),
                PathBuf::from("payslips.pdf"),
            )],
        },
        // Case 3
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "급여명세서 문서를 각각 한 페이지짜리 PDF로 분리해주세요.",
            )]),
            files: vec![(
                include_bytes!("payslips.pdf").to_vec(),
                PathBuf::from("payslips.pdf"),
            )],
        },
    ]
}
