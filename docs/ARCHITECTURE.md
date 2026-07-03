# real_live_wall — 아키텍처

크로스플랫폼 **리액티브 라이브 월페이퍼 엔진**. GPU 네이티브(wgpu), Shadertoy GLSL
호환, 실시간 오디오/시스템 반응을 목표로 한다.

> **스택 주의:** 렌더링은 **wgpu 29**를 쓴다(최신 egui-wgpu 0.35가 `wgpu ^29`를
> 요구하기 때문). GUI(egui)와 엔진이 하나의 wgpu 인스턴스를 공유하며, 설정 패널은
> 셰이더와 같은 프레임 위에 `LoadOp::Load`로 합성된다. release 빌드는
> `windows_subsystem = "windows"`로 콘솔 없는 GUI 앱이다.

## 설계 원칙

1. **GPU 네이티브 / 저부하** — 모든 씬은 풀스크린 프래그먼트 셰이더 1패스로 그린다.
   버텍스 버퍼도, 지오메트리도 없다. 유휴 시 부하가 셰이더 비용에 수렴한다.
2. **하나의 uniform 계약** — Rust · WGSL · GLSL 세 곳이 **바이트 단위로 동일한**
   uniform 블록(`Uniforms`)을 공유한다. 셰이더 언어가 무엇이든 같은 데이터가 들어온다.
3. **차별화는 데이터에서** — 시간뿐 아니라 **오디오 스펙트럼 · CPU/메모리 · 마우스**가
   전부 셰이더로 흐른다. "그냥 재생기"가 아니라 "반응하는" 배경.
4. **어디서도 죽지 않는다** — 오디오 장치 없음, 잘못된 셰이더, 서피스 로스트 등
   모든 실패는 로그만 남기고 엔진은 계속 돈다.

## 데이터 흐름

```
                 ┌────────────────────────────────────────────┐
   cpal 캡처 ───►│ audio.rs : 링버퍼 → Hann → rustfft → 64 bins │──┐
 (루프백/입력)    └────────────────────────────────────────────┘  │
                 ┌────────────────────────────────────────────┐  │   ┌──────────────┐
  sysinfo   ───► │ reactive.rs : CPU/메모리 (1 Hz 샘플링)        │──┼──►│  Uniforms     │
                 └────────────────────────────────────────────┘  │   │ (std140,      │
                 ┌────────────────────────────────────────────┐  │   │  bytemuck Pod)│
  winit 입력 ───►│ app.rs : 시간/델타/프레임/마우스               │──┘   └──────┬───────┘
                 └────────────────────────────────────────────┘             │ write_buffer
                                                                             ▼
   shaders/*.glsl ──► shader.rs (Shadertoy 래핑) ──► renderer.rs ──► wgpu 파이프라인 ──► 화면
   (WGSL 기본 씬)                                    (풀스크린 삼각형 + FS)
```

## 모듈

| 파일 | 역할 |
|---|---|
| `main.rs` | CLI 파싱, `--stop` 조기 처리, user-event winit 루프 부팅, 종료 시 바탕화면 복구 |
| `config.rs` | clap 기반 실행 옵션 (`--mode`, `--shader`, `--audio`, `--gain`, `--watch`, `--stop`) |
| `app.rs` | `ApplicationHandler<AppEvent>`: 멀티모니터 창 생명주기, 프레임 루프, 입력, 핫리로드, 월페이퍼 스폰/종료 |
| `gpu.rs` | `GpuContext`(device/queue 공유) + 창별 `Gpu`(surface). 여러 모니터가 한 디바이스 공유 |
| `renderer.rs` | uniform 버퍼·바인드그룹·파이프라인 소유, 셰이더 핫스왑, HDR 씬 패스 |
| `postfx.rs` | HDR 오프스크린 → bright-pass → 가우시안 블룸(핑퐁) → ACES 톤매핑+비네트 합성 |
| `shader.rs` | 풀스크린 WGSL 버텍스, 기본 WGSL 씬, **Shadertoy→GLSL 래퍼** |
| `uniforms.rs` | Rust/WGSL/GLSL 공유 uniform 레이아웃 (`#[repr(C)]` + `Pod`) |
| `audio.rs` | cpal 캡처(루프백 우선) + rustfft 스펙트럼 분석, graceful fallback |
| `reactive.rs` | sysinfo CPU/메모리 샘플링 |
| `ui.rs` | egui 설정 패널(씬/오디오/게인/미터/월페이퍼 적용), 셰이더 위에 합성 |
| `platform.rs` | 월페이퍼 표면 획득 — Windows `WorkerW` 모니터별 부착, 네임드 이벤트 기반 원격 종료, 바탕화면 복구, mac/linux 스텁 |
| `persist.rs` | 설정 영속화 — `%APPDATA%/real_live_wall/config.toml` (serde+toml) 로드/저장 |
| `tray.rs` | 시스템 트레이 아이콘 + 팝업 메뉴 (전용 스레드·메시지 루프, `Shell_NotifyIconW`) |
| `startup.rs` | 로그인 자동 시작 — HKCU Run 레지스트리 키 등록/해제 |

