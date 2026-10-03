# hwpx2html

**공무원은 지금처럼 쓰고, 국민과 AI는 원문 그대로 읽습니다.**

한글(HWPX) 문서를 이미지까지 포함한 **단일 HTML 파일**로 변환합니다. 원문의 쪽 배치와 서식을 최대한 보존하면서, 문단·제목·표·목록을 HTML 구조로 담아 웹브라우저와 검색·AI 도구에서 활용할 수 있게 합니다.

> Convert a Korean HWPX document into a single self-contained HTML file, so that both people and AI can read public documents as originally written.

현재 버전은 **v0.10**입니다. 실행 프로그램과 Cargo 패키지에 표시되는 버전은 `0.10.0`입니다.

## 왜 필요한가요?

공공기관은 한글로 보도자료, 공고문, 정책 자료를 작성합니다. HWPX를 그대로 게시하면 열람 환경에 제약이 있고, 검색엔진이나 AI 서비스에서 내용을 활용하려면 별도의 포맷 지원이나 변환이 필요합니다.

`hwpx2html`은 기존 문서를 다시 작성하지 않고 HTML로 공개할 수 있게 합니다.

| 게시 방식 | 사람이 읽기 | 검색·AI 도구에서 활용 | 문서 구조 |
| --- | --- | --- | --- |
| HWPX 그대로 게시 | HWPX를 지원하는 프로그램·뷰어 필요 | 포맷 지원 또는 별도 변환 필요 | 원본에 보존 |
| PDF로 변환 | 브라우저·PDF 뷰어로 열람 | 텍스트 추출·OCR 품질에 따라 다름 | 태그·변환 방식에 따라 다름 |
| 마크다운으로 재작성 | 마크다운 뷰어·웹 게시 환경 필요 | 텍스트 활용에 적합 | 복잡한 표·쪽 배치 표현에 한계 |
| **hwpx2html로 HTML 변환** | **웹브라우저로 열람** | **HTML 텍스트와 구조 활용** | **원문의 구조와 배치 보존을 목표로 변환** |

- **작성 방식은 그대로:** 평소처럼 한글에서 작성한 HWPX를 변환합니다.
- **이미지와 서식을 함께:** 이미지·표·글자 서식·쪽 배치를 HTML에 담습니다.
- **파일 하나로 게시:** `--resource-mode embedded`로 이미지와 CSS를 내장합니다.
- **실행 프로그램 제공:** Windows 64비트용 프로그램은 Rust나 Python 설치 없이 실행할 수 있습니다.
- **내 PC에서 변환:** 변환에 서버 업로드나 외부 AI 서비스가 필요하지 않습니다.

