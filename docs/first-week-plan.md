# Seasoned Hand — First Week Plan

> 첫 7일 행동 가이드. SDD 시작용.
>
> **단일 진입점은 `/BASELINE.md`입니다.** 모든 결정사항·아키텍처·로드맵은 거기.
> 이 파일은 **첫 주 행동만** 다룹니다.

---

## 0. 어디까지 왔는가

✅ **컨셉 정의**: Digital Employee = Manus 깊이 + Hermes 시간 축 학습
✅ **아키텍처 확정**: Rust + Axum + Tokio + Rig backend, Next.js frontend, Bifrost gateway, AIO Sandbox, SQLite + Redis
✅ **방법론**: Spec-Driven Development (BMAD phase / GSD daily)
✅ **이름**: Seasoned Hand
✅ **태글라인**: *Every task makes the hand wiser.*
✅ **라이선스**: MIT
✅ **호스팅**: 개인 GitHub 계정 (나중에 org 이전)
✅ **초기 스캐폴드**: 32 파일 (이 init 디렉토리)
✅ **Phase 0 requirements**: 27 stories 식별

🔜 **다음 단계**: Phase 0 architecture → 26개 story 작성 → 구현 시작

---

## 1. 모든 핵심 결정 한 페이지

### 정체성
| 항목 | 값 |
|---|---|
| 프로젝트명 | Seasoned Hand |
| 형식 | `seasoned-hand` (kebab) / `Seasoned Hand` (prose) |
| 태글라인 | Every task makes the hand wiser. |
| 한국어 보조 | 매 작업이 손을 더 영리하게 |
| 라이선스 | MIT |
| 호스팅 | `github.com/slowdoctor-dev/seasoned-hand` |
| 정체성 | Digital Employee (not assistant) |

### 기술 스택
| Layer | 선택 |
|---|---|
| LLM Gateway | Bifrost (Go) — 11μs 오버헤드 |
| Control plane | Rust + Axum + Tokio + Rig |
| Frontend | Next.js 15 + Tailwind v4 + React 19 |
| Sandbox | AIO Sandbox (Docker per session) |
| DB | SQLite WAL + Redis |
| Model routing | 12-slot (3 main + 9 auxiliary) |
| Tool catalog | 32+ (Manus 29 + 3 추가) |
| Event types | 7 (Message/Action/Observation/Plan/Knowledge/Datasource/Skill) |

### 6-Phase 로드맵
| Phase | 주 | 산출 | 학습 |
|---|---|---|---|
| 0 | 3 | Foundation skeleton | (없음) |
| 1 | 4 | Manus 5-layer (deep execution) | (없음) |
| 2 | 3 | Employee interface (briefing, deliverables) | (없음) |
| 3 | 4 | 4-layer learning (SOPs/Playbooks/History/Glossary) | **시작** |
| 4 | 3 | Curator + self-improvement | (성숙) |
| 5 | 3 | Multi-user + organization | (확장) |
| 6 | 2 | Open source release | (출시) |

**총 22주 = 약 5개월** (풀타임 기준)

### 방법론
- BMAD 페르소나 (phase 시작): Analyst → Architect → PM
- GSD 워크플로우 (daily): Discuss → Plan → Execute → Verify
- 한 story = 1~3시간 = 1 PR = 1 commit
- 매 story마다 fresh AI 세션 (컨텍스트 폐기)
- 모든 결정은 `/specs/` 마크다운에 (외부 상태)

---

## 2. 첫 7일 액션 플랜

### Day 1 — 환경 + GitHub (2시간)

**오전**:
- [ ] 압축 풀기: `mkdir seasoned-hand && cd seasoned-hand && tar xzf ~/Downloads/seasoned-hand-init.tar.gz`
- [ ] `git init && git branch -M main`
- [ ] GitHub Private repo 생성 (개인 계정)
- [ ] 첫 commit + push (메시지는 `docs/setup-checklist.md` 참고)

**오후**:
- [ ] 로컬 환경 점검 (Docker, Rust, Node, pnpm, just, AI 코딩 도구 1개 이상)
- [ ] `.env` 작성 (API 키 최소 1개)
- [ ] 도메인 등록: `seasonedhand.dev` ($12, Cloudflare Registrar)
- [ ] `just status` 작동 확인

### Day 2 — Phase 0 Architecture (BMAD Architect, 2시간)

목적: Story들이 의존하는 기술적 뼈대 작성. Story 0.1만 있고 26개는 미작성 상태에서 architecture가 필요.

