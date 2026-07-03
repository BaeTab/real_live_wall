# real_live_wall

> **리액티브 · 크로스플랫폼 라이브 월페이퍼 엔진** — GPU 네이티브(wgpu),
> Shadertoy GLSL 호환, 실시간 오디오/시스템 반응.

시중의 라이브 월페이퍼는 대부분 "영상·웹페이지를 배경에 트는 재생기"다.
`real_live_wall`은 다르다. **바탕화면이 지금 내 컴퓨터의 상태 — 재생 중인 음악의
스펙트럼, CPU/메모리 부하, 시간 — 에 실시간으로 반응**하고, **Shadertoy의 GLSL
셰이더를 거의 그대로** 돌린다. 그리고 Windows/macOS/Linux를 **하나의 셰이더 포맷**으로
겨냥한다. 모든 씬은 **HDR 블룸 · ACES 톤매핑 · 슈퍼샘플 AA**를 거쳐 시네마틱하게 출력된다.

## ✨ 차별점

| | Wallpaper Engine | Lively | **real_live_wall** |
|---|:---:|:---:|:---:|
| 크로스플랫폼 | ❌ Windows | ❌ Windows | ✅ Win/Mac/Linux(진행 중) |
| GPU 네이티브 (Vulkan/DX12/Metal) | 부분 | ❌(브라우저) | ✅ wgpu |
| 오디오 반응(FFT) | 제한적 | ❌ | ✅ 64-bin 스펙트럼 + bass/mid/treble |
| 시스템 반응(CPU/메모리) | ❌ | ❌ | ✅ |
| Shadertoy 셰이더 호환 | ❌ | ❌ | ✅ `mainImage()` 그대로 |
| HDR 블룸·톤매핑·AA | ✅ | ❌ | ✅ 시네마틱 포스트FX |
| 설정 GUI | ✅ | 일부 | ✅ egui 패널(F1) |
| 오픈소스 | ❌ | ✅ | ✅ |

## 🖼️ 스크린샷

**설정 GUI (egui)** — 씬/셰이더 선택, 오디오 소스·게인, 실시간 미터(FPS·CPU·오디오),
그리고 **"바탕화면에 적용"** 버튼. `F1`로 패널을 숨기면 순수 배경만 남습니다.

![real_live_wall 설정 GUI](docs/screenshots/gui.png)

**Shadertoy GLSL** — `shaders/plasma.glsl`을 수정 없이 로드 (naga GLSL 프론트엔드)

![Shadertoy plasma 셰이더](docs/screenshots/shadertoy_plasma.png)

> 실측 환경: Windows 11 · NVIDIA RTX 3060 · Vulkan 백엔드.

## 🚀 빠른 시작

필요: [Rust](https://rustup.rs) (stable), 그리고 Vulkan/DX12/Metal 지원 GPU.

가장 쉬운 길: [릴리즈](https://github.com/BaeTab/real_live_wall/releases)에서 zip을 받아
`real_live_wall.exe`를 **더블클릭** → 설정 GUI 창이 뜹니다. 씬·오디오를 고르고
**"바탕화면에 적용"**을 누르면 데스크톱 배경으로 실행됩니다. (`F1` = 패널 토글)

소스로 실행:

```bash
# 설정 GUI가 있는 미리보기 창 (기본 오로라 씬)
cargo run --release

# Shadertoy 스타일 GLSL 씬 로드 + 파일 변경 시 핫리로드
cargo run --release -- --shader shaders/audio_bars.glsl --watch

# 처음부터 데스크톱 월페이퍼로 (GUI 없이)
cargo run --release -- --mode wallpaper --shader shaders/plasma.glsl
```

단축키: `F1` 설정 패널 토글 · `Esc` 종료(preview 모드).

## ⚙️ CLI 옵션

| 옵션 | 기본값 | 설명 |
|---|---|---|
| `--mode <preview\|wallpaper>` | `preview` | 미리보기 창 / 실제 바탕화면 |
| `--shader <path>`, `-s` | (기본 WGSL 씬) | Shadertoy GLSL 파일 |
| `--audio <auto\|input\|loopback\|off>` | `auto` | 오디오 소스 (Windows는 auto=루프백) |
| `--gain <f32>` | `6.0` | 오디오 감도 |
| `--ssaa <f32>` | `1.5` | 슈퍼샘플 AA 배율 (1.0=끔, 2.0=최상, 부하↑) |
| `--watch` | `false` | 셰이더 파일 핫리로드 |
| `--width`/`--height` | `1280`/`720` | preview 창 크기 |

## 🎨 셰이더 작성 (Shadertoy 호환)

Shadertoy와 동일하게 `mainImage`만 정의하면 된다:

```glsl
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    fragColor = vec4(uv, 0.5 + 0.5 * sin(iTime), 1.0);
}
```

지원 uniform(표준): `iResolution`, `iTime`, `iTimeDelta`, `iFrame`, `iMouse`,
`iDate`, `iSampleRate`, `iFrameRate`.

**엔진 확장**(리액티브 월페이퍼용):

| 이름 | 의미 |
|---|---|
| `float iBass / iMid / iTreble / iVolume` | 오디오 밴드 에너지 (0..1) |
| `float iSpectrum(float x)` | `x`(0..1) 위치의 FFT 스펙트럼 |
| `float iCpu / iMem` | CPU·메모리 부하 (0..1) |

예제: [`shaders/plasma.glsl`](shaders/plasma.glsl)(순수 Shadertoy),
[`shaders/audio_bars.glsl`](shaders/audio_bars.glsl)(오디오 반응).

### 🌊 기본 제공 씬

`shaders/` 폴더의 `.glsl`은 설정 GUI 드롭다운에 자동으로 나타납니다.

| 씬 | 설명 |
|---|---|
| 기본 (오로라) | 오로라 + 별 + 64밴드 스펙트럼 이퀄라이저 (WGSL) |
| `ocean` | 황혼의 바다 + 태양 반짝임 (bass 반응) |
| `sunset_clouds` | 노을 하늘에 흐르는 fbm 구름 |
| `mountains` | 해질녘 다층 산 실루엣 + 별 |
| `rain` | 빗줄기 + bass에 번쩍이는 번개 |
| `forest_fireflies` | 안개 낀 숲의 반딧불 (volume 반응) |
| `plasma` | 클래식 Shadertoy 플라즈마 |
| `audio_bars` | 오디오 스펙트럼 바 |

## 🧭 아키텍처

풀스크린 셰이더 1패스 + 세 언어(Rust/WGSL/GLSL) 공유 uniform 계약.
자세한 내용은 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## 🗺️ 로드맵

- [ ] macOS/Linux 월페이퍼 표면 구현
- [ ] 멀티모니터 개별 씬
- [ ] Shadertoy `iChannel0` 오디오 텍스처 완전 호환
- [ ] 멀티패스(버퍼) 셰이더
- [ ] 날씨/캘린더 리액티브 소스
- [ ] 전체화면·배터리 감지 자동 일시정지
- [ ] 씬 매니페스트(JSON) + 갤러리

## 📄 라이선스

MIT OR Apache-2.0