## uniform 계약

```glsl
layout(std140, set = 0, binding = 0) uniform Uniforms {
    vec4 resolution;  // xy = 픽셀, z = 1, w = 종횡비
    vec4 mouse;       // xy = 현재, zw = 클릭 (origin: 좌하단)
    vec4 time;        // x=iTime, y=iTimeDelta, z=iFrame, w=sampleRate
    vec4 audio;       // x=bass, y=mid, z=treble, w=volume  (0..1)
    vec4 sys;         // x=cpu, y=mem, z=beat, w=fps
    vec4 date;        // year, month, day, secondsInDay
    vec4 spectrum[16];// 64 FFT bins (0..1)
};
```

Shadertoy 셰이더는 `shader.rs`의 래퍼가 위 블록을 `#define iResolution ...` 등으로
다시 매핑하고, `mainImage(out vec4, in vec2)`를 호출하는 `main()`을 자동 생성한다.
확장 헬퍼: `iBass/iMid/iTreble/iVolume`, `iCpu/iMem`, `float iSpectrum(float x)`.

## 렌더 파이프라인

- **버텍스**: 항상 WGSL 풀스크린 삼각형(3정점, 버퍼 없음).
- **프래그먼트**: 기본 WGSL 씬 또는 래핑된 GLSL. 둘 다 같은 파이프라인 레이아웃
  (group 0, binding 0 = uniform, FRAGMENT 가시성)을 쓴다.
- **HDR 후처리 파이프라인**: 씬은 스왑체인이 아니라 `Rgba16Float` 오프스크린 타깃에
  **슈퍼샘플(SSAA) 해상도**로 렌더 → `postfx`가 bright-pass → 2회 핑퐁 가우시안 블룸
  → ACES 톤매핑 + 채도 + 비네트로 스왑체인에 합성. SSAA가 앨리어싱을 없애고, 블룸이
  라이트·글로우를 살려 "프리미엄" 룩을 만든다.
- egui 패널은 합성 결과 위에 `LoadOp::Load`로 그린다.

## 멀티모니터 렌더링

월페이퍼 모드는 `ActiveEventLoop::available_monitors()`로 **모니터마다 borderless 창 1개**를
만든다. 각 창은 자기 `wgpu::Surface`와 후처리 체인(`PostFx`)을 갖지만, **디바이스/큐와 씬
`Renderer`(파이프라인 + uniform 버퍼)는 전체가 하나를 공유**한다. 한 프레임에서 모니터별로
`update_uniforms`(해상도/종횡비 갱신) → `submit` 순서가 큐에서 보장되므로, 유니폼 버퍼 하나를
재사용해도 각 모니터가 자기 해상도로 그려진다. 프레임 루프는 primary 창의 `RedrawRequested`
하나에서 전 모니터를 그린다(`about_to_wait`는 primary만 `request_redraw`).

## 플랫폼별 월페이퍼 표면

