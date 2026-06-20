# 릴리스 노트 초안

## Rich Edit 전환 안정화 릴리스

- 본문 편집기 내부 구현은 Rich Edit로 변경되었지만 저장 형식은 plain text 유지.
- 문서 본문은 기존과 같이 SQLite `nodes.content`에 UTF-8 plain text로 저장된다.
- 검색창은 기존 Win32 Edit Control을 유지하며, 본문 편집기 설정과 독립적으로 동작한다.
- 한글, 이모지, CRLF 여러 줄 문서, 찾기/바꾸기, 저장 후 재열기, word wrap, 글꼴, 테마, 읽기 전용 탭 동작을 릴리스 전 고정 체크리스트로 검증한다.