영국 정부의 GOV.UK도 문서 공개 시 가능한 한 HTML을 사용하도록 [안내](https://www.gov.uk/guidance/publishing-accessible-documents)하고, PDF를 게시할 때 [HTML 대안을 함께 제공하는 정책](https://www.gov.uk/government/organisations/cabinet-office/about/accessible-documents-policy)을 두고 있습니다. `hwpx2html`은 이러한 공개 방식을 한국 공공기관의 HWPX 업무 환경에서 활용하려는 도구입니다.

## 내려받기

[최신 릴리스](https://github.com/everydaymind/hwpx2html/releases/latest)에서 `hwpx2html-0.10.0-windows-x86_64.zip`을 내려받아 압축을 풉니다. ZIP에는 `hwpx2html.exe`, 이 사용 안내, 라이선스와 의존성 라이선스 안내가 들어 있습니다.

현재 제공하는 실행 파일은 **Windows 64비트용 명령줄 프로그램**입니다. 프로그램을 더블 클릭하기보다 PowerShell에서 아래 명령으로 실행해 주세요. 다른 운영체제에서는 소스로 빌드할 수 있습니다.

## 게시 도우미: 명령줄 없이 변환하기

오프라인 게시 도우미 `hwpx2html-publisher.html`을 단일 파일로 제공합니다. v0.10부터 릴리스 ZIP에 함께 포함되어 있습니다.

1. `hwpx2html-publisher.html`을 Chrome 또는 Edge에서 엽니다. 브라우저별 로컬 파일 동작은 다음 검증에서 확인합니다.
2. HWPX 파일을 선택하거나 끌어 놓고 **HTML로 변환**을 누릅니다. 원본은 PC 안에서 처리하며 서버로 전송하지 않습니다.
3. 미리보기와 한국어 경고를 원본과 대조합니다. 경고의 쪽 번호를 누르면 한 쪽 보기 미리보기가 해당 쪽으로 이동합니다. 전체 보기에서는 쪽 번호를 안내합니다.
4. 경고가 있으면 확인 항목을 체크한 뒤 **HTML 내려받기**로 결과를 저장합니다. 공개 HTML 주소를 입력하면 링크와 iframe 삽입 코드를 복사할 수 있습니다. div 직접 포함 안내와 변환 정보 복사도 제공합니다.

한 쪽 보기·전체 보기, 제목·목록 구조 추론, 엄격 모드를 선택할 수 있습니다. 제목은 원본 파일명이며, 그림과 CSS는 HTML에 내장됩니다. 미리보기와 내려받기는 같은 변환 결과를 사용합니다. 네이티브 CLI와 WASM의 바이트 동일성은 검증 예정입니다.

원본의 그림 설명과 의미 있는 원본 그림 파일명이 대체 텍스트에, 원본 문서 제목이 HTML 메타데이터에 남을 수 있으므로 공개 전에 확인하세요. 이 도구는 개인정보 제거 기능을 제공하지 않으며, 메모·변경 추적이 일괄 제거된다고 보장하지 않습니다. 대형 문서의 브라우저 메모리 사용과 업무 PC의 파일 실행 정책도 확인이 필요합니다.

소스에서 도구를 만들려면 Rust의 WASM 타깃과 Python 3.11 이상이 필요합니다. `wasm-bindgen`과 npm은 필요하지 않습니다.

```powershell
rustup target add wasm32-unknown-unknown
python scripts/build_publisher.py
```

실행할 파일은 `target/publisher/hwpx2html-publisher.html`입니다. 소스 폴더의 `web/publisher.html`을 열어도 빌드한 도구로 이동합니다. `web/publisher.html.in`은 WASM과 스크립트를 넣기 전의 빌드용 템플릿이므로 직접 실행하지 않습니다.

기관 서버에서 변환하려면 CLI 또는 Rust의 `convert::convert_bytes` API로 단일 HTML을 만든 뒤 저장하고 게시글에서 링크하거나 iframe으로 표시할 수 있습니다. 서버 운영과 CMS 업로드 연동은 기관별로 구현합니다.

## 빠른 시작: 단일 HTML 만들기

`hwpx2html.exe`와 `문서.hwpx`를 같은 폴더에 놓습니다. 파일 탐색기에서 해당 폴더를 열고 주소창에 `powershell`을 입력해 PowerShell을 실행한 다음, 아래 명령을 입력합니다.

```powershell
.\hwpx2html.exe convert --input "문서.hwpx" --output "문서.html" --resource-mode embedded
```

생성된 `문서.html`을 웹브라우저로 열어 확인합니다. 이미지와 CSS가 파일 안에 들어 있으므로 **HTML 파일 하나만** 복사하거나 홈페이지에 올리면 됩니다. 일반적인 변환 성공 시에는 별도의 메시지를 출력하지 않습니다.

다른 폴더에 있는 파일도 경로를 지정해서 변환할 수 있습니다.

```powershell
.\hwpx2html.exe convert --input "C:\문서\보도자료.hwpx" --output "C:\문서\보도자료.html" --resource-mode embedded
```

## 폴더의 문서 한꺼번에 변환하기

```powershell
.\hwpx2html.exe batch --input-dir ".\원본" --output-dir ".\HTML" --recursive --resource-mode embedded --verbose --report ".\변환결과.jsonl"
```

하위 폴더까지 찾아 같은 폴더 구조로 HTML을 만듭니다. `--verbose`는 진행 상황을 표시하고, `--report`는 문서별 결과와 경고를 JSONL 파일에 기록합니다. JSONL은 한 줄에 결과 하나를 담는 JSON 형식입니다.

## 열람·검색·인쇄

- 기본 화면은 한 쪽씩 표시합니다. 화면 위쪽에 마우스를 가져가거나 터치하면 탐색 막대가 나타납니다.
- 방향키 또는 종이 영역의 왼쪽·오른쪽을 눌러 이전·다음 쪽으로 이동합니다.
- 문서 전체를 검색하려면 탐색 막대의 **전체 보기**로 전환한 뒤 `Ctrl+F`를 사용합니다.
- 브라우저 인쇄는 모든 쪽을 대상으로 합니다. 용지와 서식이 중요한 문서는 인쇄 미리보기도 확인해 주세요.

처음부터 모든 쪽을 표시하는 HTML이 필요하면 `--no-page-navigation`을 추가합니다.

```powershell
.\hwpx2html.exe convert --input "문서.hwpx" --output "문서.html" --resource-mode embedded --no-page-navigation
```

## 자주 쓰는 옵션

| 옵션 | 용도 |
| --- | --- |
| `--resource-mode embedded` | 이미지·CSS가 내장된 단일 HTML 생성 |
| `--resource-mode external` | HTML과 별도의 이미지·CSS 폴더 생성. **현재 기본값** |
| `--force` | 같은 이름의 기존 출력 파일 교체 |
| `--strict` | 지원하지 않는 개체나 누락될 부분이 있으면 HTML을 생성하지 않고 실패 처리 |
| `--report "결과.jsonl"` | 변환 결과와 경고 기록 |
| `--no-page-navigation` | 모든 쪽을 한 번에 표시 |
| `--no-infer-structure` | 원본에 명시된 제목·목록만 사용하고 문단 표시로 추가 구조를 추론하지 않음 |

모든 옵션은 도움말에서 확인할 수 있습니다.

```powershell
.\hwpx2html.exe --version
.\hwpx2html.exe convert --help
.\hwpx2html.exe batch --help
```

### 이미지·CSS를 별도 파일로 제공하기

```powershell
.\hwpx2html.exe convert --input "문서.hwpx" --output "문서.html" --resource-mode external
```

`문서.html`과 `문서.html.assets` 폴더가 만들어집니다. 게시하거나 옮길 때는 **두 항목을 함께**, 같은 상대 위치에 두어야 합니다. 이미지가 많은 문서에서 HTML 자체의 크기를 줄이고 싶을 때 사용할 수 있습니다.

## 홈페이지·AI 서비스에서 활용하기

변환한 HTML을 홈페이지에 독립 문서로 게시하고 게시판에서 링크하면 브라우저로 바로 열람할 수 있습니다. 기존 페이지에서는 아래처럼 생성 HTML 전체를 `<div>` 안에 그대로 포함하거나, 별도 HTML을 `iframe`으로 표시할 수 있습니다. 게시판 편집기에 붙여 넣는 경우에는 편집기가 HTML·CSS·스크립트를 제거하는지 확인하세요.

### 홈페이지의 `<div>` 안에 직접 포함하기

아래 설명은 **생성된 HTML 파일 전체를 `<html>` 태그까지 포함해 홈페이지의 `<div>` 안에 그대로 넣는 방식**을 전제로 합니다. `<head>`의 CSS·스크립트와 `<body>`의 용지·문서 요소를 분리하거나 옮기지 않고 원본 그대로 포함합니다. 홈페이지의 서버 템플릿 등에서 파일 전체 내용을 해당 위치에 출력하고, 감싸는 컨테이너의 스타일을 조정하면 됩니다.

컨테이너의 기본 설정 예시는 다음과 같습니다.

```html
<style>
  .hwpx-scroll {
    max-width: 100%;
    overflow-x: auto;
  }

  #hwpx-document {
    position: relative;
    isolation: isolate;
    min-height: 5700px; /* 추가 높이 확보가 필요한 5쪽 예시. 문서에 맞게 변경/생략 */
  }
</style>

<div class="hwpx-scroll">
  <div id="hwpx-document" lang="ko">
    <!-- 생성된 HTML 파일 전체를 이 위치에 그대로 포함합니다. 아래는 구조 예시입니다. -->
    <!doctype html>
    <html lang="ko">
      <head>
        <!-- 생성 파일의 meta, style 또는 CSS 링크, script 등을 그대로 포함 -->
      </head>
      <body>
        <!-- 생성 파일의 용지 요소, main, script 등을 그대로 포함 -->
      </body>
    </html>
  </div>
</div>
```

- **절대좌표의 기준점을 지정하세요.** `#hwpx-document`에 `position: relative`를 설정해야 쪽·문단의 절대좌표가 이 컨테이너를 기준으로 배치됩니다. 지정하지 않으면 문서가 게시판 제목이나 첨부파일 영역 위에 겹칠 수 있습니다. `isolation: isolate`는 음수 `z-index`를 쓰는 용지 배경 등이 홈페이지 배경 뒤로 사라지지 않도록 쌓임 범위를 분리합니다. CSS의 위치 기준은 [MDN 설명](https://developer.mozilla.org/en-US/docs/Web/CSS/position)을 참고하세요.
- **컨테이너 높이를 확인하세요.** 생성 파일 전체를 그대로 포함하면 문서 높이를 확보하는 CSS도 함께 들어갑니다. 홈페이지 CSS가 높이·배치를 덮는 등의 이유로 컨테이너 높이가 사라지면 하단의 목록 버튼·이전/다음 글·푸터가 본문과 겹칠 수 있습니다. 절대좌표 요소만으로는 부모 높이가 늘어나지 않으며 `position: relative`도 높이를 확보하지 않습니다. 이런 경우 컨테이너에 필요한 `height` 또는 `min-height`를 지정하세요. 위 `5700px`는 전체 보기로 표시하는 5쪽 보도자료의 추가 공간 예시이며 모든 문서에 적용하는 값이 아닙니다. 전체 보기/한 쪽 보기, 쪽 수·용지 크기·쪽 사이 간격·표시 배율에 맞춰 조정하고, 자동 높이가 정상이라면 생략할 수 있습니다.
- **용지 폭을 강제로 줄이지 마세요.** 출력은 원문의 쪽 좌표를 보존합니다. 좁은 게시판에서는 바깥 `.hwpx-scroll`처럼 가로 스크롤을 제공할 수 있습니다. 문서 안의 표·이미지에 일괄적으로 `width: 100%`나 `max-width: 100%`를 적용하면 원래 배치가 바뀔 수 있습니다.
- **먼저 감싸는 컨테이너의 스타일을 조정하세요.** 생성 파일의 CSS·스크립트를 수정하거나 분리할 필요 없이, 위와 같이 컨테이너에 위치 기준과 필요한 높이·스크롤을 지정하는 방식으로 표시할 수 있습니다. 다만 홈페이지의 `p`, `table`, `span`, `img` 또는 `.detail-cont *` 규칙, 특히 `!important`가 붙은 글꼴·줄간격·자간·여백·배경·표 배치 규칙이 문서 스타일을 덮는지는 확인하세요. 생성 CSS의 `html`, `body`, `main`, `@page` 규칙도 홈페이지에 영향을 줄 수 있습니다. 충돌이 실제로 있는 경우 사이트 쪽 스타일을 조정하세요. `position: relative`와 `isolation: isolate`는 CSS 선택자까지 격리하지는 않습니다.
- **스크립트의 실행 방식도 확인하세요.** 자간 보정에는 생성 스크립트가 필요합니다. 홈페이지의 CSP가 인라인 스크립트·스타일이나 내장 이미지(`data:` URI)를 제한하면 사이트 정책에 맞게 통합해야 합니다. JavaScript의 `innerHTML`로 삽입한 `<script>`는 자동 실행되지 않으며([MDN 설명](https://developer.mozilla.org/en-US/docs/Web/API/Element/innerHTML)), 현재 생성 스크립트는 `DOMContentLoaded`에 초기화하므로 페이지 로딩이 끝난 뒤 동적으로 넣는 경우에도 별도 통합이 필요합니다.
- **기본 쪽 탐색의 동작을 확인하세요.** 파일 전체를 그대로 포함하면 쪽 탐색 스크립트도 함께 들어갑니다. 이 스크립트는 페이지 전체를 대상으로 요소를 찾고, 창의 스크롤·URL 해시·상단 탐색 막대를 사용하므로 홈페이지의 동작과 함께 확인하세요. 처음부터 모든 쪽을 표시하고 싶을 때만 아래처럼 `--no-page-navigation`으로 변환할 수 있습니다. 파일 전체를 포함하기 위해 필요한 옵션은 아니며, 자간 보정 스크립트는 유지됩니다.

```powershell
.\hwpx2html.exe convert --input "문서.hwpx" --output "문서.html" --resource-mode embedded --no-page-navigation
```

`external` 출력을 삽입하면 이미지·CSS의 상대 경로가 **삽입한 홈페이지의 URL을 기준**으로 해석되므로 자원 경로도 맞춰야 합니다. 직접 삽입 후에는 첫 쪽·마지막 쪽, 표와 이미지, 홈페이지의 상단·하단을 함께 확인하세요. 인쇄용 CSS도 홈페이지와 공유되므로 인쇄는 독립 HTML을 열어 확인하는 편이 좋습니다.

v0.9에는 다른 페이지에 삽입할 수 있도록 CSS·스크립트·ID를 자동 격리하는 전용 출력 모드가 없습니다. 여러 문서를 한 페이지에 직접 넣으면 생성 클래스·ID가 충돌할 수 있습니다. 사이트별 스타일 조정이 어렵거나 여러 문서를 표시해야 한다면 `iframe`으로 각각의 독립 HTML을 표시하세요.

출력은 글자만 담은 이미지가 아니라 텍스트와 문단·표·셀·제목·목록 요소를 담은 HTML입니다. 쪽을 넘는 문단과 표도 하나의 논리 요소로 연결해 검색·추출 도구에서 활용할 수 있게 합니다. 실제 검색 수집과 AI의 이해 품질은 게시 환경, 원본 문서의 구조, 사용하는 서비스에 따라 달라집니다.

## 지원 범위와 확인할 사항

- **ZIP 기반 HWPX**를 지원합니다. 기존 바이너리 `.hwp` 파일이나 확장자만 `.hwpx`로 바꾼 파일은 지원하지 않습니다. 한글에서 HWPX 형식으로 다시 저장해 주세요.
- 글자·문단·표·지원되는 그림·글상자·일부 도형·수식 등을 변환합니다. 모든 HWPX 기능을 완전히 재현하는 것은 아닙니다.
- 미지원 개체나 표현은 해당 부분을 건너뛰고 경고를 남길 수 있습니다. 결과를 검토하거나 `--strict --report "결과.jsonl"`로 누락을 허용하지 않는 변환을 수행하세요.
- 원문의 글꼴을 HTML에 내장하지는 않습니다. 열람 PC의 글꼴과 브라우저에 따라 글자 폭이나 배치가 달라질 수 있습니다. 원문과 같은 글꼴이 설치된 환경에서 더 가깝게 표시됩니다.
- 이미지 안의 글자를 OCR로 추출하지 않습니다. 이미지의 내용 설명은 원본에 있는 정보에 의존합니다.
- 변환 후에도 제목 단계·표 머리셀·이미지 설명 등은 게시 전에 확인해 주세요. HTML 변환만으로 모든 문서의 웹 접근성이 보장되지는 않습니다.

## 소스로 빌드하기

[Rust](https://www.rust-lang.org/tools/install)와 Git을 설치한 다음 실행합니다. Rust edition 2021을 사용하며 의존성 버전은 `Cargo.lock`으로 고정합니다.

```text
git clone https://github.com/everydaymind/hwpx2html.git
cd hwpx2html
cargo build --release --locked
```

실행 파일은 Windows에서 `target/release/hwpx2html.exe`, Linux·macOS에서 `target/release/hwpx2html`에 생성됩니다. Linux·macOS에서는 위 활용 예시의 `.\hwpx2html.exe`를 `./target/release/hwpx2html`로 바꾸어 실행하면 됩니다.

## 개발자 활용: 문서 배치 정보 추출

```powershell
.\hwpx2html.exe inspect --input "문서.hwpx" --output "문서배치.json"
```

`inspect`는 원본 텍스트와 쪽·개체 배치 정보를 JSON으로 기록합니다. 데이터 형식은 [page-manifest.schema.json](schemas/page-manifest.schema.json)을 참고하세요. Rust 라이브러리도 함께 제공하며, 같은 소스의 공개 모듈을 사용할 수 있습니다.

## 이런 분께 권합니다

- 보도자료·공고문·정책 자료를 홈페이지에 게시하는 공공기관 담당자
- 공공 문서의 개방성과 접근성을 높이려는 정보화 담당자
- 공공 문서를 검색·AI 서비스에 활용하려는 개발자와 연구자

## 문의·문제 제보

[GitHub Issues](https://github.com/everydaymind/hwpx2html/issues)에 프로그램 버전, 실행 명령, 기대한 결과와 실제 결과를 적어 주세요. 재현 문서를 첨부할 때는 공개할 수 있는 내용인지 먼저 확인해 주세요.

## 라이선스

[MIT License](LICENSE). Copyright (c) 2026 everydaymind.

의존성 라이선스와 고지는 릴리스 및 공개 소스에 포함된 `THIRD_PARTY_NOTICES.txt`를 참고하세요.
