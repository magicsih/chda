# 0.1.23 인수조건 — 설계 1.0

상태: 설계 1.0 승인 완료([기록](approval.md)). 아래는 실행 결과가 아닌 검증 계약이다. 기존 catalog의 13개 feature area와 기존 release case 16개는 모든 최종 native 회차·E2E에 계속 포함한다.

| ID · issue | 요구되는 regression 및 native scenario |
| --- | --- |
| ISSUE-208 · [#208](https://github.com/magicsih/chda/issues/208) | 색 변경 frame은 output=false. quiet pane들의 age·previous_activity가 theme/resize/scroll/search/selection에서 유지; 새 program output·terminal input은 갱신; live theme 적용 유지 |
| ISSUE-209 · [#209](https://github.com/magicsih/chda/issues/209) | main 우선·branch/path 이름순. pointer/menu/sheet 대상·선택·scroll/collapse가 hook/session/Git refresh에서 유지; 신규/제거 worktree만 목록 변경; Sessions 순서 유지 |
| ISSUE-210 · [#210](https://github.com/magicsih/chda/issues/210) | real Git repo의 UTF-8 filename이 stat·patch에 literal 표시; base·less -+F -R·stat/patch 순서 동일; 사용자 config 무변경; control character quoting 유지 |
| ISSUE-211 · [#211](https://github.com/magicsih/chda/issues/211) | visible noted-row tooltip을 띄운 뒤 모든 context menu 종류에서 tooltip 즉시 제거·재생성 억제; menu 종료 후 hover 정상. real native pixel 검증 |
| ISSUE-212 · [#212](https://github.com/magicsih/chda/issues/212) | bare space/parentheses/escaped space absolute·relative·~ 경로의 시작/중간/끝/soft-wrap row 모두 complete longest target. shorter existing prefix 패배; hard newline·adjacent path 분리; OSC/URL/quoted/#164 유지; 16 FS check 및 overflow 무링크; hover/click/menu target 동일 |
| ISSUE-216 · [#216](https://github.com/magicsih/chda/issues/216) | light/dark·normal/selected/waiting background에서 star/Git marks 3:1·diff text 4.5:1 unit ratios와 native visibility; PR badge 무변경 |
| ISSUE-217 · [#217](https://github.com/magicsih/chda/issues/217) | Sessions/PROJECT header에 pointer 고정 후 toggle 시 tooltip 사라짐; 다시 표시될 때 현재 action. sibling header controls tooltip 회귀 유지 |
| ISSUE-219 · [#219](https://github.com/magicsih/chda/issues/219) | Usage Escape/×, Star/Refresh/Copy/Reveal 후 같은 pane typing. navigation action은 새 pane focus. Cancel-default destructive sheet의 Tab/Shift-Tab/Enter/Space/Escape, launch permission arrows 및 exact args. confirm press/release가 terminal stdin에 전달되지 않음 |
| ISSUE-220 · [#220](https://github.com/magicsih/chda/issues/220) | badge click menu bounds가 badge 밑·window 안(default/640×600). tracked/notignored/missing/directory/symlink/traversal 메시지에 정확한 relative entry·이유, raw os-error 없음. 전체 검증 전 copy 없음; no-follow/race guards 유지 |
| ISSUE-224 · [#224](https://github.com/magicsih/chda/issues/224) | recycled row hard-line reflow; scroll-up 뒤 rows grow/column reflow, Debug integrity abort 없이 valid history pin 유지. no-pull/preserved cursor/allocator failure. native font/window resize·selection copy. Kitty temp image 허용 dir 및 boundary·optional data, all OS build. source/patch mismatch·재실행·미준비 build guard |
| ISSUE-226 · [#226](https://github.com/magicsih/chda/issues/226) | stale shell ~ + exact agent hook cwd=repo 저장/복원→repo. real CLI one-liner/two-step/managed/restore-agents off, repeated/mismatched sessions·legacy JSON·handoff. missing authoritative cwd는 잘못된 resume 없이 진단 |
| ISSUE-227 · [#227](https://github.com/magicsih/chda/issues/227) | idle/review parent + Working/Waiting/Unknown child에서 session ×·cmd-w·tab × 확인. Cancel은 pane/child 유지. click 직전 child 도착도 확인. all children complete/ended 즉시 close; 다른 conversation child는 영향 없음 |
| ISSUE-228 · [#228](https://github.com/magicsih/chda/issues/228) | 같은 tab split agents에 #1/#2 non-shrink bounds·tooltip pane 의미. 280/180px·18pt·640×600 native readability; single-agent 행 불변 |
| ISSUE-229 · [#229](https://github.com/magicsih/chda/issues/229) | synthetic real CLI rendering의 fenced code+nested list 부분 선택 원래 indent, 선택 밖 answer/fence 추가 없음. pipes/quotes/table/indent 보존. unknown layout/conversation, duplicate/incomplete lookup exact VT fallback. original preview/clipboard/scope/cold restore·ordinary Copy 유지; 붙여넣기 전송 없음 |
| ISSUE-230 · [#230](https://github.com/magicsih/chda/issues/230) | actual mouse dispatch: PROJECT/STARRED single select only, double open once, right-menu dismiss 무실행/Open terminal shell/agent default 준비·권한. cross-window pane reuse, missing worktree 무생성, disclosure/diff/unstar event 분리, Sessions single navigation 유지 |
| ISSUE-231 · [#231](https://github.com/magicsih/chda/issues/231) | managed/preset/typed/resumed/restored Claude·Codex /exit, top-level shell exit·same surviving shell. last pane/split/background/repeated exit에도 layout·output/code/signal 유지, dead input 거부·live indicators clear; explicit close만 cleanup; exact restart/restore/update 회귀 |

## Release coverage

0.1.23의 정상 evidence에는 위 16 ISSUE case와 기존 PATH-164/NAV-149/PR-150/TREE-151/REMOVE-152/QUOTA-153/CLOSE-154/NOTIFY-155/COPY-158/POLICY-159/RESTART-160/CHILD-161/REVIEW-162/PREP-163/PICKER-181/SESSIONS-182 전체가 필요하다. 과거 release evidence 계약은 그대로 보존한다.

세 native 회차는 계획·expected result·source/config·reviewer·evidence를 시작 전에 기록한다. 단위/화면 없는 scenario·Copilot code review를 native pass로 세지 않는다. round fail·not_run·blocked와 artifact fail/unknown은 있는 그대로 기록한다. private raw evidence에는 실행 자료를 보존하고 공개 metadata에는 허용된 식별자/hash/version/result만 넣는다.
