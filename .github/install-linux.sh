#!/usr/bin/env bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev libudev-dev libvulkan1 mesa-vulkan-drivers xvfb
