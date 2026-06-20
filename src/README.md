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
