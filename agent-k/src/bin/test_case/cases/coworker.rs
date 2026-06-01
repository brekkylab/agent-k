use std::path::PathBuf;

use ailoy::message::{Message, Part, Role};

use super::Case;

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
        // Case 12 — xlsx create: 학생 성적 (skill is provided by the coworker agent; toggle via `--no-skill`)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "학생 성적 관리용 엑셀 파일 하나 만들어줘. 학생 12명 정도 \
                 예시 데이터로 채우고, 컬럼은 학번/이름/주민번호/전공점수/\
                 영어점수/교양점수까지 직접 입력, 합계/평균/순위/평가는 \
                 자동 계산되게 수식으로 넣어 줘. 평가는 평균 90 이상 \
                 '우수', 60 미만 '재시험', 나머지 '보통'. 평균 60점 미만인 \
                 학생은 행 전체가 빨갛게 표시되도록 조건부 서식도 넣어 줘. \
                 그리고 시트 아래쪽에 최소/최대값, 여학생/남학생 수, 김씨 \
                 성 학생 수, 평균 80점 이상 학생 수 같은 요약 통계도 \
                 자동 계산되게 같이 넣어줘. 저장은 students.xlsx 같은 \
                 이름으로.",
            )]),
            files: vec![],
            shared_files: Vec::new(),
        },
        // Case 13 — xlsx create: sales_dashboard.xlsx multi-sheet (skill from coworker agent)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "sales_dashboard.xlsx 파일 하나 만들어줘.\n\n\
                 요구사항:\n\
                 - 시트는 \"원본데이터\", \"월별요약\", \"대시보드\" 3개\n\
                 - 원본데이터에는 거래내역 예시 데이터 100건 생성\n\
                 - 컬럼: \n\
                   거래일자 / 주문번호 / 고객명 / 지역 / 상품카테고리 / 상품명 / \n\
                   수량 / 단가 / 공급가액 / 부가세 / 최종금액 / 담당자\n\n\
                 규칙:\n\
                 - 공급가액 = 수량 * 단가\n\
                 - 부가세 = 공급가액의 10%\n\
                 - 최종금액 = 공급가액 + 부가세\n\
                 - 모든 계산은 엑셀 수식으로 넣기\n\
                 - 주문번호는 ORD-2026-0001 형식\n\n\
                 월별요약 시트:\n\
                 - 월별 매출 합계\n\
                 - 지역별 매출 합계\n\
                 - 카테고리별 매출 비율\n\
                 - 피벗테이블 사용\n\
                 - 피벗 새로고침해도 안 깨지게 구성\n\n\
                 대시보드 시트:\n\
                 - 월별 매출 차트\n\
                 - 지역 TOP5 차트\n\
                 - KPI 카드 4개: 총매출 / 평균주문금액 / 최고매출지역 / 총주문수\n\
                 - 조건부서식 사용\n\
                 - 인쇄 시 A4 가로 1페이지 맞춤\n\n\
                 - 저장명은 sales_dashboard.xlsx",
            )]),
            files: vec![],
            shared_files: Vec::new(),
        },
        // Case 14 — xlsx edit: sales_dashboard.xlsx with 수수료 column insert (skill from coworker agent)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "artifacts/sales_dashboard.xlsx 엑셀 파일을 수정해줘.\n\n\
                 수정 사항:\n\
                 1. 월별요약 탭에 분기별 매출 요약 표 추가 (Q1/Q2/Q3/Q4)\n\
                 2. 원본데이터 탭의 부가세 수식을 10% → 20%로 변경\n\
                 3. 원본데이터 탭의 공급가액과 부가세 사이에 \"수수료\" 컬럼을 \
                 신설하고 값은 단가의 5%로 계산 (부가세·최종금액 컬럼은 한 칸 \
                 오른쪽으로 밀리고, 최종금액 = 공급가액 + 수수료 + 부가세로 재계산)\n\n\
                 다른 시트, 차트, 서식, KPI 카드는 모두 그대로 유지.\n\
                 결과는 같은 파일명으로 저장.",
            )]),
            files: vec![(
                include_bytes!("sales_dashboard.xlsx").to_vec(),
                PathBuf::from("sales_dashboard.xlsx"),
            )],
            shared_files: Vec::new(),
        },
        // Case 15 — xlsx create: financial_report.xlsx (skill from coworker agent)
        Case {
            query: Message::new(Role::User).with_contents([Part::text(
                "financial_report.xlsx 생성해줘.\n\n\
                 주의:\n\
                 이 파일은 임원 보고용이라 절대 깨지면 안 됨.\n\n\
                 요구사항:\n\
                 - 손익계산서\n\
                 - 대차대조표\n\
                 - 현금흐름표\n\
                 - 요약대시보드\n\n\
                 데이터:\n\
                 - 최근 24개월 예시 데이터 자동 생성\n\n\
                 필수 기능:\n\
                 - 모든 합계는 수식 사용\n\
                 - 전년동기대비 증감률 계산\n\
                 - 적자 항목은 빨간색 괄호 표기\n\
                 - 차트 포함\n\
                 - 인쇄영역 설정\n\
                 - 페이지 번호 포함\n\
                 - 머리글/바닥글 설정",
            )]),
            files: vec![],
            shared_files: Vec::new(),
        },
    ]
}
