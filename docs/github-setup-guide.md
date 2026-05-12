# GitHub 셋업 가이드 — 즉시 시작용

> 압축 풀고 → GitHub 올리기까지 한 페이지. Day 1 작업.

---

## 0. 사전 점검 (5분)

### 본인 GitHub 핸들 확인
- github.com 로그인 후 본인 username 확인
- 본인 username: `slowdoctor-dev` (이미 setup-repo.sh와 모든 docs에 박혀있음)

### SSH 키 설정 (없다면)
```bash
# SSH 키 생성 (이미 있으면 skip)
ssh-keygen -t ed25519 -C "your-email@example.com"

# 공개 키 복사 후 GitHub에 등록
cat ~/.ssh/id_ed25519.pub
# → github.com → Settings → SSH and GPG keys → New SSH key

# 테스트
ssh -T git@github.com
# "Hi <username>! You've successfully authenticated"
```

### git 사용자 설정 (없다면)
```bash
git config --global user.name "Your Name"
git config --global user.email "your-email@example.com"
```

---

## 1. GitHub repo 생성 (3분)

### 1.1 브라우저에서 repo 생성

1. https://github.com/new 접속
2. **Repository name**: `seasoned-hand`
3. **Description**: 
   ```
   An open-source autonomous AI agent platform. Every task makes the hand wiser.
   ```
4. **Visibility**: ⚠ **Private 권장** (Phase 0 완료 후 Public 전환)
   - Public도 가능하지만 초기 commit이 영구 공개되는 점 유의
5. **Initialize this repository with**: 
   - ❌ Add a README file (이미 있음)
   - ❌ Add .gitignore (이미 있음)
   - ❌ Choose a license (이미 있음 - MIT)
6. **Create repository** 클릭

→ 빈 repo가 생성됨. 다음 화면에서 push 명령들이 표시되지만 무시.

---

## 2. 로컬 셋업 (5분)

### 2.1 압축 풀기

```bash
# 원하는 위치에 디렉토리 만들기
mkdir -p ~/projects && cd ~/projects

# 압축 파일 다운로드 후 풀기
tar xzf ~/Downloads/seasoned-hand-init.tar.gz
# → seasoned-hand-init/ 디렉토리 생성됨

# 폴더명을 프로젝트명으로 변경
mv seasoned-hand-init seasoned-hand
cd seasoned-hand
```

### 2.2 자동 셋업 스크립트 실행

압축 안에 `setup-repo.sh`가 포함되어 있습니다. 이게 자동으로:
- `git init` + `main` 브랜치
- 초기 commit 생성
- (Placeholder 치환은 이미 완료된 상태)
- GitHub remote 추가 (선택)
- push (선택)

```bash
bash setup-repo.sh
```

대화형으로 진행됩니다:
1. **GitHub username 입력** — 본인 핸들
2. **Initial commit 생성** — Y (기본값)
3. **Remote 추가 + push** — 위에서 GitHub repo 만들었으면 Y

---

## 3. 수동 셋업 (스크립트 안 쓸 경우)

스크립트가 어떤 작업하는지 보고 싶거나 직접 하고 싶으면:

```bash
cd ~/projects/seasoned-hand

# Placeholder는 이미 'slowdoctor-dev'로 치환되어 있음 — 추가 작업 불필요

# 1) git 초기화
git init
git branch -M main

# 3) setup-repo.sh 제거 (1회용 스크립트)
rm setup-repo.sh

# 4) 첫 commit
git add .
git commit -m "chore: initial scaffold

- BASELINE.md as single entry point
- AGENTS.md as LLM-agnostic source of truth
- 10 ADRs, 17 principles, 27 Phase 0 stories
- 6-phase roadmap (22 weeks)
- Manus direct Q&A external validation

Every task makes the hand wiser."

# 5) GitHub remote 추가 + push
git remote add origin git@github.com:slowdoctor-dev/seasoned-hand.git
git push -u origin main
```

---

## 4. 첫 push 후 확인 (3분)

### 4.1 GitHub 페이지 점검
- README가 잘 렌더링 되는가?
- LICENSE가 GitHub에 의해 "MIT License"로 자동 인식되는가?
- BASELINE.md, AGENTS.md, ARCHITECTURE.md 등 클릭해서 마크다운 렌더링 확인

### 4.2 placeholder 잔재 검사
```bash
# placeholder 패턴이 남아있는지 (이미 모두 치환됐어야 함)
grep -rn "<your-username>\|<your-handle>\|<owner>" . --include="*.md"
# → 빈 결과면 정상
```

