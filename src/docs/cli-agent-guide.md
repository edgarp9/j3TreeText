# j3TreeText CLI Agent Guide

이 문서는 다른 AI 에이전트가 `j3TreeTextCli`로 SQLite 문서 트리를 안전하게 조회, 생성, 편집하기 위한 실행 지침이다.

## 기본 원칙

- 항상 작업 대상 DB를 확인하고 가능하면 `--db <path>`를 명시한다.
- 변경 전 `tree` 또는 `show`로 현재 상태를 확인한다.
- `delete`는 soft delete이지만, `purge`는 복구 불가능한 영구 삭제이므로 사용자가 명시적으로 요청한 경우에만 실행한다.
- `edit`는 현재 DB의 최신 `updated_at`을 기준으로 조건부 저장한다. 다른 프로세스가 먼저 같은 문서를 바꾸면 저장 충돌 오류로 실패한다.
- 실행 실패 시 stderr의 사용자 메시지와 `detail:` 줄을 함께 보고한다.

## 실행 형식

소스 트리에서 실행:

```powershell
cargo run --bin j3TreeTextCli -- --db <db-path> <command> [args]
```

빌드된 바이너리로 실행:

```powershell
target\debug\j3TreeTextCli.exe --db <db-path> <command> [args]
```

도움말:

```powershell
cargo run --bin j3TreeTextCli -- --help
```

`--db <path>`는 전역 옵션이므로 명령 앞에 둔다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db tree
```

## 권장 작업 흐름

1. 대상 DB 확인

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db tree
```

2. 수정할 문서 확인

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db show 3
```

3. 변경 실행

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db edit 3 --content "new content"
```

4. 결과 확인

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db show 3
```

## 명령 참조

### tree

활성 문서 트리를 출력한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db tree
```

출력 예:

```text
- [1] Root
  - [2] Untitled
  - [3] CLI Note
```

### show

문서 메타데이터와 본문을 출력한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db show <node_id>
```

휴지통 문서를 직접 조회:

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db show --trash <node_id>
```

출력 예:

```text
id: 3
parent_id: 1
title: CLI Note
created_at: 2026-05-01T07:50:07.302Z
updated_at: 2026-05-01T07:50:08.429Z
content:
hello
```

### create

지정 부모 아래 새 문서를 만든다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db create --parent <parent_id> --title "<title>"
```

초기 본문을 함께 저장:

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db create --parent 1 --title "Draft" --content "hello"
```

파일에서 본문 읽기:

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db create --parent 1 --title "Draft" --content-file .\draft.txt
```

stdin에서 본문 읽기:

```powershell
Get-Content .\draft.txt -Raw | cargo run --bin j3TreeTextCli -- --db .\sample.db create --parent 1 --title "Draft" --stdin
```

### edit

문서 본문을 교체한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db edit <node_id> --content "new content"
```

파일 내용으로 교체:

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db edit <node_id> --content-file .\body.txt
```

stdin 내용으로 교체:

```powershell
Get-Content .\body.txt -Raw | cargo run --bin j3TreeTextCli -- --db .\sample.db edit <node_id> --stdin
```

기존 본문 뒤에 덧붙이기:

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db edit <node_id> --content "`nappend text" --append
```

### rename

문서 제목을 바꾼다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db rename <node_id> --title "New Title"
```

같은 부모 아래 활성 문서와 제목이 중복되면 실패한다.

### search

활성 문서의 제목과 본문을 검색한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db search "keyword"
```

출력 예:

```text
[3] CLI Note (parent: Root)
```

### delete

문서와 활성 하위 문서를 soft delete한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db delete <node_id>
```

루트 문서 `[1] Root`는 삭제할 수 없다.

### trash

휴지통 문서를 출력한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db trash
```

출력 예:

```text
- [3] CLI Note (deleted: 2026-05-01T07:50:08.091Z)
```

### restore

휴지통 문서와 삭제된 하위 문서를 복원한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db restore <node_id>
```

원래 부모가 활성 상태이면 원래 부모 아래로 복원하고, 원래 부모가 없거나 삭제 상태이면 루트 아래로 복원한다.

### purge

휴지통 문서와 하위 문서를 영구 삭제한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db purge <node_id>
```

이 작업은 되돌릴 수 없다. 사용자가 영구 삭제를 명시적으로 요청한 경우에만 실행한다.

### move

문서를 다른 부모 문서의 마지막 자식으로 이동한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db move <node_id> --parent <parent_id>
```

루트 문서는 이동할 수 없고, 문서를 자기 자신이나 자기 하위 문서 아래로 이동할 수 없다.

### move-up / move-down

같은 부모 아래에서 문서를 한 칸 위 또는 아래로 이동한다.

```powershell
cargo run --bin j3TreeTextCli -- --db .\sample.db move-up <node_id>
cargo run --bin j3TreeTextCli -- --db .\sample.db move-down <node_id>
```

## 본문 입력 규칙

`create`와 `edit`는 다음 본문 입력 옵션 중 하나만 받을 수 있다.

- `--content <text>`
- `--content-file <path>`
- `--stdin`

`create`에서 본문 입력 옵션을 생략하면 빈 본문 문서를 만든다.

현재 `--content-file`과 `--stdin`은 CLI 경계에서 UTF-8 문자열로 읽는다. GUI의 import/export 인코딩 선택 기능과는 별개다.

## 오류 처리 가이드

명령 실패 시 프로세스는 non-zero exit code로 종료하고 stderr에 다음 형태를 출력한다.

```text
<사용자 메시지>
detail: <내부 오류 상세>
```

에이전트는 사용자에게 최소한 다음을 보고한다.

- 실행한 명령
- 사용자 메시지
- `detail:` 줄
- 변경이 적용되지 않았을 가능성

대표 오류:

- 같은 부모 아래 제목 중복: 제목을 바꾸거나 다른 부모를 선택한다.
- 저장 충돌: `show <node_id>`로 최신 본문을 다시 확인한 뒤 사용자에게 덮어쓰기 또는 새 문서 저장 방향을 확인한다.
- 노드 없음: `tree` 또는 `trash`로 현재 노드 ID를 다시 확인한다.
- 영구 삭제 대상 아님: 먼저 `trash`로 삭제 상태를 확인한다.

## 안전한 에이전트 체크리스트

- 변경 전 `tree` 또는 `show`를 실행했다.
- 대상 `node_id`와 `parent_id`를 출력에서 확인했다.
- `--db` 경로가 사용자가 의도한 DB다.
- `purge`는 사용자가 영구 삭제를 명시한 경우에만 실행했다.
- 변경 후 `tree`, `show`, `search`, `trash` 중 적절한 조회 명령으로 결과를 확인했다.
