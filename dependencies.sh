#!/bin/bash

# Script to automatically install Rust and required dependencies

# Check if script is run as root for package installation
if [ "$EUID" -ne 0 ]; then
    echo "This script requires root privileges to install packages. Please run with sudo."
    exit 1
fi

# Check if curl is installed
if ! command -v curl &> /dev/null; then
    echo "Installing curl..."
    apt-get update && apt-get install -y curl
    if [ $? -ne 0 ]; then
        echo "Error: Failed to install curl."
        exit 1
    fi
fi

# Install required dependencies
echo "Installing libssl-dev, pkgconf, pcscd, pcsc-tools, libccid, pkg-config, libgbm-dev, libxkbcommon-dev, libudev-dev, and libseat-dev..."
apt-get update && apt-get install -y libssl-dev pkgconf pcscd pcsc-tools libccid pkg-config libgbm-dev libxkbcommon-dev libudev-dev libseat-dev
if [ $? -ne 0 ]; then
    echo "Error: Failed to install one or more packages."
    exit 1
fi

# Download and run the official Rust installation script
echo "Downloading and installing Rust..."
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
if [ $? -ne 0 ]; then
    echo "Error: Rust installation failed."
    exit 1
fi

# Source the environment to make cargo available in the current session
source "$HOME/.cargo/env" 2>/dev/null || echo "Warning: Failed to source Rust environment."

# Verify installation of Rust
if command -v rustc &> /dev/null; then
    rustc --version
    echo "Rust and all dependencies (libssl-dev, pkgconf, pcscd, pcsc-tools, libccid, pkg-config, libgbm-dev, libxkbcommon-dev, libudev-dev, libseat-dev) installed successfully!"
else
    echo "Error: Rust compiler not found after installation."
    exit 1
fi