### 4.3 민감 정보 final 검사
```bash
# .env가 실수로 들어갔는지
ls -la .env 2>/dev/null
# → "No such file or directory" 면 정상

# API 키 패턴 (실수 방지용)
grep -rE "sk-[a-zA-Z0-9]{20,}|AIzaSy[a-zA-Z0-9_-]+" . 2>/dev/null
# → 빈 결과면 정상
```

---

## 5. 외부 의존성 셋업 (선택, 추후 진행 가능)

이건 Phase 0 Story 0.1 시작 직전(Day 4)에 해도 됩니다:

### 5.1 도메인 (Day 3-4)
- `seasonedhand.dev` ($12/년, Cloudflare Registrar 권장)
- `seasonedhand.io` (선택, 보험)

### 5.2 API 키 (Day 4)
- Anthropic API key (https://console.anthropic.com/)
- OpenAI API key (https://platform.openai.com/api-keys)
- 최소 1개

### 5.3 npm scope (선택, Day 5+)
- `@seasoned-hand` org 생성

### 5.4 DockerHub (선택, Phase 6 가까이)
- `seasonedhand` namespace

---

## 6. 안전성 — 이미 점검 완료

압축 파일 안전성은 이미 검증됐습니다:

| 점검 | 결과 |
|---|---|
| API 키 노출 | ❌ 없음 |
| 개인정보 (이메일·전화·이름) | ❌ 없음 |
| LEAD 클리닉 정보 | ❌ 없음 |
| 큰 바이너리 파일 | ❌ 없음 |
| `.env` 실수 포함 | ❌ 없음 |
| Cargo.lock 정책 | ✅ 올바름 (커밋함 — 애플리케이션이라) |

**바로 push 해도 안전**합니다.

---

## 7. Public vs Private — 솔직한 권장

### Private 권장 시점: 지금 (Phase 0 시작)
- Phase 0 완료 전 (3주간) 실험·수정 자유로움
- Placeholder 누락이나 초기 commit 어색함이 영구 공개 안 됨
- 작동하는 demo 없이 Public은 마케팅 자살

### Public 전환 시점: Phase 0 완료 시
- `docker compose up` 으로 작동하는 demo 있음
- README가 정돈됨
- 첫 인상 통제 가능
- 이때 한 번에 launch (공개 발표, 커뮤니티 채널 등)

### Private → Public 전환 방법 (나중에)
```
GitHub repo → Settings → 맨 아래 "Danger Zone"
  → "Change repository visibility" → Public
```

한 번 클릭. 5초.

---

## 8. 첫 push 후 다음 행동

```
[지금] GitHub repo 만들고 push           ← 여기
   ↓
Day 2: BMAD Architect 페르소나
   → specs/phase-0/architecture.md 작성
   ↓
Day 3: BMAD PM 페르소나
   → 26개 story 작성 (story-0.2 ~ story-0.27)
   ↓
Day 4: Story 0.1 구현 시작 (Bifrost Docker)
   → 여기서 Cowork 이관 고려
   ↓
... Phase 0 진행 (3주)
   ↓
Phase 0 완료 → Public 전환 (선택)
```

자세한 7일 플랜: `docs/first-week-plan.md`

---

## 9. 막힐 때

### SSH 키 인증 실패
```bash
# 디버그
ssh -vT git@github.com

# SSH agent 시작
eval "$(ssh-agent -s)"
ssh-add ~/.ssh/id_ed25519
```

### git push 실패
```bash
# 원격 URL 확인
git remote -v

# HTTPS로 시도 (SSH 안 될 때)
git remote set-url origin https://github.com/<username>/seasoned-hand.git
git push -u origin main
# → username + Personal Access Token 요구
```

### Placeholder 치환 실패
```bash
# 수동 확인
grep -rn "<your-" . --include="*.md"

# 개별 파일 수정
$EDITOR BASELINE.md  # vim, nano, code 등
```

---

## 10. 한 줄 요약

```bash
# 압축 풀기 → 자동 셋업 → push
tar xzf seasoned-hand-init.tar.gz
cd seasoned-hand-init && mv ../seasoned-hand-init ../seasoned-hand
cd ../seasoned-hand
bash setup-repo.sh

# 또는 디렉토리명 그대로 두고
tar xzf seasoned-hand-init.tar.gz
cd seasoned-hand-init
bash setup-repo.sh
```

`Every task makes the hand wiser.` — Day 1 끝.
