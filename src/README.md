# j3TreeText

j3TreeText는 SQLite 파일에 트리형 문서를 저장하는 데스크톱 편집기와 CLI입니다.

## Linux 배포

Linux GUI 배포에는 다음 파일을 실행 파일과 같은 폴더에 둡니다.

- `j3TreeText`
- `icon.svg`
- `icon.png` fallback 권장

사용자 영역 desktop entry와 hicolor 아이콘은 명시적 명령으로만 등록합니다.

```bash
./j3TreeText --install
./j3TreeText
./j3TreeText --uninstall
```

인자 없이 실행하면 기본 DB를 열고, DB 경로 인자 1개를 주면 해당 파일을 문서 저장소로 사용합니다.

## 라이선스 고지

배포 산출물에는 `LICENSE`, `about.txt`, `THIRD_PARTY_NOTICES.txt`를 함께 포함합니다.
의존성 또는 포함 리소스 변경 후에는 `python tools/generate_third_party_notices.py`로 고지 목록을 갱신하고, About 창의 고지 파일 안내가 실제 배포 파일과 일치하는지 확인합니다.
