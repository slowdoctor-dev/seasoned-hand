# Vibe Coding 2026 — 큰 프로젝트를 위한 방법론

> 우리 프로젝트(Seasoned Hand = 마누스 + 헤르메스)에 맞춰 정리. SDD(Spec-Driven Development) 기반 + 깊이별 vibe 강도 조절.

---

## 0. 결론 한 줄

**Spec-Driven Development 채택, BMAD 방법론 + GSD 프레임워크 + AI 코딩 에이전트(Claude Code, Codex CLI, Cursor 등)를 우리 환경에 맞게 조합**.

이유:
- 22주 5개월짜리 큰 프로젝트는 vibe 만으로 불가능 (3개월 벽)
- BMAD가 가장 성숙한 다중 에이전트 SDD 방법론 (12+ 전문 에이전트, MIT)
- GSD가 가장 가볍고 5개월 만에 61K stars, AI 에이전트 친화적
- 두 방법론 결합 시 우리 프로젝트 규모에 최적

---

## 1. 2026 시점의 합의

### 1.1 Vibe Coding 한계 (실증)

**3개월 벽**: 빠른 프로토타입은 가능하지만 3개월 즈음 기술 부채가 압도적으로 누적.

**METR 2025 RCT**: 경험 많은 오픈소스 개발자가 AI 도구 사용 시 **24% 빠를 거라 예측 → 실제 19% 느림**. 사후에도 20% 빨랐다고 믿음.

**Karpathy 원 정의**: "throwaway weekend projects". 본인이 프로덕션 코드용이 아니라고 명시.

→ 우리 22주 프로젝트엔 vibe 만으론 부적합.

### 1.2 Spec-Driven Development (SDD) 합의

**핵심 전환**:
```
전통: 코드가 1순위, 스펙은 보조
SDD: 스펙이 1순위, 코드는 스펙에서 파생
```

수정 비용 비교:
- Vibe: 에러 발견 → AI 출력의 의도를 역공학해서 수정 (비싸고 느림)
- SDD: 에러 발견 → 스펙 업데이트 → 재생성 (거의 무료)

**3단계 워크플로우** (SDD 표준):
1. Requirements (요구사항) — 무엇을 만드나
2. Design (설계) — 어떻게 만드나
3. Tasks (작업 분해) — 어떤 단위로 만드나
4. Code generation (코드 생성) — 마지막 단계

### 1.3 외부 상태 원칙

큰 프로젝트의 핵심 통찰: **외부 상태는 파일·git에 살아야 함, LLM 컨텍스트 윈도우 안에 살면 안 됨**.

스펙·계획·진행 기록 모두 `.md` 파일과 git commit으로. 각 AI 작업은 fresh context에서 시작해서 필요한 것만 로드. **AI는 기억할 필요 없음 — 스펙을 읽음**.

이게 헤르메스의 학습 원리와 같음. **재귀적**: 학습하는 에이전트를 만드는 방법 자체가 학습 원리를 따름.

---

## 2. 2026 주요 SDD 프레임워크 비교

| 프레임워크 | 무게 | 적합 규모 | 특징 |
|---|---|---|---|
| **BMAD-METHOD** | 무거움 | 엔터프라이즈 | 12+ 전문 에이전트, 전체 SDLC |
| **GSD (Get Shit Done)** | 가벼움 | 솔로~소팀 | AI 에이전트 친화, 5개월 만에 61K stars |
| **GitHub Spec Kit** | 중간 | 다양 | 표준화, 다중 IDE 지원 |
| **Kiro (AWS)** | 무거움 | 엔터프라이즈 | 자체 IDE, AWS 통합 |
| **OpenSpec** | 매우 가벼움 | 프로토타입 | 순수 markdown |
| **Ralph Loop** | 가벼움 | 야간 자동화 | PRD 기반 무인 실행 |

### 2.1 우리 선택: GSD + BMAD 하이브리드

**왜 둘 다인가**:
- GSD: 일상 개발 워크플로우 (어느 AI 에이전트로든)
- BMAD: 큰 단계 전환 시 (Phase 시작 시 PM/Architect 에이전트로 설계)

BMAD는 4가지 에이전트 페르소나가 핵심:
- **Analyst** (Business Analyst) — 요구사항 명확화
- **PM** (Product Manager) — PRD 작성
- **Architect** — 기술 설계
- **Dev** — 구현

GSD는 단순:
- Discuss → Plan → Execute → Verify

→ **Phase 시작에 BMAD로 PRD/Architecture, 일상은 GSD로 Plan→Execute→Verify**.

---

## 3. 큰 프로젝트의 추가 원칙

### 3.1 컨텍스트 로테이션

큰 프로젝트는 한 세션에 다 안 들어감. 작업마다 fresh context:

```
Task 1: [fresh context] → spec 로드 → 구현 → commit → 종료
Task 2: [fresh context] → spec 로드 → 구현 → commit → 종료
...
```

→ 매 작업 후 컨텍스트 폐기. 다음 작업은 새로 시작.

### 3.2 작업 단위 (Story / Slice)

BMAD의 "Story" 개념. 한 작업 = 한 PR = 한 commit branch.

```
한 story의 단위:
- 1-3시간 안에 끝낼 수 있는 분량
- 명확한 acceptance criteria
- 독립적으로 테스트 가능
- 다른 story와 최소 결합
```

→ 우리 Phase별 작업을 다시 story 단위로 쪼개야.

### 3.3 검증 게이트 (Quality Gates)

vibe와 SDD의 차이는 **자동 게이트**:

