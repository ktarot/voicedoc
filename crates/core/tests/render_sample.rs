//! 눈으로 확인할 견본 문서를 만든다: `cargo test --test render_sample -- --ignored`

use voicedoc_core::doc::Document;
use voicedoc_core::export::{self, Format};

#[test]
#[ignore = "견본 파일 생성용"]
fn render_sample() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut body = std::fs::read_to_string(root.join("testdata/sample-ko.md")).unwrap();
    body.push_str("\n## 녹취 전문\n");
    for i in 0..40 {
        body.push_str(&format!(
            "\n[00:{:02}:00] 회의 내용을 길게 적어 여러 쪽으로 넘어가는지 확인합니다. 줄바꿈과 쪽 나눔이 자연스러운지 살펴봅니다. 한글과 English 단어가 섞여도 괜찮아야 합니다.\n",
            i
        ));
    }
    let doc = Document::new("주간 회의 - 회의록", body);
    for format in Format::ALL {
        let path = root.join("out").join(format!("sample.{}", format.extension()));
        export::save(&doc, format, &path).unwrap();
    }
}
