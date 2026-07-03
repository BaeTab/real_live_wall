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
| `main.rs` | CLI 파싱, winit 이벤트 루프 부팅 |
| `config.rs` | clap 기반 실행 옵션 (`--mode`, `--shader`, `--audio`, `--gain`, `--watch`) |
| `app.rs` | `ApplicationHandler`: 창 생명주기, 프레임 루프, 입력, 핫리로드 |
| `gpu.rs` | wgpu 인스턴스/어댑터/디바이스/서피스 부트스트랩, 리사이즈 |
| `renderer.rs` | uniform 버퍼·바인드그룹·파이프라인 소유, 셰이더 핫스왑, HDR 씬 패스 |
| `postfx.rs` | HDR 오프스크린 → bright-pass → 가우시안 블룸(핑퐁) → ACES 톤매핑+비네트 합성 |
| `shader.rs` | 풀스크린 WGSL 버텍스, 기본 WGSL 씬, **Shadertoy→GLSL 래퍼** |
| `uniforms.rs` | Rust/WGSL/GLSL 공유 uniform 레이아웃 (`#[repr(C)]` + `Pod`) |
| `audio.rs` | cpal 캡처(루프백 우선) + rustfft 스펙트럼 분석, graceful fallback |
| `reactive.rs` | sysinfo CPU/메모리 샘플링 |
| `ui.rs` | egui 설정 패널(씬/오디오/게인/미터/월페이퍼 적용), 셰이더 위에 합성 |
| `platform.rs` | 월페이퍼 표면 획득 — Windows `WorkerW` 부착, mac/linux 스텁 |

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

## 플랫폼별 월페이퍼 표면

- **Windows**: `Progman`에 `0x052C` 메시지 → `WorkerW` 생성 → `EnumWindows`(+Win11
  폴백)로 찾아 `SetParent(우리 창, WorkerW)`. 이후 `SetWindowPos`로 **주 모니터에 정확히
  꽉 차게**(가상 데스크톱 좌표 보정) + `HWND_BOTTOM`으로 아이콘 뒤에 배치. (멀티모니터
  개별 씬은 예정)
- **macOS**(예정): 각 스크린마다 `kCGDesktopWindowLevel` NSWindow.
- **Linux**(예정): X11 루트/데스크톱 타입, Wayland는 `wlr-layer-shell`.

## 로드맵 (엔진 강화 방향)

- [ ] 멀티모니터 개별 씬
- [ ] Shadertoy `iChannel0` 오디오 텍스처 완전 호환 (현재는 uniform 스펙트럼)
- [ ] 멀티패스(버퍼) 셰이더 그래프
- [ ] 날씨/캘린더/알림 리액티브 소스
- [ ] macOS/Linux 표면 구현
- [ ] 씬 매니페스트(JSON) + 워크샵/갤러리
- [ ] 배터리/포그라운드 전체화면 감지 시 자동 일시정지
