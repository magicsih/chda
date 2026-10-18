#!/bin/sh
set -eu
cd "$(dirname "$0")"
wayland-scanner client-header wlr-virtual-pointer-unstable-v1.xml wlr-virtual-pointer-unstable-v1-client-protocol.h
wayland-scanner private-code wlr-virtual-pointer-unstable-v1.xml wlr-virtual-pointer-unstable-v1-protocol.c
cc -std=c11 -Wall -Wextra -Werror $(pkg-config --cflags wayland-client) \
  wayland-drag.c wlr-virtual-pointer-unstable-v1-protocol.c \
  $(pkg-config --libs wayland-client) -o wayland-drag
