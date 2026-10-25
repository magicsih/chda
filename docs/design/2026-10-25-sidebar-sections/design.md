# Project sections and quieter session status

Approved scope: the owner's implementation request on 2026-10-25 incorporates
the supplied sidebar plan. Baseline: `8512a802c5c3eaffba1c2c1179c8aa58646ec3a3`.
No release, tag, version edit or installed-user-app replacement is included.

Projects contains Starred first, followed by ordered user sections and
Uncategorized. Without user sections it retains its flat list. Section headers
remain targets when empty or collapsed. Project headers drag within and across
sections; a project menu offers Move to section. Sections have create, rename,
delete, reorder and independent collapse actions. Deleting a section retains
all projects in Uncategorized. Adding a new folder normally uses Uncategorized;
a direct section drop uses that section, including the existing plain-folder
choice. Re-adding an existing project through the default path retains its
classification. The section-name input stays above the scrolling project list.

`repos` remains the registration and project-order authority. Ordered
`project-sections` entries contain `id`, `name` and `collapsed`; `repo-sections`
maps registered main paths to section ids. Renaming never changes ids. An absent
or deleted section means Uncategorized. `uncategorized-collapsed` is separate
from repository collapse. User navigation opens its target section and
repository; Projects itself stays collapsed if the user closed it. Background
refreshes retain disclosure and both scroll positions.

Sessions retains row and status-slot sizes. Working uses three dots with a
shared 1200ms GPUI animation, capped at 24 frames per second, with at least one
dot at the fitted status color on each frame. No workspace spinner timer is
used. `reduce-motion = true` and macOS Reduce Motion render a still frame; the
existing activity clock also reads accessibility preferences without changing
them. Ages occupy a fixed 34px slot, blank for Working, unknown time or less
than one minute since actual terminal input/output. Other ages are `1m`, `2m`,
`1h` and the existing compact day format.

Only Claude Code and Codex receive session pictures, derived from the provider
and full confirmed id. Until the id arrives, the same 15px slot has a neutral
icon and explanation, with no navigation or preview action. Below 200px both
pictures and neutral slots are hidden. Application control and child-state
icons are painted vectors. Terminal Nerd Font support is retained.

Acceptance and execution are recorded in `test-plan.json` and `verification.md`.
The feature catalog generates README and Pages. Native captures are required
because GPUI's test platform does not draw pixels.
