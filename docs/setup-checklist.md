# Setup Checklist

> 프로젝트 시작 전 확보해야 할 항목들. 순서대로 진행 권장.

---

## 1. GitHub (5분)

- [ ] 개인 계정 아래 `seasoned-hand` repo 생성
  - **시작은 Private** (Phase 0 끝나면 Public 전환)
  - Description: "An open-source autonomous agent platform. Every task makes the hand wiser."
  - 라이선스: MIT (이 init 디렉토리에 LICENSE 포함됨)
  - .gitignore: 없음 선택 (이 init 디렉토리에 포함됨)
  - README: 없음 선택 (이 init 디렉토리에 포함됨)

```bash
# 로컬에서
cd ~/dev  # 또는 원하는 위치
mkdir seasoned-hand && cd seasoned-hand
tar xzf ~/Downloads/seasoned-hand-init.tar.gz
git init
git branch -M main
git add .
git commit -m "chore: initial scaffold

- README, LICENSE (MIT), AGENTS.md, CLAUDE.md
- specs/01-architecture/ARCHITECTURE.md (immutable v1.0)
- specs/phase-0/requirements.md + story-0.1.md
- docs/{methodology,getting-started,manifesto,brand,using-claude-and-codex}.md
- prompts/{bmad-analyst,bmad-architect,bmad-pm,gsd-execute-story}.md
- scripts/{spec-check,status}.sh
- .github/{ISSUE_TEMPLATE,PULL_REQUEST_TEMPLATE,workflows/ci.yml}
- justfile, docker-compose.yml, .env.example, .gitignore

Tagline: Every task makes the hand wiser."

git remote add origin git@github.com:slowdoctor-dev/seasoned-hand.git
git push -u origin main
```

## 2. 도메인 (10분)

순서대로:

- [ ] **seasonedhand.dev** ($12/년, Cloudflare Registrar 권장)
  - 가장 우선. 개발자 톤. HSTS preload 기본.
- [ ] **seasonedhand.io** ($35~50/년, 선택)
  - 보험. 잡아두면 좋음.
- [ ] **seasonedhand.com** (가격 변동 있음, 후순위)
  - 사업가 청중 대비. 가격이 합리적이면 잡음.

확인 도구:
- https://instantdomainsearch.com/
- https://www.namecheap.com/domains/
- https://www.cloudflare.com/products/registrar/

## 3. npm scope (5분)

- [ ] npm 계정 로그인
- [ ] `@seasoned-hand` org 생성 (무료)
  - https://www.npmjs.com/org/create
- [ ] 더미 패키지 publish (점유)
  - 향후 `@seasoned-hand/core`, `@seasoned-hand/sdk` 등

```bash
mkdir /tmp/seasoned-hand-placeholder && cd /tmp/seasoned-hand-placeholder
npm init -y
# package.json 수정: "name": "@seasoned-hand/placeholder"
npm publish --access public
```

## 4. crates.io (5분)

- [ ] crates.io GitHub 로그인
- [ ] 사용자명 확인 (`seasonedhand` 또는 `seasoned-hand` 권장)
- [ ] 더미 crate publish (점유, 선택)
  - Rust crate 이름 충돌 흔함. 미리 잡아두는 게 안전.

## 5. DockerHub (5분)

- [ ] DockerHub 계정
- [ ] `seasonedhand` org 또는 namespace 생성
- [ ] 향후 `seasonedhand/control-plane`, `seasonedhand/frontend` 이미지 push

## 6. 소셜 (선택, 10분)

- [ ] X(Twitter) `@seasonedhand` (가능하면)
- [ ] Mastodon `@seasonedhand@<instance>` (선택)
- [ ] Discord 서버 (Phase 6 가까이 가서)

## 7. 로컬 개발 환경 (30분)

- [ ] Docker Desktop 또는 호환 (Colima 등)
- [ ] Rust 1.78+ — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- [ ] Node 20+ + pnpm — `brew install node && npm install -g pnpm`
- [ ] just — `brew install just` 또는 `cargo install just`
- [ ] Claude Code — `curl -fsSL https://claude.ai/install.sh | bash` (native installer)
- [ ] (선택) Codex CLI — `npm install -g @openai/codex`
- [ ] (선택) Ollama (로컬 LLM) — `brew install ollama`

## 8. API 키 (5분)

최소 1개:

- [ ] Anthropic API key (https://console.anthropic.com/)
- [ ] OpenAI API key (https://platform.openai.com/api-keys)
- [ ] Google AI key (https://aistudio.google.com/app/apikey) — 무료 tier 있음
- [ ] OpenRouter key (https://openrouter.ai/keys) — 200+ 모델 한 키

`.env` 파일에 채우기:
```bash
cp .env.example .env
$EDITOR .env
```

## 9. 첫 verify (5분)

```bash
cd seasoned-hand
just status
# 출력: "No phases yet. Run 'just analyst-prompt' to start Phase 0."
# (실제로는 Phase 0 이미 있음. status.sh 스크립트가 출력 다른 것.)

just verify
# Phase -1이라 Rust/Frontend 게이트는 skip, spec-check만 실행
# 모든 체크 ✓ 떠야 함
```

## 10. Phase 0 시작 (즉시)

```bash
# fresh 세션
claude code

# 첫 프롬프트
just story-prompt
# Claude에게: "Implement story 0.1 (Bifrost Docker setup)."
```

또는 일단 BMAD 페르소나로 Phase 0 architecture 작성:

```bash
claude code
just architect-prompt
# Claude에게: "Phase 0 architecture를 작성해주세요. 
# requirements.md는 이미 있고, 27개 story가 식별되어 있습니다."
```

---

## 시간 예산

- 1번 ~ 8번: **약 1.5시간** (한 번에 처리 가능)
- 9번: 5분
- 10번: 1시간~ (Story 0.1 첫 구현)

**오늘 처리 가능**. 내일 Story 0.2 시작.

---

## 다음 마일스톤

- Week 1: Story 0.1 ~ 0.5 (Bifrost + Rust 기본 + Event Stream)
- Week 2: Story 0.6 ~ 0.15 (32 도구 + Agent Runner)
- Week 3: Story 0.16 ~ 0.27 (Frontend + 통합 테스트)
- **Phase 0 종료**: 3주 후
- **Phase 6 완료 (오픈소스 출시)**: 22주 후 (약 5개월)