```bash
# 선호하는 AI 도구로 fresh 세션 시작
claude code             # Claude Code 사용 시
# 또는: codex            # Codex CLI 사용 시
# 또는: cursor .         # Cursor 사용 시

just architect-prompt
# AI에게 표시된 프롬프트 붙여넣기 후:
# "Phase 0 architecture를 /specs/phase-0/architecture.md에 작성해주세요.
#  requirements.md는 완성되어 있고, 27 stories가 식별되어 있습니다.
#  /specs/01-architecture/ARCHITECTURE.md의 결정사항(Bifrost, Rust, Next.js 등)을
#  Phase 0 수준에서 구체화하면 됩니다."
```

산출: `/specs/phase-0/architecture.md` (12 섹션, 약 500줄)

### Day 3 — Phase 0 Stories 분해 (BMAD PM, 3시간)

```bash
# fresh AI 세션 (어떤 도구든)
claude code  # 또는 codex, cursor

just pm-prompt
# AI에게:
# "Phase 0 architecture를 27개 story로 분해해주세요.
#  requirements.md §4에 story 테이블이 이미 있고,
#  story-0.1.md는 이미 작성되어 있습니다.
#  story-0.2 ~ story-0.27을 _template.md 양식으로 만들어주세요."
```

산출: `/specs/phase-0/stories/story-0.2.md` ~ `story-0.27.md`

검증: `just status` → "Stories: 27 total, Ready: 26" 같은 출력

### Day 4 — Story 0.1 구현 (Bifrost, 2시간)

첫 GSD 사이클. **이게 중요**: 도구·방법론이 실제로 작동하는지 검증.

```bash
# fresh AI 세션
claude code  # 또는 codex --profile story, 또는 cursor

just story-prompt
# AI에게: "Implement story 0.1 (Bifrost Docker setup)"
```

순서: Discuss → Plan → "go" → Execute → Verify → Commit → PR → Merge

산출: `bifrost/config.yaml`, `docker-compose.yml` 갱신, `scripts/test-bifrost.sh` 작동

### Day 5 — Story 0.2, 0.3 (Rust 초기화 + SQLite, 4시간)

여기서 **Rust 학습 곡선** 본격 시작. 막히면 AI에게 "rust beginner" 컨텍스트 명시. 한 도구가 막히면 다른 도구로 시도 (Claude ↔ Codex).

### Day 6 — Story 0.4, 0.5 (Event Stream + Redis pub/sub, 4시간)

기반 데이터 모델. 이게 잘 잡혀야 이후 모든 게 깔끔.

### Day 7 — Retrospective (1시간)

```
- 이번 주 완료 story: ___개
- 막힌 곳: ___
- 추정 시간 vs 실제 차이: ___배
- 다음 주 조정: ___
```

`/docs/retros/week-01.md`에 기록.

---

## 3. SDD 흐름 한 다이어그램

```
┌──────────────────────────────────────────────────────────┐
│  Phase N 시작                                              │
└────────────┬─────────────────────────────────────────────┘
             │
             ▼
   ┌──────────────────┐
   │ BMAD Analyst     │  requirements.md 작성·검토
   │ (fresh session)  │
   └────────┬─────────┘
            │
            ▼
   ┌──────────────────┐
   │ BMAD Architect   │  architecture.md 작성
   │ (fresh session)  │
   └────────┬─────────┘
            │
            ▼
   ┌──────────────────┐
   │ BMAD PM          │  story-N.1.md ~ story-N.M.md 작성
   │ (fresh session)  │
   └────────┬─────────┘
            │
            ▼
   ┌──────────────────┐
   │ Story-by-Story   │  매 story마다:
   │ Implementation   │   - fresh session
   │ (GSD workflow)   │   - Discuss → Plan → Execute → Verify
   │                  │   - 1 PR = 1 commit
   └────────┬─────────┘
            │
            ▼
   ┌──────────────────┐
   │ Phase N 종료     │  /specs/phase-N/retrospective.md
   │ Retrospective    │  → Phase N+1 시작
   └──────────────────┘
```

---

## 4. 주의사항 — 자주 빠지는 함정

### 함정 1: 한 세션에서 여러 story
**증상**: "내친 김에 0.2도 해버리자"
**결과**: 컨텍스트 오염, spec 무시
**대처**: 매 story 후 `/clear` 또는 새 터미널

### 함정 2: 코드 vs 스펙 drift
**증상**: 코드 짜다 보니 spec과 다름. 일단 계속 짜고 나중에 spec 갱신
**결과**: drift 누적, 누가 source of truth인지 모름
**대처**: 발견 즉시 멈춰서 spec 먼저 갱신, 같은 commit에 포함