- **Windows**: `Progman`에 `0x052C` 메시지 → `WorkerW` 생성 → `EnumWindows`(+Win11
  폴백)로 찾아 각 창을 `SetParent(창, WorkerW)`. 이후 `SetWindowPos`로 **각 모니터 사각형에
  정확히 꽉 차게**(WorkerW client 원점 = 가상 데스크톱 좌상단이므로 `(rect.x-SM_XVIRTUALSCREEN,
  rect.y-SM_YVIRTUALSCREEN)`) + `HWND_BOTTOM`으로 아이콘 뒤에 배치.
- **macOS**(예정): 각 스크린마다 `kCGDesktopWindowLevel` NSWindow.
- **Linux**(예정): X11 루트/데스크톱 타입, Wayland는 `wlr-layer-shell`.

## 설정 영속화 · 트레이 · 자동 시작

- **영속화(`persist.rs`)**: preview 프로세스가 `config.toml`을 로드해 GUI 초기 상태(씬/
  오디오/게인/SSAA)를 복원하고, 설정 변경·종료 시 저장한다. CLI로 명시한 값(clap 기본값과
  다른 값)은 저장값보다 우선. wallpaper 프로세스는 명시적 인자로 실행되므로 저장하지 않는다.
- **트레이(`tray.rs`)**: wallpaper 프로세스가 **전용 스레드**에 message-only 창 + 메시지
  루프를 띄우고 `Shell_NotifyIconW`로 아이콘을 등록한다. 메뉴 선택(WM_COMMAND)은
  `TrayCommand`로 매핑돼 `EventLoopProxy<AppEvent>::send_event(AppEvent::Tray(..))`로
  메인 루프에 전달된다. "다음 씬"은 공유 렌더러의 셰이더를 교체(전 모니터 반영), "설정 열기"는
  preview 프로세스를 새로 스폰.
- **자동 시작(`startup.rs`)**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`에
  현재 씬을 재현하는 커맨드(exe + `--mode wallpaper --shader ... --audio ... --gain ...`)를
  `REG_SZ`로 등록/해제.
- **단일 인스턴스**: wallpaper 시작 시 `platform::wallpaper_running()`(stop 이벤트 존재
  여부)로 이미 실행 중이면 즉시 종료.

## 월페이퍼 생명주기 / 원격 종료

프리뷰의 **"바탕화면에 적용"**은 자기 exe를 `--mode wallpaper`로 **별도 프로세스** 스폰한다.
프리뷰 창을 닫으면 자식 핸들이 사라져 못 끄던 문제를 없애기 위해, 월페이퍼 프로세스는
**세션-로컬 네임드 이벤트**(auto-reset)를 등록하고 워커 스레드가 이를 대기한다. 어느
인스턴스든 `--stop`(= `OpenEvent`+`SetEvent`) 또는 패널 버튼이 이벤트를 신호하면, 워커
스레드가 `EventLoopProxy<AppEvent>::send_event(StopWallpaper)`로 루프를 깨워
`event_loop.exit()`. 실행 여부는 이벤트를 `OpenEvent`해 보는 것으로 감지한다. 프로세스가
빠져나오면 `main`이 `SPI_GETDESKWALLPAPER`→`SPI_SETDESKWALLPAPER`로 정적 바탕화면을
리페인트해 잔상(검은 영역)을 없앤다.

## 로드맵 (엔진 강화 방향)

- [x] 멀티모니터 — 모니터마다 전체 장면 개별 렌더 (완료)
- [x] 트레이 · 자동 시작 · 설정 저장 · 단일 인스턴스 (v1.0)
- [ ] 모니터별 씬 개별 선택
- [ ] Shadertoy `iChannel0` 오디오 텍스처 완전 호환 (현재는 uniform 스펙트럼)
- [ ] 멀티패스(버퍼) 셰이더 그래프
- [ ] 날씨/캘린더/알림 리액티브 소스
- [ ] macOS/Linux 표면 구현
- [ ] 씬 매니페스트(JSON) + 워크샵/갤러리
- [ ] 배터리/포그라운드 전체화면 감지 시 자동 일시정지
