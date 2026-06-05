# Getting Started — for the human

> 이 파일은 사람이 읽는 가이드입니다. AI 에이전트는 `/AGENTS.md`부터.

## 0. 사전 준비

설치할 것 (필수):
- Docker Desktop (또는 호환: Colima, Rancher Desktop)
- Rust 1.78+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Node 20+ + pnpm (`npm install -g pnpm`)
- `just` (`brew install just` 또는 `cargo install just`)

AI 코딩 에이전트 (1개 이상 — 본인 선호에 따라):
- **Claude Code** (`curl -fsSL https://claude.ai/install.sh | bash` — native installer 권장)
- **Codex CLI** (`npm install -g @openai/codex`)
- **Cursor** (https://cursor.sh — GUI 에디터)
- **Cline** (VS Code 확장)
- 또는 위 도구를 둘 이상 동시 사용

선택:
- Ollama (`brew install ollama`) — 로컬 LLM
- LM Studio — 로컬 LLM GUI

> **Docker 없이 개발하기 (frontend / API 작업)**
> Docker는 *작업(task) 실행*(샌드박스 컨테이너 생성)과 샌드박스 통합 테스트에만
> 필수입니다. 컨트롤 플레인은 Docker/Redis 없이도 부팅됩니다(`SandboxClient`는
> Docker에 지연 연결, Redis는 graceful degrade). frontend/UI 또는 `/v1` REST
> 작업만 한다면:
> ```bash
> just dev-server-nodocker   # SQLite 기반 /v1 API on :3000 (Docker 불필요)
> just dev-ui                # Dioxus UI (dx 필요: cargo install dioxus-cli)
> ```
> 단, 실제 작업 실행은 여전히 Docker가 필요합니다.

API 키 1개 이상:
- Anthropic ($) — 강력 추천
- OpenAI ($)
- Google AI ($, 무료 tier 있음)
- OpenRouter ($) — 200+ 모델 한 키로

## 1. 초기 설정 (Phase -1)

```bash
# 1) Repo 초기화 (수동)
mkdir seasoned-hand && cd seasoned-hand

# 2) 이 init 디렉토리의 모든 파일을 복사
tar xzf ~/Downloads/seasoned-hand-init.tar.gz

# 3) Git 초기 커밋
git init
git branch -M main
git add .
git commit -m "chore: initial scaffold"
# git remote add origin git@github.com:slowdoctor-dev/seasoned-hand.git
# git push -u origin main

# 4) 환경 변수
cp .env.example .env
$EDITOR .env  # API 키 채우기

# 5) 디렉토리 구조 검증
just status  # Phase 정보 출력
```

## 2. AI 도구별 설정 (사용할 것만)

### Claude Code 사용 시

이미 작동합니다. CLAUDE.md가 자동으로 AGENTS.md를 import합니다.

```bash
# Fresh 세션 시작
claude code

# 또는 특정 폴더에서
cd seasoned-hand && claude code
```

### Codex CLI 사용 시

```bash
# 글로벌 설정 (선택)
mkdir -p ~/.codex
cp .codex/config.toml.example ~/.codex/config.toml
$EDITOR ~/.codex/config.toml  # 모델·프로필 조정

# 프로젝트에서 시작
cd seasoned-hand
codex                       # default profile
codex --profile fast        # 빠른 이터레이션
codex --profile review      # read-only 리뷰
codex --profile story       # story 구현 (workspace-write)
```

Codex는 AGENTS.md를 자동으로 읽습니다. CLAUDE.md는 무시(또는 fallback).

### Cursor 사용 시

Cursor 0.50+ 는 AGENTS.md를 네이티브로 읽습니다. 추가 설정 불필요.

### 둘 이상 동시 사용

`docs/using-claude-and-codex.md` 참고. 추천 패턴:
- Claude Code: story 구현 (복잡한 다중 파일)
- Codex CLI: 빠른 수정·sandbox 실험
- 막힐 때 다른 도구로 시도 (다른 모델 = 다른 관점)

## 3. Phase 0 시작 — BMAD 방식

각 단계는 **fresh AI 세션**에서 진행 (이전 컨텍스트 폐기).

### 3.1 Analyst 페르소나 (요구사항 명확화)

```bash
# 사용하는 도구로 fresh 세션 시작
claude code           # Claude Code
# 또는
codex --profile fast  # Codex
# 또는
cursor .              # Cursor

# 페르소나 활성화 — 다음 명령으로 프롬프트 표시
just analyst-prompt
```

표시된 내용을 AI에게 붙여넣기. AI가 BMAD Analyst로 동작.

**Phase 0은 이미 작성됨**:
```
"Phase 0 requirements.md는 이미 작성되어 있습니다.
검토하고 필요하면 보완해주세요."
```

### 3.2 Architect 페르소나 (기술 설계)

```bash
# 새 fresh 세션
just architect-prompt
```

AI에게 붙여넣고:
```
"Phase 0 architecture를 /specs/phase-0/architecture.md에 작성해주세요.
requirements.md는 완성되어 있습니다."
```

### 3.3 PM 페르소나 (스토리 분해)

```bash
# 새 fresh 세션
just pm-prompt
```

AI에게:
```
"Phase 0 architecture를 27개 story로 분해해주세요.
story-0.1.md는 이미 작성되어 있으니, story-0.2 ~ story-0.27을
_template.md 양식으로 만들어주세요."
```

## 4. Phase 0 구현 — GSD 방식

각 story마다 fresh 세션:

```bash
# 사용하는 AI 도구로
claude code  # 또는 codex, cursor 등
just story-prompt
```

AI에게:
```
"Implement story 0.1"
```

AI가:
1. 파일들 읽음 (/AGENTS.md, /specs/01-architecture/ARCHITECTURE.md, story 파일)
2. Discuss (불명확한 점 질문)
3. Plan 출력 → 사용자 OK 대기
4. Execute (구현 + 테스트)
5. `just verify` 실행
6. Commit + 상태 갱신

## 5. 진행 상황 확인

```bash
just status
```

출력 예시:
```
Current phase: 0
Stories: 27 total
  Done:        3
  In-progress: 1
  Ready:       23
  Blocked:     0
```

## 6. 진행 중 막힐 때

### 코드 vs 스펙이 다를 때

```
1. 멈춤
2. 스펙을 어떻게 업데이트할지 결정
3. 같은 커밋에 스펙 + 코드 변경
4. 계속
```

### Story가 너무 큼

```
1. 멈춤
2. PM 페르소나로 재분해
3. Story X.Y를 X.Y.a, X.Y.b로 분리
4. 원래 story는 archive에 옮김
```

### 한 AI가 헤맴

증상: 같은 코드 반복 수정, 새 디버그 코드 추가, 요청 안 한 기능 추가.

원인: 컨텍스트 부족 또는 스펙 모호.

해결 (시도 순서):
```
1. 세션 종료, fresh 세션 시작
2. 스펙 다시 검토 — 모호한가?
3. 필요시 스펙 수정 (BMAD Architect 페르소나로)
4. 다른 AI 도구로 시도 (Claude → Codex 또는 그 반대)
5. 그래도 안 되면 story를 더 잘게 쪼개기
```

### AI 도구 자체 문제

증상: 특정 도구에서만 실패, 다른 도구에선 잘 됨.
- AGENTS.md가 너무 길어서 잘림 → 300줄 이내 유지
- 도구별 sandbox 정책 차이 → `docs/using-claude-and-codex.md` 참고
- 모델 capabilities 차이 → 다른 도구 시도

## 7. Phase 종료 시

각 Phase 마지막 story는 항상 "통합 테스트":

```
- 전체 phase의 acceptance criteria 검증
- E2E 테스트 작성
- /specs/phase-N/retrospective.md 작성
- 다음 phase 계획 시작
```

## 8. 일상 워크플로우

```bash
# 아침 — 진행 상황 확인
just status

# 작업 시작 — fresh AI 세션 (선호 도구로)
claude code   # 또는 codex, cursor
just story-prompt
# "Implement story 0.X"

# 작업 중 — 자동 검증
just verify  # (AI가 알아서 실행)

# 작업 끝 — 커밋
git push

# 다음 작업 — fresh 세션 (도구 바꿔도 OK)
```

## 9. 주의사항

- ❌ 한 세션에 여러 story 처리 (컨텍스트 오염)
- ❌ Story 외 작업 추가 (scope creep)
- ❌ 스펙 안 보고 코드 작성
- ❌ verify 게이트 우회
- ❌ AGENTS.md를 300줄 초과로 (Codex가 잘라먹음)
- ❌ Claude 전용 기능을 코드에 의존 (Codex로 안 돌아감)
- ✅ 막히면 BMAD 페르소나로 돌아가서 스펙 보완
- ✅ 매일 `just status` 확인
- ✅ Story 추정이 2x 이상 빗나가면 PM 페르소나로 재분해
- ✅ 도구 바꿔보기 (Claude ↔ Codex)는 디버깅 기법

## 10. 성공 지표

- 매주 5-10개 story 완료
- `just verify` 항상 통과
- Phase별 timeline 안에서
- 누적 commit history가 phase별로 깔끔
- 어떤 외부인이 봐도 `/specs` 읽으면 시스템 이해 가능
- 어떤 AI 도구를 써도 같은 결과 (도구 종속성 없음)
