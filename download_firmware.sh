#!/bin/bash

# This script downloads the required CYW43439 firmware blobs for the Raspberry Pi Pico 2 W.
# The blobs are sourced directly from the official embassy-rs/embassy repository.

set -e

BASE_URL="https://raw.githubusercontent.com/embassy-rs/embassy/main/cyw43-firmware"

echo "Creating cyw43-firmware directory..."
mkdir -p cyw43-firmware

echo "Downloading CYW43439 firmware..."
curl -L -o cyw43-firmware/43439A0.bin "$BASE_URL/43439A0.bin"

echo "Downloading Country Lookup Matrix (CLM)..."
curl -L -o cyw43-firmware/43439A0_clm.bin "$BASE_URL/43439A0_clm.bin"

echo "Downloading NVRAM settings..."
curl -L -o cyw43-firmware/nvram_rp2040.bin "$BASE_URL/nvram_rp2040.bin"

echo "Downloading Bluetooth firmware (BTFW)..."
curl -L -o cyw43-firmware/43439A0_btfw.bin "$BASE_URL/43439A0_btfw.bin"

echo "Done! All firmware files downloaded to cyw43-firmware/ from official source."
