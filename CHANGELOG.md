# Changelog

이 프로젝트의 주요 변경 사항을 기록합니다. 형식은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/),
버전은 [Semantic Versioning](https://semver.org/lang/ko/)을 따릅니다.

## [1.0.0] - 2026-07-03

첫 정식 릴리즈 — "매일 쓰는 완성품" 수준의 제품 완성도.

### 추가 (Added)
- **시스템 트레이** — 월페이퍼 프로세스에 트레이 아이콘이 상주. 설정 창 없이도
  **설정 열기 · 다음 씬 · 자동 시작 토글 · 종료**를 트레이 메뉴에서 제어. (전용 스레드 +
  메시지 루프, `Shell_NotifyIconW`)
- **로그인 자동 시작** — 설정 패널/트레이의 "자동 시작"으로 `HKCU\...\Run`에 등록/해제.
  마지막 씬을 그대로 재현하는 커맨드로 등록된다.
- **설정 저장** — 씬/오디오/게인/SSAA를 `%APPDATA%\real_live_wall\config.toml`에 자동
  저장하고 재실행 시 복원(`serde` + `toml`). CLI로 명시한 값은 저장값보다 우선한다.
- **단일 인스턴스 가드** — 월페이퍼는 한 번에 하나만 실행(중복 실행 방지).
- 설정 패널에 **"로그인 시 자동 시작"** 체크박스 추가.

### 변경 (Changed)
- `shaders/` 디렉터리를 현재 경로뿐 아니라 실행 파일 옆에서도 탐색(릴리즈 zip·자동 시작
  대응).

## [0.4.0] - 2026-07-03

### 추가 (Added)
- **멀티모니터 지원** — 월페이퍼 모드에서 연결된 **모든 모니터**에 각각 전체 장면을
  개별 렌더한다. 모니터마다 borderless 창 1개(+ 자체 서피스·후처리)를 만들고, 하나의
  GPU 디바이스와 씬 렌더러를 공유한다. (`gpu.rs`를 `GpuContext` + 창별 `Gpu`로 분리)
- **월페이퍼 원격 종료** — 실행 중인 월페이퍼를 어디서든 끌 수 있다. 설정 패널의
  **"■ 월페이퍼 중지"** 버튼 또는 `real_live_wall.exe --stop`. 세션-로컬 네임드 이벤트로
  프로세스 간 신호하며, 프리뷰 창을 닫았다가 다시 열어도 끌 수 있다.
- **바탕화면 복구** — 월페이퍼 종료 시 정적 바탕화면을 리페인트해 잔상을 없앤다.
- **앱 아이콘** — 오로라 그라디언트 + 스펙트럼 바 아이콘을 exe에 임베드
  (`build.rs` + `winresource`). 탐색기·작업표시줄·창 아이콘에 적용.

### 변경 (Changed)
- **자연 씬 화질 대폭 개선** — `ocean` `sunset_clouds` `mountains` `rain` 재작성.
  밝은 하이라이트(태양·물빛 글리터·번개, HDR 블룸 유발), 도메인워프 fbm 디테일,
  산 실루엣 안티에일리어싱(`step`→`smoothstep`), 대기 원근, 밴딩 제거용 디더링.
- `--stop` CLI 옵션 추가, winit 이벤트 루프를 user-event 기반으로 전환.

## [0.3.0] - 2026-07-03

### 추가
- **시네마틱 포스트FX** — `Rgba16Float` 슈퍼샘플 오프스크린 → bright-pass →
  핑퐁 가우시안 블룸 → ACES 톤매핑 + 채도 + 비네트. `--ssaa` 옵션.
- Windows `WorkerW` 부착으로 실제 바탕화면 월페이퍼(주 모니터).

## [0.2.0] - 2026-07-03

### 추가
- **egui 설정 GUI 패널**(F1) — 씬/셰이더 선택, 오디오 소스·게인, 실시간 미터,
  "바탕화면에 적용" 버튼.
- 자연 씬 5종(`ocean` `sunset_clouds` `mountains` `rain` `forest_fireflies`).

## [0.1.0] - 2026-07-03

### 추가
- 최초 공개 — Rust + wgpu 리액티브 크로스플랫폼 라이브 월페이퍼 엔진.
- 풀스크린 셰이더 1패스, Rust/WGSL/GLSL 공유 uniform 계약.
- 기본 WGSL 오로라 씬 + 64밴드 스펙트럼, Shadertoy GLSL(`mainImage`) 무수정 로드.
- 실시간 오디오(FFT) · CPU/메모리 리액티브 입력, GLSL 핫리로드.

[1.0.0]: https://github.com/BaeTab/real_live_wall/releases/tag/v1.0.0
[0.4.0]: https://github.com/BaeTab/real_live_wall/releases/tag/v0.4.0
[0.3.0]: https://github.com/BaeTab/real_live_wall/releases/tag/v0.3.0
[0.2.0]: https://github.com/BaeTab/real_live_wall/releases/tag/v0.2.0
[0.1.0]: https://github.com/BaeTab/real_live_wall/releases/tag/v0.1.0