```
Code generation
    ↓
Lint pass?  → No → 재생성
    ↓ Yes
Type check pass?  → No → 재생성
    ↓ Yes
Tests pass?  → No → 재생성
    ↓ Yes
Spec compliance check?  → No → 재생성
    ↓ Yes
Merge
```

각 게이트는 자동. AI가 자기 검증.

### 3.4 Living Specs

스펙은 한 번 쓰고 잊는 게 아님. 코드 변경 시 스펙도 같이 변경. git에서 코드와 함께 버전 관리:

```
git log specs/auth.md
- v3: OAuth 2.0 추가
- v2: 이메일 검증 추가
- v1: 초안
```

스펙과 코드가 차이 나면 CI 실패. **코드만 바꾸고 스펙 안 바꾸면 PR reject**.

---

## 4. 우리 프로젝트에 맞춘 vibe 강도

ExpertBeacon의 vibe 스펙트럼을 우리에 매핑:

| 단계 | 우리 적용 부분 |
|---|---|
| **Low vibe** (AI 초안, 사람 깊이 리뷰) | 보안, sandbox 격리, verifier, hooks |
| **Medium vibe** (AI 광범위 편집, 사람 아키텍처·테스트 리뷰) | 도구 구현, UI 컴포넌트, 일반 비즈니스 로직 |
| **High vibe** (사람은 결과만 검증) | 프로토타입, CSS 스타일링, 예제 |

→ Phase 0~2(코어)는 Low vibe. Phase 3~5(학습·UI)는 Medium vibe. Phase 6(문서·예제)는 High vibe.

---

## 5. 도구 스택 결정

| 용도 | 도구 |
|---|---|
| AI 코딩 에이전트 | **Claude Code**, **Codex CLI**, 또는 **Cursor** (선택) |
| SDD 프레임워크 | **BMAD** (Phase 설계) + **GSD** (일상 워크플로우) |
| 코드 호스팅 | **GitHub** (Spec Kit 호환) |
| Living specs | `.md` 파일 in `/specs` 디렉토리, git 버전 관리 |
| 작업 추적 | `tasks.md` per story + GitHub Issues |
| CI/CD | GitHub Actions + 자동 게이트 |
| 협업 (필요 시) | GitHub Discussions |

---

## 6. 우리 워크플로우 (확정)

```
Phase 시작 시 (예: Phase 1):
  1. BMAD Analyst로 요구사항 명확화 → /specs/phase-N/requirements.md
  2. BMAD Architect로 기술 설계 → /specs/phase-N/architecture.md
  3. BMAD PM으로 story 분해 → /specs/phase-N/stories/*.md
  4. 각 story는 acceptance criteria 포함

Story 작업 시 (Daily):
  1. fresh AI 에이전트 세션 시작 (Claude Code, Codex CLI 등)
  2. /specs 디렉토리 로드
  3. 작업할 story 명시: "Story #12 구현"
  4. Plan: AI가 구현 계획 작성
  5. 사용자 OK
  6. Execute: AI가 구현 + 테스트
  7. Verify: 자동 게이트 통과 확인
  8. Commit: 한 story = 한 commit
  9. 컨텍스트 폐기

Phase 종료 시:
  1. 모든 story 완료 확인
  2. Integration test
  3. Phase retrospective → /specs/phase-N/retrospective.md
  4. 다음 Phase 시작
```

---

## 7. 22주를 다시 story로 분해

이전에 Phase는 22주로 나눴음. SDD 관점에선 다시 story로 분해.

예: Phase 0 (3주) → 약 15-20개 story

```
Phase 0: 기반 인프라
├── Story 0.1: Bifrost Docker 배포 + 검증
├── Story 0.2: Rust 프로젝트 초기화 (Axum hello)
├── Story 0.3: SQLite 스키마 정의 + 마이그레이션
├── Story 0.4: Event Stream 데이터 모델
├── Story 0.5: Event Stream API (append/query)
├── Story 0.6: Tool catalog 데이터 모델
├── Story 0.7: AIO Sandbox Docker 통합
├── Story 0.8: Tool dispatcher 기본 골격
├── Story 0.9: WebSocket 서버 (Axum)
├── Story 0.10: 12-slot model router 골격
├── Story 0.11: Next.js 프로젝트 초기화
├── Story 0.12: 3-패널 레이아웃 (resizable)
├── Story 0.13: WebSocket 클라이언트
├── Story 0.14: Chat 컴포넌트 (notify 렌더)
├── Story 0.15: Phase 0 통합 테스트
```

각 story 1-3시간. 총 15개 × 2시간 = 30시간 = 1-2주 풀타임 또는 3주 짬짬이.

---

## 8. 결정 사항 요약

| 항목 | 결정 |
|---|---|
| 방법론 | Spec-Driven Development |
| 프레임워크 (Phase 설계) | BMAD-METHOD |
| 프레임워크 (일상) | GSD |
| 주 도구 | Claude Code 또는 Codex CLI (사용자 선택) |
| 보조 도구 | Cursor |
| 스펙 형식 | Markdown in `/specs` |
| 작업 단위 | Story (1-3시간) |
| 컨텍스트 정책 | 매 story 후 폐기 |
| 검증 | 자동 게이트 (lint, type, test, spec) |
| 비전 강도 | Phase별 다르게 (Low/Medium/High) |

---

## 9. 한 줄 결론

**큰 프로젝트의 vibe는 vibe가 아니다 — 스펙 우선, 코드 파생**. 우리 22주를 BMAD 페르소나로 phase 설계하고, GSD 워크플로우로 일상 구현하고, 모든 외부 상태를 `/specs` markdown으로 관리. AI가 기억할 필요 없게 — **읽으면 됨**.
