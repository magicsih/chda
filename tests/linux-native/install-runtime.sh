#!/bin/sh
set -eu
if command -v apt-get >/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    # Bullseye's security packages moved to Debian's archive after LTS ended.
    if [ -f /etc/apt/sources.list ] && . /etc/os-release && [ "$VERSION_CODENAME" = bullseye ]; then
        sed -i 's,deb.debian.org/debian-security,archive.debian.org/debian-security,g' /etc/apt/sources.list
    fi
    apt-get update
    apt-get install -y --no-install-recommends \
        ca-certificates bash git python3 python3-dbus python3-gi python3-pil \
        dbus-x11 mesa-vulkan-drivers vulkan-tools libegl1 libgles2 libgl1-mesa-dri \
        xvfb x11-utils xdotool openbox scrot xclip fonts-dejavu-core fonts-noto-cjk
    if [ "$1" = 1 ]; then
        apt-get install -y --no-install-recommends \
            labwc sway wtype wlrctl grim wl-clipboard wayland-utils \
            build-essential pkg-config libwayland-dev wlr-randr mutter xauth \
            tesseract-ocr strace xdg-utils libglib2.0-bin ibus ibus-hangul fcitx5 fcitx5-hangul \
            xdg-desktop-portal xdg-desktop-portal-gtk adwaita-icon-theme
    fi
elif command -v dnf >/dev/null; then
    dnf install -y --setopt=install_weak_deps=False \
        bash git python3 python3-dbus python3-gobject python3-pillow \
        dbus-daemon mesa-vulkan-drivers vulkan-tools \
        xorg-x11-server-Xvfb xdpyinfo xwininfo xdotool openbox scrot xclip \
        dejavu-sans-mono-fonts google-noto-sans-cjk-fonts
    dnf clean all
elif command -v pacman >/dev/null; then
    pacman -Syu --noconfirm --needed \
        bash git python python-dbus python-gobject python-pillow \
        dbus vulkan-swrast vulkan-tools \
        xorg-server-xvfb xorg-xdpyinfo xdotool openbox scrot xclip \
        ttf-dejavu noto-fonts-cjk
    pacman -Scc --noconfirm
else
    echo 'Unsupported validation distribution' >&2
    exit 1
fi
