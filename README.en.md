# MouseRoaming — Cross-Platform Mouse, Keyboard & Clipboard Sharing

[繁體中文](README.md) | [简体中文](README.zh-CN.md) | [English](README.en.md)

MouseRoaming is a high-performance, low-latency, seamless cross-platform mouse, keyboard, and clipboard roaming utility tailored for **Windows 11** and **macOS**. Like Barrier / Synergy, but re-architected in modern **Rust** with hardened **TLS 1.3** transport encryption, interactive **drag-and-drop screen topology**, and native OS window icon visualization.

---

## ✨ Key Features

1. **Seamless Cross-Platform Roaming (Windows ↔ macOS)**
   - Move your mouse to the screen edge to glide directly onto the adjacent machine.
   - **Automatic Local Cursor Hiding**: When the mouse roams onto a remote machine, the local host cursor automatically hides completely. It instantly reappears the moment control returns to the local screen.
   - Configurable dead corner buffers and edge-switch cooldown to prevent bouncing.

2. **Intuitive Drag-and-Drop Screen Topology**
   - Distinctive Windows 11 rounded window icon with centered 4-pane logo and macOS rounded window with traffic-light buttons and Apple logo.
   - Hostname and local LAN IP address displayed prominently beneath each machine icon.
   - Drag device cards to Top / Bottom / Left / Right slots with magnetic snapping and dashed connector previews; dragging onto an already occupied slot automatically **swaps** their positions; dragging down into the dock unbinds the position.

3. **Smart Modifier & Wheel Remapping**
   - **Ctrl ↔ Command (⌘) Auto-Swap**: Pressing `Ctrl+C` / `Ctrl+V` on Windows maps naturally to `Cmd+C` / `Cmd+V` on macOS.
   - **Cross-OS Scroll Direction Compensation**: Neutralizes the difference between Windows wheel scrolling and macOS Natural Scrolling.
   - **Bidirectional Clipboard Sync**: Seamless real-time UTF-8 text synchronization (up to 2 MB per copy, equipped with loop/echo suppression).

4. **Security & Zero-Config Discovery**
   - **TLS 1.3 Encryption**: Automated self-signed certificate generation with TOFU (Trust-On-First-Use) SHA-256 fingerprint verification and optional pairing tokens.
   - **mDNS Local Discovery**: Automatically discovers active MouseRoaming servers on the same local subnet for one-click pairing.
   - **Hardware Emergency Escape**: Press `Ctrl + Alt + Esc` at any time to instantly break remote capture and snap the cursor back to the local screen.

---

## 🚀 Getting Started

### System Requirements
- **Windows**: Windows 10 / 11 (x64)
- **macOS**: macOS 12 Monterey or later (Apple Silicon M-series & Intel)
- Both machines must be connected to the same local area network (LAN / Wi-Fi).

### Build & Run

#### Option 1: Standard GUI
```bash
cargo build --release
# Run binary
./target/release/mouseroaming
```

#### Option 2: CLI / Daemon Mode
```bash
# Run as Server (share physical mouse & keyboard)
mouseroaming --role server --port 24850

# Run as Client (controlled by Server)
mouseroaming --role client --connect 192.168.1.100:24850

# Headless / Background service (no GUI)
mouseroaming --headless --role client --connect 192.168.1.100:24850
```

---

## 🖥 User Interface Guide

### 1. Screen Layout & Overview Tab
- **Center Node (This PC)**: Displays the local host's OS window icon, device name, and LAN IP address.
- **Surrounding Edge Slots**: Displays remote devices bound to Top, Bottom, Left, or Right.
- **Drag-and-Drop Operations**:
  - Drag any connected device from the canvas or bottom dock to place it around your PC.
  - Dropping onto an empty slot binds the position.
  - Dropping onto an occupied slot **swaps** both devices seamlessly.
  - Dropping down into the bottom dock unbinds the screen slot.
- **Quick Controls**:
  - **Double-click any device**: Instantly switches mouse/keyboard control to that machine.
  - **Right-click menu**: Quick switch, move to edge, or unbind.

### 2. LAN Discovery Tab
- Automatically lists servers broadcasting on the local network.
- Displays device name, OS kind, IP address, and TLS fingerprint; click **One-Click Connect** to pair immediately.

### 3. Settings Tab
- Toggle between Server and Client roles.
- Enable/disable Ctrl ↔ Cmd swapping, scroll direction inversion, and clipboard sync.
- Adjust dead-corner margin and edge-switch cooldown.

---

## 🔒 Permissions & Security

- **macOS**: On first run, grant permissions in **System Settings → Privacy & Security → Accessibility** and **Input Monitoring**.
- **Windows**: To interact with elevated Administrator windows (e.g., Task Manager), run MouseRoaming as Administrator (due to Windows UIPI security rules).

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
