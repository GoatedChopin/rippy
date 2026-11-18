# A plain Ubuntu 22.04 desktop-less box with a virtual display, for trying the Linux build:
# Xvfb, Mesa's software Vulkan, and the libraries a desktop would already have.
FROM ubuntu:22.04
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates curl unzip python3 xvfb xauth x11-utils x11-apps imagemagick xdotool openbox \
      fonts-dejavu-core libfontconfig1 libfreetype6 libwayland-client0 libx11-xcb1 libxcb1 \
      libxkbcommon0 libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers libegl1 libgl1 \
 && rm -rf /var/lib/apt/lists/*
RUN useradd -m tester
USER tester
WORKDIR /home/tester
