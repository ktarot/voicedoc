# vendor/

## whisper-rs-sys (0.15.0, whisper.cpp 1.8.3)

crates.io의 `whisper-rs-sys` 0.15.0을 그대로 복사하고 한 군데만 고친 사본입니다.
루트 `Cargo.toml`의 `[patch.crates-io]`로 연결되어 있습니다.

### 바꾼 점

`whisper.cpp/ggml/src/ggml-vulkan/ggml-vulkan.cpp`의 `ggml_vk_instance_init()` 첫머리:
환경 변수 `VOICEDOC_NO_VULKAN`이 있으면 Vulkan 함수를 하나도 부르지 않고 예외를 던집니다.
`ggml_backend_vk_reg()`가 이 예외를 받아 Vulkan 백엔드를 등록하지 않으므로 인식은 CPU로 진행됩니다.

### 왜 필요한가

윈도 앱은 `vulkan-1.dll`(그래픽 드라이버가 설치)을 지연 로드합니다(`src-tauri/build.rs`).
그래서 드라이버가 없는 PC에서도 앱이 켜지지만, 그 상태에서 Vulkan 함수를 부르면 프로그램이 종료됩니다.
앱은 인식 전에 `vulkan-1.dll`을 직접 불러 보고, 없으면 `VOICEDOC_NO_VULKAN`을 설정합니다
(`crates/core/src/stt/local.rs`의 `prepare_vulkan`).

드라이버는 있지만 그래픽카드가 없거나 Vulkan 초기화가 실패하는 경우는 whisper.cpp 1.8.3이 스스로 처리합니다.

### 버전을 올릴 때

1. 새 `whisper-rs-sys`를 이 폴더에 복사합니다 (`.cargo-checksum.json`, `.cargo_vcs_info.json` 제외).
2. 위의 한 군데를 다시 고칩니다. `VOICEDOC_NO_VULKAN`으로 검색하면 바뀐 부분을 찾을 수 있습니다.
3. 이 문서의 버전을 고칩니다.
