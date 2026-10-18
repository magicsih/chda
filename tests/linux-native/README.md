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

The glibc 2.31 compatibility image retains Ubuntu 20.04 userspace and builds
Mesa 23.3.6 as a test-host Vulkan driver. Its official source checksum and
Python build dependencies are pinned. The driver is selected through
`VK_ICD_FILENAMES`, has its own ABI audit and manifest, and is never copied to
the application payload. This checks an old libc with a compatible driver; it
does not establish support for every driver shipped with that distribution.
Separate negative sessions retain actual application startup failures on stock
Debian 11 Mesa 20 and stock Ubuntu 20.04 Mesa 21. A successful negative check
always records product startup as failed.