### 함정 3: Vibe로 돌아감
**증상**: "AI한테 다 시키면 더 빠를 텐데"
**결과**: 3개월 벽에 부딪힘
**대처**: BMAD 페르소나 단계 절대 우회 X. SDD가 길어 보여도 결국 빠름

### 함정 4: Phase 안에서 architecture 변경
**증상**: Phase 1 진행 중 Phase 0 architecture 다시 짜고 싶음
**결과**: 끝나지 않는 리팩토링
**대처**: 다음 Phase의 retrospective에 기록, Phase 끝나고 한꺼번에

### 함정 5: 학습 시스템 조기 구현
**증상**: Phase 1에서 "playbook 미리 좀 만들어볼까"
**결과**: Phase 3가 와도 학습 작동 안 함 (검증 안 된 데이터)
**대처**: **순서를 지킨다**. Phase 0~2는 학습 0, Phase 3에서 시작

### 함정 6: 너무 많이 ask
**증상**: 매 결정마다 사용자에게 묻기
**결과**: 직원이 아니라 비서
**대처**: spec에 "ask required" 명시 안 된 건 자율 진행, accountability trail 남기기

---

## 5. AI 도구 사용 체크리스트

매 세션 시작 시:

- [ ] Fresh 세션인가? (이전 세션 잔재 0)
- [ ] `/AGENTS.md` 읽었는가?
- [ ] `/specs/01-architecture/ARCHITECTURE.md` 읽었는가?
- [ ] 작업할 story 명시했는가?
- [ ] Plan을 받았는가? (Execute 전)
- [ ] Plan에 "go" 한 후 시작했는가?

매 세션 종료 시:

- [ ] `just verify` 통과했는가?
- [ ] Story status 갱신했는가? (`done`)
- [ ] Commit 메시지가 spec과 일치하는가?
- [ ] PR 열었는가?

---

## 6. 한 줄 요약

> Phase 0 시작 직전. 모든 결정 끝남. 다음 행동은 단 두 가지: 
> (a) GitHub repo 생성 + 첫 push, (b) BMAD Architect 페르소나로 Phase 0 architecture 작성. 
> 그 다음은 26개 story 자동 흐름.

`Every task makes the hand wiser.` — **이번 주에 첫 손자국이 찍힙니다**.

---

## 7. 참고 — 모든 산출물 위치

| 파일 | 역할 |
|---|---|
| `README.md` | 외부 진입점 |
| `AGENTS.md` | AI 에이전트 source of truth |
| `CLAUDE.md` | Claude 전용 추가 |
| `LICENSE` | MIT |
| `CONTRIBUTING.md` | 기여 흐름 |
| `CODE_OF_CONDUCT.md` | 행동 강령 |
| `SECURITY.md` | 보안 정책 |
| `docker-compose.yml` | Bifrost + Redis 스켈레톤 |
| `justfile` | 작업 자동화 |
| `.env.example` | 환경변수 템플릿 |
| `.gitignore` | git 무시 |
| `.codex/config.toml.example` | Codex 설정 |
| `.github/ISSUE_TEMPLATE/*` | GitHub 템플릿 |
| `.github/workflows/ci.yml` | CI 자동화 |
| `specs/01-architecture/ARCHITECTURE.md` | **불변** 아키텍처 |
| `specs/phase-0/requirements.md` | Phase 0 요구사항 (27 stories) |
| `specs/phase-0/stories/story-0.1.md` | 첫 story (완성) |
| `specs/phase-0/stories/_template.md` | Story 양식 |
| `docs/manifesto.md` | 왜 존재하는가 |
| `docs/brand.md` | 시각·언어 정체성 |
| `docs/methodology.md` | SDD + BMAD + GSD 상세 |
| `docs/getting-started.md` | 인간 온보딩 |
| `docs/setup-checklist.md` | 도메인·계정 확보 |
| `docs/using-claude-and-codex.md` | 두 도구 함께 쓰기 |
| `docs/first-week-plan.md` | **이 파일** |
| `prompts/bmad-analyst.md` | Analyst 페르소나 |
| `prompts/bmad-architect.md` | Architect 페르소나 |
| `prompts/bmad-pm.md` | PM 페르소나 |
| `prompts/gsd-execute-story.md` | 일상 story 구현 |
| `scripts/spec-check.sh` | spec 검증 |
| `scripts/status.sh` | 진행 상황 |

총 **33 파일**.
