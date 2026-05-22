use std::path::PathBuf;

use ailoy::message::{Message, Part, Role};

pub struct Case {
    pub query: Message,
    pub files: Vec<(Vec<u8>, PathBuf)>,
    pub shared_files: Vec<(Vec<u8>, PathBuf)>,
}

pub fn get_coworker_cases() -> Vec<Case> {
    vec![
        // Case 0
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Make an HTML page that shows the current weather of major cities around the world",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
        },
        // Case 1
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "세계 주요 도시의 현재 날씨를 보여주는 HTML 페이지 만들어줘",
            )]),
            files: Vec::new(),
            shared_files: Vec::new(),
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
            shared_files: Vec::new(),
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
            shared_files: Vec::new(),
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
            shared_files: Vec::new(),
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
            shared_files: Vec::new(),
        },
        // Case 6
        Case {
            query: Message::new(Role::User).with_contents([
                Part::text(
                    "Extract information from the following two receipt images, save it as CSV, and visualize the extracted results in a single HTML page.\n\
                    CSV columns (header included, one row per image):\n\
                    image,MerchantName,MerchantAddress,MerchantPhoneNumber,TransactionDate,TransactionTime,PaymentDate,ReceiptNumber,Subtotal,TotalTax,Total\n\
                    Requirements:\n\
                    - The image column is the file stem (e.g. receipt_1)\n\
                    - Output files: rows.csv (1 header line + 2 data lines), report.html (CSV contents shown as a table)\n\
                    - One-line summary in English of how the extraction was done",
                ),
                Part::image_embedded("image/png", include_bytes!("receipt_1.png").to_vec().into()).unwrap(),
                Part::image_embedded("image/png", include_bytes!("receipt_2.png").to_vec().into()).unwrap()
            ]),
            files: vec![],
            shared_files: Vec::new(),
        },
        // Case 7
        Case {
            query: Message::new(Role::User).with_contents([
                Part::text(
                    "다음 두 영수증 이미지에서 정보를 추출해 CSV로 저장하고, 추출 결과를 한 페이지 HTML로 시각화해줘.\n\
                     CSV 컬럼 (헤더 포함, 이미지당 1행):\n\
                     image,MerchantName,MerchantAddress,MerchantPhoneNumber,TransactionDate,TransactionTime,PaymentDate,ReceiptNumber,Subtotal,TotalTax,Total\n\
                     요구사항:\n\
                     - image 컬럼은 첫 번째 이미지에 대해 receipt_1, 두 번째 이미지에 대해 receipt_2\n\
                     - 결과 파일: rows.csv (헤더 1줄 + 데이터 2줄), report.html (CSV 내용 표로 표시)\n\
                     - 한 줄로 어떻게 추출했는지 한국어 요약",
                ),
                Part::image_embedded("image/png", include_bytes!("receipt_1.png").to_vec().into()).unwrap(),
                Part::image_embedded("image/png", include_bytes!("receipt_2.png").to_vec().into()).unwrap(),
            ]),
            files: Vec::new(),
            shared_files: Vec::new(),
        },
        // Case 8
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "tax_invoice.jpg와 동일한 레이아웃과 내용의 pdf 문서를 만들어 주세요.",
            )]),
            files: vec![(
                include_bytes!("tax_invoice.jpg").to_vec(),
                PathBuf::from("tax_invoice.jpg"),
            )],
            shared_files: Vec::new(),
        },
        // Case 9
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Create a properly formatted NDA docx file between Ito Satoshi and Aperture Laboratories. Use the template in nda.md. The logo watermark should be attached start of the document, right banner.",
            )]),
            files: vec![
                (
                    include_bytes!("nda.md").to_vec(),
                    PathBuf::from("nda.md"),
                ),
                (
                    include_bytes!("image.png").to_vec(),
                    PathBuf::from("logo.png"),
                )
            ],
            shared_files: Vec::new(),
        },
        // Case 10 — pptx create (source: comirnaty0.1mg.txt)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Using artifacts/comirnaty0.1mg.txt as the source content, \
                 produce a presentation and save it to \
                 artifacts/result/comirnaty_brochure.pptx (create the \
                 artifacts/result/ directory if it does not exist). Summarize \
                 the brochure for a clinical audience. Preserve the original \
                 Korean text. Include a title slide, an agenda slide, one \
                 slide per major section (composition, dosing schedule, \
                 booster, storage, etc.), and a closing slide. Use a \
                 consistent theme, readable font sizes, and bullet points \
                 rather than walls of text. \
                 (Environment: any attached files are already at artifacts/; \
                 put helper scripts in the working directory, not /tmp.)",
            )]),
            files: vec![(
                include_bytes!("comirnaty0.1mg.txt").to_vec(),
                PathBuf::from("comirnaty0.1mg.txt"),
            )],
            shared_files: Vec::new(),
        },
        // Case 11 — pptx edit (source: comirnaty_deck gpt5.5 skills.pptx)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "Open artifacts/slides.pptx and make four edits: update the \
                 title slide subtitle to today's date in ISO format; append a \
                 new closing slide titled \"Thank You\" with a single centered \
                 line \"Questions?\"; change every slide's footer to \
                 \"Confidential — Internal Use Only\"; and on slide 3 remove \
                 the two red-arrow + small-blue-rectangle pairs that sit just \
                 below the red circles labeled \"1차\" and \"2차\". Each pair \
                 consists of a small blue rectangle with a red arrow attached \
                 to it — both shapes in each pair are visually out of place, \
                 so remove all four shapes (two red arrows + two blue \
                 rectangles) total. Apply the same removal to slide 4 if \
                 similarly out-of-place arrow/rectangle pairs exist. Save the \
                 result as \
                 artifacts/slides_edited.pptx and leave the original \
                 untouched. \
                 (Environment: any attached files are already at artifacts/; \
                 put helper scripts in the working directory, not /tmp.)",
            )]),
            files: vec![(
                include_bytes!("comirnaty_deck gpt5.5 skills.pptx").to_vec(),
                PathBuf::from("slides.pptx"),
            )],
            shared_files: Vec::new(),
        },
    ]
}
