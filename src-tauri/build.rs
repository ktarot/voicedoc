fn main() {
    // 그래픽카드 가속(Vulkan)의 vulkan-1.dll은 그래픽 드라이버가 설치한다.
    // 드라이버가 없는 PC에서도 앱이 켜지도록, 이 DLL은 실제로 쓸 때 불러온다(지연 로드).
    // 없으면 인식 엔진이 Vulkan을 건너뛰고 CPU로 동작한다 (crates/core/src/stt/local.rs).
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" && env == "msvc" {
        println!("cargo:rustc-link-arg=/DELAYLOAD:vulkan-1.dll");
        println!("cargo:rustc-link-lib=delayimp");
    }
    tauri_build::build()
}
