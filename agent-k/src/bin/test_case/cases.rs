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
                "급여명세서 문서를 각각 한 페이지짜리 PDF들로 분리해주세요.",
            )]),
            files: vec![(
                include_bytes!("payslips.pdf").to_vec(),
                PathBuf::from("payslips.pdf"),
            )],
        },
        // Case 4
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Visualize co2.csv as a journal-submission-ready figure.",
            )]),
            files: vec![(
                include_bytes!("co2.csv").to_vec(),
                PathBuf::from("co2.csv"),
            )],
        },
        // Case 5
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "co2.csv 를 저널 투고용 figure로 시각화해줘",
            )]),
            files: vec![(
                include_bytes!("co2.csv").to_vec(),
                PathBuf::from("co2.csv"),
            )],
        },
        // Case 6
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Extract information from the following two Korean receipt images, save it as CSV, and visualize the extracted results in a single HTML page.\n\
                 Images: receipt_1.png, receipt_2.png\n\
                 CSV columns (header included, one row per image):\n\
                 image,MerchantName,MerchantAddress,MerchantPhoneNumber,TransactionDate,TransactionTime,PaymentDate,ReceiptNumber,Subtotal,TotalTax,Total\n\
                 Requirements:\n\
                 - The image column is the file stem (e.g. receipt_1)\n\
                 - Output files: rows.csv (1 header line + 2 data lines), report.html (CSV contents shown as a table)\n\
                 - One-line summary in English of how the extraction was done",
            )]),
            files: vec![
                (
                    include_bytes!("receipt_1.png").to_vec(),
                    PathBuf::from("receipt_1.png"),
                ),
                (
                    include_bytes!("receipt_2.png").to_vec(),
                    PathBuf::from("receipt_2.png"),
                ),
            ],
        },
        // Case 7
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "다음 두 한국 영수증 이미지에서 정보를 추출해 CSV로 저장하고, 추출 결과를 한 페이지 HTML로 시각화해줘.\n\
                 이미지: receipt_1.png, receipt_2.png\n\
                 CSV 컬럼 (헤더 포함, 이미지당 1행):\n\
                 image,MerchantName,MerchantAddress,MerchantPhoneNumber,TransactionDate,TransactionTime,PaymentDate,ReceiptNumber,Subtotal,TotalTax,Total\n\
                 요구사항:\n\
                 - image 컬럼은 파일 stem (예: receipt_1)\n\
                 - 결과 파일: rows.csv (헤더 1줄 + 데이터 2줄), report.html (CSV 내용 표로 표시)\n\
                 - 한 줄로 어떻게 추출했는지 한국어 요약",
            )]),
            files: vec![
                (
                    include_bytes!("receipt_1.png").to_vec(),
                    PathBuf::from("receipt_1.png"),
                ),
                (
                    include_bytes!("receipt_2.png").to_vec(),
                    PathBuf::from("receipt_2.png"),
                ),
            ],
        },
    ]
}
