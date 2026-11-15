# eDEX-Remake

eDEX-Remake는 서버나 전용 모니터에 띄워둘 수 있는 Linux 터미널·시스템 모니터입니다. [EdexUi-2026](https://github.com/Ali-A-Alwahed/EdexUi-2026)의 화면 배치, 테마, 화면 키보드, 오프닝과 효과음을 Rust·GTK4·VTE로 이식했습니다.

## EdexUi-2026과의 차이

아래 비교는 이식의 기준이 된 EdexUi-2026 2026.4.22의 [`2321bd5` 커밋](https://github.com/Ali-A-Alwahed/EdexUi-2026/tree/2321bd5)을 기준으로 합니다.

| 항목 | EdexUi-2026 | eDEX-Remake |
| --- | --- | --- |
| 화면과 빌드 도구 | Electron·Chromium에서 HTML/CSS와 JavaScript를 실행하며 npm과 electron-builder로 빌드 | Rust·GTK4로 화면을 구성하며 Cargo로 네이티브 실행 파일을 빌드 |
| 지원 환경 | Linux, macOS, Windows용 빌드 구성 | Linux의 Wayland 세션과 Cage를 대상으로 개발 |
| 터미널 | xterm.js와 node-pty를 Electron IPC로 연결하며 탭 5개 제공 | VTE가 로컬 PTY를 관리. 기본 탭 5개에 추가 탭, 상단 자동 숨김과 포인터 진입 시 다시 표시를 지원 |
| 터미널 모양 | 테마 JSON으로 글꼴과 색상을 지정하고 설정에서 글꼴 크기 지정 | 터미널용 글꼴, 크기, 색상 프리셋과 사용자 지정 팔레트를 설정에서 편집. 열린 탭에 즉시 적용 |
| 파일 접근 | 하단 파일 영역과 상단 파일 탐색기·설치 앱 버튼 제공 | SYSTEM에서 파일 목록과 바로가기를 전환. 활성 터미널 경로와 연동하며 상단 파일·앱 버튼을 각각 숨길 수 있음 |
| 시각 효과와 소리 | 브라우저 기반 차트와 WebGL 지구본, Howler 효과음 | GTK·Cairo 차트와 지구본, SDL3 효과음. 연속 키 입력 효과음을 겹쳐 재생 |

GTK·Cairo에서 이식한 지구본은 원본 지도 타일과 회전·초기 스캔을 사용합니다. WebGL 버전과 타일 블렌딩, 입자 효과, 글꼴 렌더링에는 차이가 있습니다. 원본 테마의 `injectCSS`는 패널 색상·투명도·글자 크기·자간·모서리, 일부 키보드 표시·패널 배치 규칙을 변환합니다. Chromium DOM 셀렉터, 미디어 쿼리와 외부 URL 규칙의 호환 범위는 제한적입니다. 화면 비교는 소프트웨어 렌더링을 사용하는 1920×1080 Cage 세션에서 진행했습니다.

## 주요 기능

- **시스템 모니터** — CPU 사용률·클럭·온도, 메모리·스왑, 저장 공간과 마운트, 가동 시간과 배터리를 표시합니다. 메모리는 40×11 도트로 사용 중·회수 가능·여유 영역을 구분합니다. 프로세스 표는 PID, 이름, CPU, 메모리 등의 열로 정렬할 수 있습니다.
- **네트워크 모니터** — 인터페이스 상태, 주소, 전송량, TCP 왕복 지연을 표시합니다. 트래픽 차트는 업로드를 기준선 위에, 다운로드를 아래에 그립니다. GeoLite2 City 데이터베이스를 연결하면 지구본에 피어 위치를 표시할 수 있습니다.
- **탭과 셸** — 마우스가 터미널 상단을 벗어난 뒤 3초가 지나면 탭 바가 접힙니다. 상단에 포인터를 올려 다시 열고 오른쪽 **+** 또는 `Ctrl+Shift+T`로 현재 디렉터리에서 새 탭을 열 수 있습니다. 탭이 많으면 목록을 가로로 스크롤할 수 있습니다.
- **파일과 앱** — SYSTEM의 **Files**는 활성 탭의 작업 경로를 따라갑니다. fish를 직접 실행할 때는 프롬프트의 경로 보고도 사용합니다. 셸 프롬프트에서 폴더를 클릭하면 입력 중인 줄을 지우고 해당 폴더로 이동합니다. 파일 검색, 숨김 파일 표시, 설치 앱 검색·실행을 지원합니다. 이미지·글꼴·PDF·오디오·비디오 미리 보기와 UTF-8 텍스트 편집을 제공합니다.
- **외관과 입력** — 원본 테마 21개, 화면 키보드 배열 18개, 부팅 로그·글리치·환영 화면으로 구성된 오프닝, 물리 키와 화면 키의 효과음을 포함합니다. 터미널 색상은 **Follow theme**, Catppuccin Latte·Frappé·Macchiato·Mocha, Nord, **Custom** 중에서 선택할 수 있습니다. **Custom**의 색상 칸을 클릭하면 텍스트·배경·커서·선택 영역과 ANSI 16색을 직접 지정할 수 있습니다. [Catppuccin](https://github.com/catppuccin/alacritty) · [Nord](https://www.nordtheme.com/docs/colors-and-palettes/)

## Arch Linux 빌드

```bash
sudo pacman -S --needed git base-devel rust gtk4 vte4 fontconfig sdl3
git clone https://github.com/ACOPS1206/eDEX-Remake.git
cd eDEX-Remake
cargo build --release --locked
```

실행 파일은 `target/release/edex-native`에 생성됩니다. 체크아웃 밖으로 옮길 때는 저장소의 `assets/` 디렉터리를 실행 파일 옆에 함께 두세요. 단위 테스트는 `cargo test --locked`로 실행할 수 있습니다.

PDF 미리 보기에는 Poppler가 필요합니다. 오디오·비디오 미리 보기에는 GTK의 GStreamer 백엔드와 해당 형식을 지원하는 플러그인이 필요합니다.

```bash
sudo pacman -S --needed poppler gst-plugins-base gst-plugins-good
```

## 실행

이미 Wayland 그래픽 세션에 로그인했다면 저장소 디렉터리에서 실행합니다.

```bash
GDK_BACKEND=wayland ./target/release/edex-native --windowed
```

전용 모니터에 전체 화면으로 띄우려면 Cage를 설치하고 물리 TTY에 일반 사용자로 로그인한 뒤 실행합니다.

```bash
sudo pacman -S --needed cage
cage -s -- env GDK_BACKEND=wayland "$PWD/target/release/edex-native"
```

| 실행 옵션 | 동작 |
| --- | --- |
| `--intro` / `--nointro` | 이번 실행에서 오프닝을 강제로 켜거나 끔 |
| `--theme tron` / `--keyboard fr-FR` | 이번 실행의 대시보드 테마와 화면 키보드 지정 |
| `--interface enp3s0` | 모니터링할 인터페이스 지정. 실제 장치 이름으로 바꿔 사용 |
| `--sample` | 창을 열지 않고 시스템 지표를 한 번 출력 |
| `--help` / `--version` | 사용법과 빌드 버전 확인 |

## 설정과 단축키

`Ctrl+Shift+S`로 설정을 열고 **Save to Disk**를 누르면 저장됩니다. **Terminal font**, **Terminal font size**, **Terminal colors**의 변경은 열린 셸과 스크롤백을 유지한 채 적용됩니다. **Show top file manager button**과 **Show top installed apps button**은 상단의 두 진입점을 각각 제어합니다. SYSTEM과 단축키에서도 두 기능을 열 수 있습니다.

설정은 `$XDG_CONFIG_HOME` 또는 `~/.config` 아래의 기존 `EdexUi-2026/settings.json`이나 `eDEX-UI/settings.json`을 사용합니다. 원본 형식의 `shortcuts.json`에서 지원되는 앱 동작과 셸 명령 단축키를 읽습니다.

| 키 | 동작 |
| --- | --- |
| `Ctrl+1`–`Ctrl+5` | 기본 터미널 탭 선택 |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 다음 / 이전 터미널 탭 |
| `Ctrl+Shift+T` | 터미널 탭 추가 |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 터미널 복사 / 붙여넣기 |
| `Ctrl+Shift+S` / `Ctrl+Shift+K` | 설정 / 단축키 |
| `Ctrl+Shift+A` / `Ctrl+Shift+F` | 설치 앱 / 파일 검색 |
| `Ctrl+Shift+L` / `Ctrl+Shift+H` | 파일 목록 / 숨김 파일 |
| `Ctrl+Shift+P` | 화면 키보드 암호 모드 |
| `F9` / `F10` / `F11` | 하단 패널 / 사이드 패널 / 전체 화면 |
| `Ctrl+Shift+Q` | 종료 |

### GeoIP 지도

설정에서 기존 GeoLite2 City 파일을 지정하거나 **Download GeoLite2 City**로 받을 수 있습니다. 다운로드는 `curl`, `tar`, `sha384sum`을 사용하며 검사 후 설정 디렉터리의 `geoIPcache`에 저장합니다. 피어 위치는 Linux TCP 테이블과 로컬 데이터베이스에서 조회합니다. 데이터베이스가 있고 인터페이스에 공개 주소가 없으면 외부 IP 조회 서비스로 자신의 엔드포인트를 찾습니다. **Map public IPv4 addresses in terminal output**을 켜면 터미널 출력에 나타나는 공개 IPv4 주소도 로컬 DB로 조회합니다. GeoLite2 데이터 제공자는 MaxMind입니다.

## 기반 프로젝트와 라이선스

EdexUi-2026은 [GitSquared/eDEX-UI](https://github.com/GitSquared/edex-ui)를 기반으로 합니다. 이 저장소의 라이선스 전문은 [LICENSE](LICENSE)에서 확인할 수 있습니다.
