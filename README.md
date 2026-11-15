# eDEX-Remake 0.5.1

[EdexUi-2026](https://github.com/Ali-A-Alwahed/EdexUi-2026) 커밋 `2321bd5`과 그 위에 적용한 IPC·레이아웃 패치를 기준으로 삼았습니다. GTK4가 화면과 입력을 처리하고, VTE가 각 탭의 로컬 PTY를 처리합니다. 이 저장소에는 Rust 소스, 원본 테마와 미디어, 프로젝트 라이선스가 들어 있습니다.

## Arch Linux에서 실행

저장소를 받은 뒤 아래 명령으로 빌드합니다. 배포할 때는 실행 파일 옆에 `assets` 디렉터리를 함께 둡니다. Cage는 물리 TTY에서 일반 사용자로 실행합니다.

```bash
sudo pacman -S --needed rust base-devel gtk4 vte4 fontconfig sdl3 cage
cargo build --release --locked
cage -s -- env GDK_BACKEND=wayland "$PWD/target/release/edex-native"
```

효과음은 SDL3의 기본 오디오 장치에서 재생합니다. 같은 효과음이 끝나기 전에 다시 입력해도 두 소리가 재생됩니다. 미디어 미리 보기는 GTK GStreamer, PDF 미리 보기는 Poppler를 사용합니다.

```bash
sudo pacman -S gst-plugins-base gst-plugins-good poppler
```

그래픽 세션이 이미 열려 있으면 `./edex-native --windowed`를 사용할 수 있습니다. `--theme tron`, `--keyboard fr-FR`, `--interface enp3s0`, `--intro`, `--nointro`, `--sample`을 지원합니다. `--sample`은 창 없이 지표를 한 번 읽습니다. `--intro`는 저장된 시작 화면 건너뛰기 설정을 해당 실행에서 덮어씁니다.

## SYSTEM과 상단 버튼

- SYSTEM 아래의 **Files** 또는 **File Manager**를 누르면 활성 터미널 탭의 디렉터리를 열고 이후 경로를 따라갑니다. fish는 프롬프트에서 작업 경로를 보고합니다. 폴더를 누르면 셸 프롬프트에 입력 중인 줄을 지우고 해당 디렉터리로 이동합니다.
- **Shortcuts**를 누르면 설정, 테마, 키보드, 프로젝트와 설치 앱 항목으로 돌아갑니다.
- 설정에서 **Show top file manager button**과 **Show top installed apps button**을 각각 끌 수 있습니다. 두 버튼을 꺼도 SYSTEM과 키보드 단축키에서 접근할 수 있습니다.

## 이식 범위

- 원본 레이아웃의 열과 하단 패널 비율을 Cage 1920×1080 캡처로 맞췄습니다. 원본 테마 JSON 21개, 키보드 JSON 18개, 세계 지도 타일, 시작 로그, 효과음을 포함합니다. 원본 글꼴은 네이티브 로딩을 위해 TTF로 변환했습니다.
- 원본의 다섯 터미널 탭, 파일 패널과 셸 경로 연동, 파일·앱 검색, 설치 앱 실행, 파일 목록과 숨김 파일 전환, 마운트 디스크 목록을 지원합니다. 파일 미리 보기에는 PDF, 글꼴, 이미지, 오디오, 비디오와 일반 UTF-8 텍스트 편집기가 포함됩니다.
- 네트워크 그래프는 상하 대칭 축에 업로드와 다운로드를 표시하며, 메모리는 원본과 같은 40×11의 활성·회수 가능·유휴 도트로 표시합니다. 시작 화면은 부팅 로그, 로고와 글리치, 환영 화면 순서로 재생합니다.
- 전체 프로세스 표에서 PID, 이름, 사용자, CPU, 메모리, 상태, 시작 시각, 실행 시간을 정렬할 수 있습니다. 네트워크 모듈은 인터페이스별 상태, 전송량, TCP 왕복 지연과 GeoIP 핀을 표시합니다.
- 화면 키보드는 원본 문자 조합표와 제어 시퀀스를 사용합니다. 원본 형식의 `shortcuts.json`에 있는 지원되는 앱 동작과 셸 명령 단축키를 읽습니다.
- `Ctrl+Shift+S`로 설정을 편집합니다. 기존 XDG 설정 디렉터리의 `EdexUi-2026/settings.json` 또는 `eDEX-UI/settings.json`에 변경한 값을 저장합니다. 테마, 화면 키보드, 상단 버튼 표시 여부는 즉시 적용합니다. 실행 중인 셸과 스크롤백을 유지합니다. 다른 설정은 다시 실행할 때 적용합니다. 하단의 `themes`, `keyboards`에서도 바로 바꿀 수 있습니다.

| 키 | 동작 |
| --- | --- |
| Ctrl+1~5 | 터미널 탭 선택 |
| Ctrl+Tab / Ctrl+Shift+Tab | 다음 / 이전 터미널 탭 |
| Ctrl+Shift+C / V | 터미널 복사 / 붙여넣기 |
| Ctrl+Shift+S / K | 설정 / 단축키 |
| Ctrl+Shift+A / F | 설치 앱 / 파일 검색 |
| Ctrl+Shift+L / H | 파일 목록 / 숨김 파일 |
| Ctrl+Shift+P | 화면 키보드 암호 모드 |
| F9 / F10 / F11 | 하단 패널 / 사이드 패널 / 전체 화면 |
| Ctrl+Shift+Q | 종료 |

### GeoIP 지도

`Download GeoLite2 City` 버튼은 원본이 사용하던 미러에서 데이터베이스를 가져오고 SHA-384를 검증한 다음 앱 설정의 `geoIPcache` 아래에 저장합니다. 기존 GeoLite2 City 경로를 설정에서 지정할 수도 있습니다. 피어 IP는 Linux TCP 테이블에서 읽어 로컬 DB로 조회합니다. DB가 있고 인터페이스에 공개 주소가 없으면, 원본과 같은 외부 IP 조회 서비스를 사용해 자체 엔드포인트를 찾습니다. GeoLite2 데이터 제공자는 MaxMind입니다. `Map public IPv4 addresses in terminal output`을 켜면, 터미널에서 새로 찾은 공개 IPv4 주소도 로컬 DB에서 조회해 잠깐 핀으로 표시합니다.

### 화면과 동작 차이

지구본은 원본 타일, 45초 회전, 50도 원근 카메라, 초반 경도 스캔, 거리 안개, 위성 신호를 GTK·Cairo에서 구현합니다. WebGL 버전과 타일 색의 분포, 블렌딩과 세부 입자 효과에는 차이가 있습니다.

기존 테마의 `injectCSS`에서 지원하는 효과는 기본 패널의 색상·투명도·글자 크기·자간·둥근 모서리, 내장 테마가 쓰는 키보드 숨김과 패널 배치입니다. Chromium DOM, 미디어 쿼리, 외부 URL을 쓰는 규칙에는 호환 범위의 차이가 있습니다.

비교는 소프트웨어 렌더링을 사용하는 중첩 Cage에서 진행했습니다. GP66 물리 디스플레이의 240Hz 출력과 스피커는 별도 검증이 필요합니다.

## 소스 빌드

```bash
sudo pacman -S rust base-devel gtk4 vte4 fontconfig sdl3
cargo build --release --locked
cargo test --locked
```

실행 파일은 `target/release/edex-native`에 생성됩니다. EdexUi-2026의 기반 프로젝트는 [GitSquared/eDEX-UI](https://github.com/GitSquared/edex-ui)입니다.
