# Brand Guide

> Visual and verbal identity for Seasoned Hand.

---

## Name

**Seasoned Hand** — capitalized in prose, `seasoned-hand` (kebab-case) in code, repos, and packages.

Never: `SeasonedHand`, `Seasoned-Hand`, `SEASONED HAND`, "Seasoned" alone.

## Tagline

> Every task makes the hand wiser.

Use in:
- README first line
- Website hero
- Social bios

Do not modify, shorten, or translate without the project lead's review. A Korean equivalent is being considered: *"매 작업이 손을 더 영리하게."*

## Voice

| Do | Don't |
|---|---|
| Quiet confidence | Hype |
| Plain English | Marketing English |
| Specific claims | Vague superlatives |
| Acknowledge limitations | Pretend perfection |
| Korean 합쇼체 (~합니다) | 친근체 (~해요) in official docs |
| Short sentences | Walls of qualifications |

Banned words: *revolutionary*, *next-gen*, *AI-powered*, *smart*, *intelligent*, *cutting-edge*, *seamless*, *unleash*, *empower*, *game-changing*, *world-class*.

Reason: these words are how everyone else markets AI. We sound different by sounding plainer.

## Color palette

Materials, not gradients. Low saturation. Inspired by aged metal, linen, and stone.

| Name | Hex | Usage |
|---|---|---|
| **Brass** | `#B5895F` | Accent. Warm metal. |
| **Patina** | `#5C8273` | Secondary accent. Aged copper. |
| **Linen** | `#E8E2D5` | Background (light mode). |
| **Char** | `#1F1A17` | Text, dark mode background. |
| **Stone** | `#8A8074` | Muted text, borders. |

**Forbidden**:
- Gradients
- High-saturation purples, blues, magentas (typical AI SaaS palette)
- Glow, drop shadows, glassmorphism
- Neon

**Allowed**:
- Solid blocks
- Single-color line work
- Subtle paper texture

## Typography

Three-family pairing. Each serves a clear role.

| Role | Font | Why |
|---|---|---|
| Display (headlines) | **Spectral** (serif) | Time, weight, considered |
| Body | **Inter** (sans) | Modern legibility |
| Code | **JetBrains Mono** | Standard, ligature-friendly |

Alternatives if Spectral isn't available: **Cormorant Garamond**, **Source Serif**.

**Forbidden**:
- All-caps headlines (except short labels)
- Italic for emphasis (use weight instead)
- Three different sans-serif fonts on one page
- Display fonts in body text

## Logo

**Symbol**: An open hand silhouette, single color, no shading.

Hand pose: palm facing up, slightly cupped, fingers natural — neither welcoming nor demanding. The pose of someone who has worked with their hands.

**Wordmark**: `seasoned hand` (lowercase), letter-spacing +5%, single line, Spectral or matched serif.

**Lockups**:
- Horizontal: [symbol] · seasoned hand
- Stacked: [symbol] above wordmark
- Symbol only: for favicons, app icons

**Color variants**:
- Default: Char on Linen
- Inverted: Linen on Char
- Brass accent: only on hero treatments, sparingly

**Never**:
- Robot, brain, neural-net, or AI iconography
- Hand with sparkles, glow, or magical effects
- Hand holding tech objects (phone, gear, chip)
- Multiple hands, hands shaking, hands typing
- Anthropomorphic mascot characters

The hand stands alone. Its meaning is "someone who works."

## Mascot emoji

`🤚` (raised back of hand) — preferred for documentation, social posts, README badges.

Alternatives: `✋` (raised hand). Avoid `👋` (waving — too casual) and `👍` (thumbs up — wrong meaning).

## Photography & illustration style

If used at all:

- Natural light, indoor warmth
- Workspaces — workshops, libraries, study rooms
- Materials — wood, brass, paper, linen, ink
- Hands at work — not staged, not glamour shots
- Black & white acceptable; muted color preferred

Never:
- Stock photos of "diverse teams pointing at screens"
- Renders of glowing AI brains, circuit boards, neural networks
- Robots, androids, sci-fi imagery

## Iconography

Single-stroke line icons, 1.5px weight, rounded caps. Lucide Icons is a reasonable default library.

Color: inherit from text color or use Stone.

Never: filled glyphs, multi-color icons, animated icons.

## Web

**Layout**: generous whitespace, single column at narrow widths, optional sidebar at wide.

**Borders**: hairline (1px) Stone, never thick.

**Buttons**:
- Primary: Char fill, Linen text, sharp corners (radius 4px max)
- Secondary: Linen fill, Char border, Char text
- Never: pill buttons, gradients, large radii, raised shadows

**Animations**: easing-out, 150-200ms, never bouncing. Movement should feel like settling, not springing.

## Korean strings

- **분** is standard for the user-as-person (neutral, respectful).
- **합쇼체** (~합니다) is default tone.
- Numbers: 1, 2, 3 (Arabic), not 일, 이, 삼 (Sino-Korean) in technical contexts.
- Code identifiers: English. Comments: English.
- Documentation: bilingual welcomed. Primary specs in English for ecosystem reach.

## Examples of correct voice

**Good** (English):
> Seasoned Hand executes tasks autonomously and learns from verified outputs. Self-hosted, Apache-2.0-licensed.

**Bad**:
> Seasoned Hand is a revolutionary AI-powered agent platform that empowers users to unleash next-gen productivity through cutting-edge autonomous workflows.

**Good** (Korean):
> Seasoned Hand는 자율적으로 작업을 끝내고, 검증된 결과로부터 학습합니다. 자체 호스팅 가능, Apache-2.0 라이선스.

**Bad**:
> Seasoned Hand는 차세대 AI 기반 혁신적인 에이전트로 여러분의 생산성을 극대화합니다!

---

## When in doubt

Choose the plainer option. The boring one. The one that sounds like a competent engineer would say it in conversation, not the one a marketing site would say.

We're not trying to sound exciting. We're trying to sound trustworthy.
