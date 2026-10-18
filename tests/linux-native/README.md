# Native Linux package checks

These drivers start the actual packaged application with temporary HOME, XDG
paths, repository and D-Bus session. They use XTest or a Wayland virtual input
device and capture native PNGs. The startup stage reports only startup and real
shell input; it does not claim a complete UI round. An independent reviewer must
inspect the rendered images and full interaction evidence.

`Dockerfile` accepts a pinned distribution image through `BASE_IMAGE`. GitHub's
x86_64 runner executes the package. ARM preparation probes are recorded separately
and never pass an x86_64 product check. The package digest is checked in every
session, and the same candidate is used across distributions and both formats.

`native_session.py` owns and terminates only its disposable child processes.
`notification_server.py` is a test D-Bus provider. No real account or agent
conversation is used. Software Vulkan checks do not establish physical-GPU or
desktop-specific IME coverage.

`Linux packages` runs the full development journey on X11/Openbox and native
Wayland/Mutter. Mutter deliberately has no decoration protocol: the application
must draw and operate its actual client controls. Only the nested compositor's
input and capture use an X11 parent. The application receives no `DISPLAY` and
uses its Wayland socket. Labwc remains available for server-decoration checks.

The journey checks real terminal input, selection and clipboard readback,
window controls, title movement, every resize area, GTK folder selection,
notification activation across panes and windows, absent services and restart.
MCP status input is an explicit fixture through the packaged CLI, not a paid
provider conversation. Screenshots and state changes still require independent
inspection; automation never sets a full-round pass.

After the final main build succeeds, dispatch `Linux native review` with that
run id and its exact SHA. It downloads the existing artifacts for all three
rounds, validates their provenance and executes the pre-recorded
[`rounds.json`](rounds.json) in fresh sessions. Do not rebuild between rounds or
replace missing IME/visual evidence with a successful automation exit code.
